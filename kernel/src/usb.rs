//! xHCI USB host controller with bounded boot-keyboard and boot-mouse support.
//!
//! The driver owns one controller, four device slots and fixed DMA rings.  It
//! performs real xHCI slot/address/configure commands and USB control transfers;
//! normal interrupt-IN TRBs feed the existing Form-native input stream.  The
//! first revision polls the event ring, while the MSI layer can later wake the
//! same completion path without changing HID semantics.

use crate::{pci, sync::SpinMutex};
use core::sync::atomic::{compiler_fence, Ordering};

const MAX_DEVICES: usize = 4;
const MAX_SCRATCHPADS: usize = 8;
const RING_TRBS: usize = 256;
const RING_LINK_INDEX: usize = RING_TRBS - 1;
const EVENT_TRBS: usize = 256;
const MAX_POLLS: usize = 12_000_000;
const USBCMD_RUN: u32 = 1;
const USBCMD_RESET: u32 = 1 << 1;
const USBSTS_HALTED: u32 = 1;
const USBSTS_NOT_READY: u32 = 1 << 11;
const PORT_CONNECTED: u32 = 1;
const PORT_ENABLED: u32 = 1 << 1;
const PORT_RESET: u32 = 1 << 4;
const PORT_POWER: u32 = 1 << 9;
const PORT_CHANGE_BITS: u32 = 0x7F << 17;
const TRB_NORMAL: u32 = 1;
const TRB_SETUP_STAGE: u32 = 2;
const TRB_DATA_STAGE: u32 = 3;
const TRB_STATUS_STAGE: u32 = 4;
const TRB_LINK: u32 = 6;
const TRB_ENABLE_SLOT: u32 = 9;
const TRB_ADDRESS_DEVICE: u32 = 11;
const TRB_CONFIGURE_ENDPOINT: u32 = 12;
const EVENT_TRANSFER: u32 = 32;
const EVENT_COMMAND_COMPLETION: u32 = 33;
const COMPLETION_SUCCESS: u8 = 1;
const COMPLETION_SHORT_PACKET: u8 = 13;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HidEvent {
    Key(u8),
    Pointer {
        dx: i16,
        dy: i16,
        buttons: u8,
        pressed: u8,
        released: u8,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Status {
    pub controller: bool,
    pub keyboards: u8,
    pub mice: u8,
    pub keyboard_reports: u64,
    pub mouse_reports: u64,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Trb([u32; 4]);

impl Trb {
    const ZERO: Self = Self([0; 4]);

    const fn command(parameter: u64, kind: u32, extra: u32) -> Self {
        Self([
            parameter as u32,
            (parameter >> 32) as u32,
            0,
            (kind << 10) | extra,
        ])
    }

    const fn kind(self) -> u32 {
        (self.0[3] >> 10) & 0x3F
    }

    const fn completion_code(self) -> u8 {
        (self.0[2] >> 24) as u8
    }

    const fn slot(self) -> u8 {
        (self.0[3] >> 24) as u8
    }

    const fn endpoint(self) -> u8 {
        ((self.0[3] >> 16) & 0x1F) as u8
    }
}

#[repr(C, align(64))]
struct Ring([Trb; RING_TRBS]);

#[repr(C, align(64))]
struct Dcbaa([u64; 256]);

#[repr(C, align(64))]
struct Erst([u64; 2]);

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct Page([u8; 4096]);

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct ContextPage([u32; 1024]);

#[repr(C, align(64))]
struct Reports([[u8; 64]; MAX_DEVICES]);

static mut DCBAA: Dcbaa = Dcbaa([0; 256]);
static mut SCRATCHPAD_POINTERS: [u64; MAX_SCRATCHPADS] = [0; MAX_SCRATCHPADS];
static mut SCRATCHPADS: [Page; MAX_SCRATCHPADS] = [Page([0; 4096]); MAX_SCRATCHPADS];
static mut COMMAND_RING: Ring = Ring([Trb::ZERO; RING_TRBS]);
static mut EVENT_RING: Ring = Ring([Trb::ZERO; EVENT_TRBS]);
static mut ERST: Erst = Erst([0; 2]);
static mut INPUT_CONTEXTS: [ContextPage; MAX_DEVICES] = [ContextPage([0; 1024]); MAX_DEVICES];
static mut DEVICE_CONTEXTS: [ContextPage; MAX_DEVICES] = [ContextPage([0; 1024]); MAX_DEVICES];
static mut CONTROL_RINGS: [Ring; MAX_DEVICES] = [
    Ring([Trb::ZERO; RING_TRBS]),
    Ring([Trb::ZERO; RING_TRBS]),
    Ring([Trb::ZERO; RING_TRBS]),
    Ring([Trb::ZERO; RING_TRBS]),
];
static mut INTERRUPT_RINGS: [Ring; MAX_DEVICES] = [
    Ring([Trb::ZERO; RING_TRBS]),
    Ring([Trb::ZERO; RING_TRBS]),
    Ring([Trb::ZERO; RING_TRBS]),
    Ring([Trb::ZERO; RING_TRBS]),
];
static mut CONTROL_DATA: Page = Page([0; 4096]);
static mut REPORTS: Reports = Reports([[0; 64]; MAX_DEVICES]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Protocol {
    Keyboard,
    Mouse,
}

#[derive(Clone, Copy)]
struct Device {
    active: bool,
    slot: u8,
    port: u8,
    speed: u8,
    protocol: Protocol,
    interface: u8,
    endpoint_id: u8,
    max_packet: u16,
    interval: u8,
    control_tail: usize,
    control_cycle: bool,
    interrupt_tail: usize,
    interrupt_cycle: bool,
    previous_keys: [u8; 6],
    mouse_buttons: u8,
}

#[derive(Clone, Copy)]
struct ControlRequest {
    request_type: u8,
    request: u8,
    value: u16,
    target: u16,
}

impl Device {
    const EMPTY: Self = Self {
        active: false,
        slot: 0,
        port: 0,
        speed: 0,
        protocol: Protocol::Keyboard,
        interface: 0,
        endpoint_id: 0,
        max_packet: 0,
        interval: 0,
        control_tail: 0,
        control_cycle: true,
        interrupt_tail: 0,
        interrupt_cycle: true,
        previous_keys: [0; 6],
        mouse_buttons: 0,
    };
}

struct Controller {
    ready: bool,
    bar: usize,
    operational: usize,
    runtime: usize,
    doorbells: usize,
    context_size: usize,
    max_ports: u8,
    command_tail: usize,
    command_cycle: bool,
    event_head: usize,
    event_cycle: bool,
    keyboard_reports: u64,
    mouse_reports: u64,
    devices: [Device; MAX_DEVICES],
}

impl Controller {
    const fn new() -> Self {
        Self {
            ready: false,
            bar: 0,
            operational: 0,
            runtime: 0,
            doorbells: 0,
            context_size: 32,
            max_ports: 0,
            command_tail: 0,
            command_cycle: true,
            event_head: 0,
            event_cycle: true,
            keyboard_reports: 0,
            mouse_reports: 0,
            devices: [Device::EMPTY; MAX_DEVICES],
        }
    }

    fn initialize(&mut self) -> Result<Status, UsbError> {
        if self.ready {
            return Ok(self.status());
        }
        let function = pci::find_class(0x0C, 0x03, Some(0x30)).ok_or(UsbError::NoController)?;
        let bar = function.memory_bar(0).ok_or(UsbError::Unsupported)?;
        if bar.address < 0x10_0000
            || bar
                .address
                .checked_add(0x20_000)
                .is_none_or(|end| end > 0x1_0000_0000)
        {
            return Err(UsbError::Unsupported);
        }
        function.enable_memory_bus_master();
        self.bar = bar.address as usize;
        let cap_length = unsafe { mmio_read8(self.bar) } as usize;
        if !(0x20..=0x100).contains(&cap_length) {
            return Err(UsbError::Unsupported);
        }
        let hcs1 = unsafe { mmio_read32(self.bar + 0x04) };
        let hcs2 = unsafe { mmio_read32(self.bar + 0x08) };
        let hcc1 = unsafe { mmio_read32(self.bar + 0x10) };
        self.max_ports = (hcs1 >> 24) as u8;
        let max_slots = (hcs1 as u8).min(MAX_DEVICES as u8).max(1);
        let scratchpads = (((hcs2 >> 27) & 0x1F) | ((hcs2 >> 16) & 0x3E0)) as usize;
        if self.max_ports == 0 || scratchpads > MAX_SCRATCHPADS {
            return Err(UsbError::Unsupported);
        }
        self.context_size = if hcc1 & (1 << 2) != 0 { 64 } else { 32 };
        self.operational = self.bar + cap_length;
        self.doorbells = self.bar + (unsafe { mmio_read32(self.bar + 0x14) } as usize & !3);
        self.runtime = self.bar + (unsafe { mmio_read32(self.bar + 0x18) } as usize & !0x1F);

        unsafe {
            mmio_write32(
                self.operational,
                mmio_read32(self.operational) & !USBCMD_RUN,
            );
        }
        self.wait_status(USBSTS_HALTED, true)?;
        unsafe {
            mmio_write32(
                self.operational,
                mmio_read32(self.operational) | USBCMD_RESET,
            );
        }
        self.wait_command_clear(USBCMD_RESET)?;
        self.wait_status(USBSTS_NOT_READY, false)?;
        if unsafe { mmio_read32(self.operational + 0x08) } & 1 == 0 {
            return Err(UsbError::Unsupported);
        }
        unsafe { self.configure_memory(scratchpads, max_slots)? };
        unsafe {
            mmio_write32(self.operational, mmio_read32(self.operational) | USBCMD_RUN);
        }
        self.wait_status(USBSTS_HALTED, false)?;
        self.ready = true;
        let _ = crate::asl::claim(function, crate::asl::Owner::ExpUsb);
        if let Some(binding) = crate::interrupts::prepare(function) {
            crate::slog!(
                "EXPOS_INTERRUPT_PREPARED driver=xhci kind={:?} vector={:#x} armed=false\r\n",
                binding.kind,
                binding.vector
            );
        }

        let mut next = 0;
        for port in 1..=self.max_ports {
            if next == MAX_DEVICES {
                break;
            }
            if !self.reset_port(port)? {
                continue;
            }
            match self.enumerate_device(next, port) {
                Ok(()) => next += 1,
                Err(error) => {
                    crate::slog!(
                        "EXPOS_USB_DEVICE_REJECTED port={} error={:?}\r\n",
                        port,
                        error
                    )
                }
            }
        }
        Ok(self.status())
    }

    unsafe fn configure_memory(
        &mut self,
        scratchpads: usize,
        max_slots: u8,
    ) -> Result<(), UsbError> {
        unsafe {
            clear_bytes(core::ptr::addr_of_mut!(DCBAA.0).cast::<u8>(), 0, 2048);
            clear_bytes(
                core::ptr::addr_of_mut!(COMMAND_RING.0).cast::<u8>(),
                0,
                4096,
            );
            clear_bytes(core::ptr::addr_of_mut!(EVENT_RING.0).cast::<u8>(), 0, 4096);
            for index in 0..MAX_DEVICES {
                clear_bytes(
                    core::ptr::addr_of_mut!(INPUT_CONTEXTS[index].0).cast::<u8>(),
                    0,
                    4096,
                );
                clear_bytes(
                    core::ptr::addr_of_mut!(DEVICE_CONTEXTS[index].0).cast::<u8>(),
                    0,
                    4096,
                );
                clear_bytes(
                    core::ptr::addr_of_mut!(CONTROL_RINGS[index].0).cast::<u8>(),
                    0,
                    4096,
                );
                clear_bytes(
                    core::ptr::addr_of_mut!(INTERRUPT_RINGS[index].0).cast::<u8>(),
                    0,
                    4096,
                );
            }
            if scratchpads != 0 {
                for index in 0..scratchpads {
                    SCRATCHPAD_POINTERS[index] = dma(core::ptr::addr_of!(SCRATCHPADS[index].0))?;
                }
                DCBAA.0[0] = dma(core::ptr::addr_of!(SCRATCHPAD_POINTERS))?;
            }
            let dcbaa = dma(core::ptr::addr_of!(DCBAA.0))?;
            let command = dma(core::ptr::addr_of!(COMMAND_RING.0))?;
            let events = dma(core::ptr::addr_of!(EVENT_RING.0))?;
            ERST.0[0] = events;
            ERST.0[1] = EVENT_TRBS as u64;
            let erst = dma(core::ptr::addr_of!(ERST.0))?;
            mmio_write64(self.operational + 0x30, dcbaa);
            mmio_write64(self.operational + 0x18, command | 1);
            mmio_write32(self.operational + 0x38, max_slots as u32);
            let interrupter = self.runtime + 0x20;
            mmio_write32(interrupter + 0x08, 1);
            mmio_write64(interrupter + 0x10, erst);
            mmio_write64(interrupter + 0x18, events | (1 << 3));
            mmio_write32(interrupter, mmio_read32(interrupter) & !2);
        }
        self.command_tail = 0;
        self.command_cycle = true;
        self.event_head = 0;
        self.event_cycle = true;
        Ok(())
    }

    fn reset_port(&mut self, port: u8) -> Result<bool, UsbError> {
        let register = self.port_register(port);
        let status = unsafe { mmio_read32(register) };
        if status & PORT_CONNECTED == 0 {
            return Ok(false);
        }
        unsafe {
            let preserved = status & !PORT_CHANGE_BITS;
            mmio_write32(register, preserved | PORT_POWER | PORT_RESET);
        }
        for _ in 0..MAX_POLLS {
            let value = unsafe { mmio_read32(register) };
            if value & PORT_RESET == 0 && value & PORT_ENABLED != 0 {
                unsafe {
                    mmio_write32(
                        register,
                        (value & !PORT_CHANGE_BITS) | (value & PORT_CHANGE_BITS),
                    )
                };
                return Ok(true);
            }
            core::hint::spin_loop();
        }
        Err(UsbError::Timeout)
    }

    fn enumerate_device(&mut self, index: usize, port: u8) -> Result<(), UsbError> {
        let port_status = unsafe { mmio_read32(self.port_register(port)) };
        let speed = ((port_status >> 10) & 0xF) as u8;
        let slot = self.command(Trb::command(0, TRB_ENABLE_SLOT, 0))?.slot();
        if slot == 0 {
            return Err(UsbError::Protocol);
        }
        let device_context = unsafe { dma(core::ptr::addr_of!(DEVICE_CONTEXTS[index].0))? };
        unsafe { DCBAA.0[slot as usize] = device_context };
        let mut device = Device {
            active: false,
            slot,
            port,
            speed,
            ..Device::EMPTY
        };
        unsafe { self.prepare_address_context(index, device)? };
        let input = unsafe { dma(core::ptr::addr_of!(INPUT_CONTEXTS[index].0))? };
        self.command(Trb::command(input, TRB_ADDRESS_DEVICE, (slot as u32) << 24))?;

        let configuration_descriptor = ControlRequest {
            request_type: 0x80,
            request: 6,
            value: 0x0200,
            target: 0,
        };
        let descriptor_len = self.control_in(index, &mut device, configuration_descriptor, 9)?;
        if descriptor_len < 9 {
            return Err(UsbError::Protocol);
        }
        let total = unsafe {
            let bytes = core::ptr::addr_of!(CONTROL_DATA.0).cast::<u8>();
            u16::from_le_bytes([core::ptr::read(bytes.add(2)), core::ptr::read(bytes.add(3))])
        } as usize;
        if !(9..=4096).contains(&total) {
            return Err(UsbError::Protocol);
        }
        let length = self.control_in(index, &mut device, configuration_descriptor, total as u16)?;
        let descriptor = unsafe {
            core::slice::from_raw_parts(core::ptr::addr_of!(CONTROL_DATA.0).cast::<u8>(), length)
        };
        let parsed = parse_hid_configuration(descriptor).ok_or(UsbError::Unsupported)?;
        device.protocol = parsed.protocol;
        device.interface = parsed.interface;
        device.endpoint_id = endpoint_id(parsed.endpoint_address);
        device.max_packet = parsed.max_packet.min(64);
        device.interval = parsed.interval;
        if device.endpoint_id < 2 || device.max_packet == 0 {
            return Err(UsbError::Protocol);
        }
        unsafe { self.prepare_endpoint_context(index, device)? };
        self.command(Trb::command(
            input,
            TRB_CONFIGURE_ENDPOINT,
            (slot as u32) << 24,
        ))?;
        self.control_out(index, &mut device, 0x00, 9, parsed.configuration as u16, 0)?;
        self.control_out(index, &mut device, 0x21, 11, 0, parsed.interface as u16)?;
        device.active = true;
        self.devices[index] = device;
        self.submit_interrupt(index)?;
        crate::slog!(
            "EXPOS_USB_HID_DEVICE port={} slot={} protocol={} endpoint={} packet={}\r\n",
            port,
            slot,
            match device.protocol {
                Protocol::Keyboard => "keyboard",
                Protocol::Mouse => "mouse",
            },
            device.endpoint_id,
            device.max_packet
        );
        Ok(())
    }

    unsafe fn prepare_address_context(
        &mut self,
        index: usize,
        device: Device,
    ) -> Result<(), UsbError> {
        unsafe {
            clear_bytes(
                core::ptr::addr_of_mut!(INPUT_CONTEXTS[index].0).cast::<u8>(),
                0,
                4096,
            );
            let input = core::ptr::addr_of_mut!(INPUT_CONTEXTS[index].0).cast::<u8>();
            write_context_dword(input, 0, 1, 0x3);
            let slot = self.context(input, 1);
            write_dword(slot, 0, (device.speed as u32) << 20 | (1 << 27));
            write_dword(slot, 1, (device.port as u32) << 16);
            let endpoint = self.context(input, 2);
            let packet = match device.speed {
                1 | 2 => 8,
                3 => 64,
                4 | 5 => 512,
                _ => return Err(UsbError::Unsupported),
            };
            write_dword(endpoint, 1, (3 << 1) | (4 << 3) | (packet << 16));
            let ring = dma(core::ptr::addr_of!(CONTROL_RINGS[index].0))?;
            write_dword(endpoint, 2, ring as u32 | 1);
            write_dword(endpoint, 3, (ring >> 32) as u32);
            write_dword(endpoint, 4, 8);
            compiler_fence(Ordering::Release);
        }
        Ok(())
    }

    unsafe fn prepare_endpoint_context(
        &mut self,
        index: usize,
        device: Device,
    ) -> Result<(), UsbError> {
        unsafe {
            clear_bytes(
                core::ptr::addr_of_mut!(INPUT_CONTEXTS[index].0).cast::<u8>(),
                0,
                4096,
            );
            let input = core::ptr::addr_of_mut!(INPUT_CONTEXTS[index].0).cast::<u8>();
            write_context_dword(input, 0, 1, 1 | (1_u32 << device.endpoint_id));
            let slot = self.context(input, 1);
            write_dword(
                slot,
                0,
                (device.speed as u32) << 20 | ((device.endpoint_id as u32) << 27),
            );
            write_dword(slot, 1, (device.port as u32) << 16);
            let endpoint = self.context(input, device.endpoint_id as usize + 1);
            let interval = interval_encoding(device.speed, device.interval) as u32;
            write_dword(endpoint, 0, interval << 16);
            write_dword(
                endpoint,
                1,
                (3 << 1) | (7 << 3) | ((device.max_packet as u32) << 16),
            );
            let ring = dma(core::ptr::addr_of!(INTERRUPT_RINGS[index].0))?;
            write_dword(endpoint, 2, ring as u32 | 1);
            write_dword(endpoint, 3, (ring >> 32) as u32);
            write_dword(
                endpoint,
                4,
                device.max_packet as u32 | ((device.max_packet as u32) << 16),
            );
            compiler_fence(Ordering::Release);
        }
        Ok(())
    }

    fn control_in(
        &mut self,
        index: usize,
        device: &mut Device,
        request: ControlRequest,
        length: u16,
    ) -> Result<usize, UsbError> {
        unsafe {
            clear_bytes(
                core::ptr::addr_of_mut!(CONTROL_DATA.0).cast::<u8>(),
                0,
                4096,
            )
        };
        let setup = setup_packet(
            request.request_type,
            request.request,
            request.value,
            request.target,
            length,
        );
        self.push_control(
            index,
            device,
            Trb([
                setup as u32,
                (setup >> 32) as u32,
                8,
                (TRB_SETUP_STAGE << 10) | (3 << 16) | (1 << 6) | (1 << 4),
            ]),
        )?;
        let data = unsafe { dma(core::ptr::addr_of!(CONTROL_DATA.0))? };
        self.push_control(
            index,
            device,
            Trb([
                data as u32,
                (data >> 32) as u32,
                length as u32,
                (TRB_DATA_STAGE << 10) | (1 << 16) | (1 << 4),
            ]),
        )?;
        self.push_control(
            index,
            device,
            Trb([0, 0, 0, (TRB_STATUS_STAGE << 10) | (1 << 5)]),
        )?;
        unsafe { mmio_write32(self.doorbells + device.slot as usize * 4, 1) };
        let event = self.wait_transfer(device.slot, 1)?;
        let residual = (event.0[2] & 0x00FF_FFFF) as usize;
        Ok(length as usize - residual.min(length as usize))
    }

    fn control_out(
        &mut self,
        index: usize,
        device: &mut Device,
        request_type: u8,
        request: u8,
        value: u16,
        target: u16,
    ) -> Result<(), UsbError> {
        let setup = setup_packet(request_type, request, value, target, 0);
        self.push_control(
            index,
            device,
            Trb([
                setup as u32,
                (setup >> 32) as u32,
                8,
                (TRB_SETUP_STAGE << 10) | (1 << 6) | (1 << 4),
            ]),
        )?;
        self.push_control(
            index,
            device,
            Trb([0, 0, 0, (TRB_STATUS_STAGE << 10) | (1 << 16) | (1 << 5)]),
        )?;
        unsafe { mmio_write32(self.doorbells + device.slot as usize * 4, 1) };
        self.wait_transfer(device.slot, 1)?;
        Ok(())
    }

    fn push_control(&self, index: usize, device: &mut Device, trb: Trb) -> Result<(), UsbError> {
        unsafe {
            push_ring(
                core::ptr::addr_of_mut!(CONTROL_RINGS[index]),
                &mut device.control_tail,
                &mut device.control_cycle,
                trb,
            )
        }
    }

    fn submit_interrupt(&mut self, index: usize) -> Result<(), UsbError> {
        let device = &mut self.devices[index];
        if !device.active {
            return Err(UsbError::Protocol);
        }
        unsafe {
            clear_bytes(
                core::ptr::addr_of_mut!(REPORTS.0[index]).cast::<u8>(),
                0,
                64,
            )
        };
        let report = unsafe { dma(core::ptr::addr_of!(REPORTS.0[index]))? };
        let trb = Trb([
            report as u32,
            (report >> 32) as u32,
            device.max_packet as u32,
            (TRB_NORMAL << 10) | (1 << 5),
        ]);
        unsafe {
            push_ring(
                core::ptr::addr_of_mut!(INTERRUPT_RINGS[index]),
                &mut device.interrupt_tail,
                &mut device.interrupt_cycle,
                trb,
            )?;
            compiler_fence(Ordering::Release);
            mmio_write32(
                self.doorbells + device.slot as usize * 4,
                device.endpoint_id as u32,
            );
        }
        Ok(())
    }

    fn command(&mut self, command: Trb) -> Result<Trb, UsbError> {
        unsafe {
            push_ring(
                core::ptr::addr_of_mut!(COMMAND_RING),
                &mut self.command_tail,
                &mut self.command_cycle,
                command,
            )?;
            compiler_fence(Ordering::Release);
            mmio_write32(self.doorbells, 0);
        }
        for _ in 0..MAX_POLLS {
            if let Some(event) = self.next_event() {
                if event.kind() != EVENT_COMMAND_COMPLETION {
                    continue;
                }
                if event.completion_code() != COMPLETION_SUCCESS {
                    return Err(UsbError::Completion(event.completion_code()));
                }
                return Ok(event);
            }
            core::hint::spin_loop();
        }
        Err(UsbError::Timeout)
    }

    fn wait_transfer(&mut self, slot: u8, endpoint: u8) -> Result<Trb, UsbError> {
        for _ in 0..MAX_POLLS {
            if let Some(event) = self.next_event() {
                if event.kind() != EVENT_TRANSFER
                    || event.slot() != slot
                    || event.endpoint() != endpoint
                {
                    continue;
                }
                if !matches!(
                    event.completion_code(),
                    COMPLETION_SUCCESS | COMPLETION_SHORT_PACKET
                ) {
                    return Err(UsbError::Completion(event.completion_code()));
                }
                return Ok(event);
            }
            core::hint::spin_loop();
        }
        Err(UsbError::Timeout)
    }

    fn next_event(&mut self) -> Option<Trb> {
        let pointer = unsafe {
            core::ptr::addr_of!(EVENT_RING.0)
                .cast::<Trb>()
                .add(self.event_head)
        };
        let control = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*pointer).0[3])) };
        if (control & 1 != 0) != self.event_cycle {
            return None;
        }
        let mut event = Trb::ZERO;
        for index in 0..4 {
            event.0[index] =
                unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*pointer).0[index])) };
        }
        self.event_head += 1;
        if self.event_head == EVENT_TRBS {
            self.event_head = 0;
            self.event_cycle = !self.event_cycle;
        }
        let next = unsafe {
            dma(core::ptr::addr_of!(EVENT_RING.0)
                .cast::<Trb>()
                .add(self.event_head))
            .ok()?
        };
        unsafe { mmio_write64(self.runtime + 0x38, next | (1 << 3)) };
        Some(event)
    }

    fn poll(&mut self) -> Option<HidEvent> {
        if !self.ready {
            return None;
        }
        for _ in 0..8 {
            let event = self.next_event()?;
            if event.kind() != EVENT_TRANSFER
                || !matches!(
                    event.completion_code(),
                    COMPLETION_SUCCESS | COMPLETION_SHORT_PACKET
                )
            {
                continue;
            }
            let Some(index) = self.devices.iter().position(|device| {
                device.active
                    && device.slot == event.slot()
                    && device.endpoint_id == event.endpoint()
            }) else {
                continue;
            };
            let requested = self.devices[index].max_packet as usize;
            let residual = (event.0[2] & 0x00FF_FFFF) as usize;
            let length = requested.saturating_sub(residual.min(requested));
            compiler_fence(Ordering::Acquire);
            let report = unsafe {
                core::slice::from_raw_parts(
                    core::ptr::addr_of!(REPORTS.0[index]).cast::<u8>(),
                    length,
                )
            };
            match self.devices[index].protocol {
                Protocol::Keyboard => self.keyboard_reports += 1,
                Protocol::Mouse => self.mouse_reports += 1,
            }
            let decoded = decode_report(&mut self.devices[index], report);
            let _ = self.submit_interrupt(index);
            if decoded.is_some() {
                return decoded;
            }
        }
        None
    }

    fn status(&self) -> Status {
        let mut keyboards = 0;
        let mut mice = 0;
        for device in self.devices.iter().filter(|device| device.active) {
            match device.protocol {
                Protocol::Keyboard => keyboards += 1,
                Protocol::Mouse => mice += 1,
            }
        }
        Status {
            controller: self.ready,
            keyboards,
            mice,
            keyboard_reports: self.keyboard_reports,
            mouse_reports: self.mouse_reports,
        }
    }

    fn wait_status(&self, mask: u32, set: bool) -> Result<(), UsbError> {
        for _ in 0..MAX_POLLS {
            if (unsafe { mmio_read32(self.operational + 0x04) } & mask != 0) == set {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(UsbError::Timeout)
    }

    fn wait_command_clear(&self, mask: u32) -> Result<(), UsbError> {
        for _ in 0..MAX_POLLS {
            if unsafe { mmio_read32(self.operational) } & mask == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(UsbError::Timeout)
    }

    const fn port_register(&self, port: u8) -> usize {
        self.operational + 0x400 + (port as usize - 1) * 0x10
    }

    unsafe fn context(&self, input: *mut u8, index: usize) -> *mut u8 {
        unsafe { input.add(index * self.context_size) }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UsbError {
    NoController,
    Unsupported,
    Timeout,
    Protocol,
    Completion(u8),
}

static CONTROLLER: SpinMutex<Controller> = SpinMutex::new(Controller::new());

pub fn initialize() -> Status {
    let mut controller = CONTROLLER.lock();
    match controller.initialize() {
        Ok(status) => {
            crate::slog!(
                "EXPOS_USB_READY controller=xhci keyboard={} mouse={} polling=true\r\n",
                status.keyboards,
                status.mice
            );
            status
        }
        Err(UsbError::NoController) => Status {
            controller: false,
            keyboards: 0,
            mice: 0,
            keyboard_reports: 0,
            mouse_reports: 0,
        },
        Err(error) => {
            crate::slog!("EXPOS_USB_FAILED controller=xhci error={:?}\r\n", error);
            Status {
                controller: false,
                keyboards: 0,
                mice: 0,
                keyboard_reports: 0,
                mouse_reports: 0,
            }
        }
    }
}

pub fn poll_event() -> Option<HidEvent> {
    CONTROLLER.lock().poll()
}

pub fn status() -> Status {
    CONTROLLER.lock().status()
}

#[derive(Clone, Copy)]
struct ParsedHid {
    configuration: u8,
    interface: u8,
    protocol: Protocol,
    endpoint_address: u8,
    max_packet: u16,
    interval: u8,
}

fn parse_hid_configuration(bytes: &[u8]) -> Option<ParsedHid> {
    if bytes.len() < 9 || bytes[1] != 2 {
        return None;
    }
    let configuration = bytes[5];
    let mut offset = 9;
    let mut interface = None;
    while offset + 2 <= bytes.len() {
        let length = bytes[offset] as usize;
        let kind = bytes[offset + 1];
        if length < 2 || offset + length > bytes.len() {
            return None;
        }
        if kind == 4 && length >= 9 {
            interface = if bytes[offset + 5] == 3 && bytes[offset + 6] == 1 {
                match bytes[offset + 7] {
                    1 => Some((bytes[offset + 2], Protocol::Keyboard)),
                    2 => Some((bytes[offset + 2], Protocol::Mouse)),
                    _ => None,
                }
            } else {
                None
            };
        } else if kind == 5 && length >= 7 {
            if let Some((number, protocol)) = interface {
                let address = bytes[offset + 2];
                if address & 0x80 != 0 && bytes[offset + 3] & 3 == 3 {
                    return Some(ParsedHid {
                        configuration,
                        interface: number,
                        protocol,
                        endpoint_address: address,
                        max_packet: u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]),
                        interval: bytes[offset + 6],
                    });
                }
            }
        }
        offset += length;
    }
    None
}

fn decode_report(device: &mut Device, report: &[u8]) -> Option<HidEvent> {
    match device.protocol {
        Protocol::Keyboard if report.len() >= 8 => {
            let modifiers = report[0];
            let mut pressed = None;
            for usage in &report[2..8] {
                if *usage != 0 && !device.previous_keys.contains(usage) && pressed.is_none() {
                    pressed = hid_key(*usage, modifiers);
                }
            }
            device.previous_keys.copy_from_slice(&report[2..8]);
            pressed.map(HidEvent::Key)
        }
        Protocol::Mouse if report.len() >= 3 => {
            let buttons = report[0] & 7;
            let previous = device.mouse_buttons;
            device.mouse_buttons = buttons;
            Some(HidEvent::Pointer {
                dx: report[1] as i8 as i16,
                dy: report[2] as i8 as i16,
                buttons,
                pressed: buttons & !previous,
                released: previous & !buttons,
            })
        }
        _ => None,
    }
}

fn hid_key(usage: u8, modifiers: u8) -> Option<u8> {
    let shift = modifiers & 0x22 != 0;
    let super_key = modifiers & 0x88 != 0;
    let base = match usage {
        0x04..=0x1D => b'a' + usage - 0x04,
        0x1E..=0x26 => b'1' + usage - 0x1E,
        0x27 => b'0',
        0x28 => b'\n',
        0x29 => 0x1B,
        0x2A => 0x08,
        0x2B => b'\t',
        0x2C => b' ',
        0x2D => b'-',
        0x2E => b'=',
        0x2F => b'[',
        0x30 => b']',
        0x31 => b'\\',
        0x33 => b';',
        0x34 => b'\'',
        0x35 => b'`',
        0x36 => b',',
        0x37 => b'.',
        0x38 => b'/',
        0x4F => {
            return Some(if super_key {
                crate::input::KEY_SUPER_RIGHT
            } else {
                crate::input::KEY_RIGHT
            })
        }
        0x50 => {
            return Some(if super_key {
                crate::input::KEY_SUPER_LEFT
            } else {
                crate::input::KEY_LEFT
            })
        }
        0x51 => {
            return Some(if super_key {
                crate::input::KEY_SUPER_DOWN
            } else {
                crate::input::KEY_DOWN
            })
        }
        0x52 => {
            return Some(if super_key {
                crate::input::KEY_SUPER_UP
            } else {
                crate::input::KEY_UP
            })
        }
        _ => return None,
    };
    let key = crate::input::apply_modifiers(base, shift, false);
    if super_key {
        crate::input::super_binding(key)
    } else {
        Some(key)
    }
}

fn endpoint_id(address: u8) -> u8 {
    let number = address & 0x0F;
    number * 2 + u8::from(address & 0x80 != 0)
}

fn interval_encoding(speed: u8, interval: u8) -> u8 {
    if speed >= 3 {
        interval.saturating_sub(1).min(15)
    } else {
        let mut value = interval.max(1) - 1;
        let mut exponent = 0;
        while value != 0 {
            value >>= 1;
            exponent += 1;
        }
        (exponent + 2).min(15)
    }
}

const fn setup_packet(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> u64 {
    request_type as u64
        | ((request as u64) << 8)
        | ((value as u64) << 16)
        | ((index as u64) << 32)
        | ((length as u64) << 48)
}

unsafe fn push_ring(
    ring: *mut Ring,
    tail: &mut usize,
    cycle: &mut bool,
    mut trb: Trb,
) -> Result<(), UsbError> {
    if *tail == RING_LINK_INDEX {
        let base = unsafe { dma(core::ptr::addr_of!((*ring).0))? };
        let link = Trb([
            base as u32,
            (base >> 32) as u32,
            0,
            (TRB_LINK << 10) | (1 << 1) | u32::from(*cycle),
        ]);
        unsafe { write_trb(core::ptr::addr_of_mut!((*ring).0[RING_LINK_INDEX]), link) };
        *tail = 0;
        *cycle = !*cycle;
    }
    trb.0[3] = (trb.0[3] & !1) | u32::from(*cycle);
    unsafe { write_trb(core::ptr::addr_of_mut!((*ring).0[*tail]), trb) };
    *tail += 1;
    Ok(())
}

unsafe fn write_trb(destination: *mut Trb, trb: Trb) {
    for index in 0..4 {
        unsafe {
            core::ptr::write_volatile(
                core::ptr::addr_of_mut!((*destination).0[index]),
                trb.0[index],
            )
        };
    }
}

unsafe fn write_context_dword(base: *mut u8, context: usize, dword: usize, value: u32) {
    unsafe { write_dword(base.add(context * 32), dword, value) }
}

unsafe fn write_dword(base: *mut u8, dword: usize, value: u32) {
    unsafe { core::ptr::write_volatile(base.cast::<u32>().add(dword), value) }
}

unsafe fn clear_bytes(pointer: *mut u8, value: u8, bytes: usize) {
    unsafe { core::ptr::write_bytes(pointer, value, bytes) }
}

unsafe fn dma<T>(pointer: *const T) -> Result<u64, UsbError> {
    let address = pointer as u64;
    if address == 0 || address >= 0x1_0000_0000 {
        Err(UsbError::Unsupported)
    } else {
        Ok(address)
    }
}

unsafe fn mmio_read8(address: usize) -> u8 {
    unsafe { core::ptr::read_volatile(address as *const u8) }
}

unsafe fn mmio_read32(address: usize) -> u32 {
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

unsafe fn mmio_write32(address: usize, value: u32) {
    unsafe { core::ptr::write_volatile(address as *mut u32, value) }
}

unsafe fn mmio_write64(address: usize, value: u64) {
    unsafe { core::ptr::write_volatile(address as *mut u64, value) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_boot_keyboard_and_mouse_interfaces() {
        let keyboard = [
            9, 2, 25, 0, 1, 1, 0, 0x80, 50, 9, 4, 2, 0, 1, 3, 1, 1, 0, 7, 5, 0x81, 3, 8, 0, 10,
        ];
        let parsed = parse_hid_configuration(&keyboard).unwrap();
        assert_eq!(parsed.protocol, Protocol::Keyboard);
        assert_eq!(parsed.interface, 2);
        assert_eq!(endpoint_id(parsed.endpoint_address), 3);

        let mouse = [
            9, 2, 25, 0, 1, 1, 0, 0x80, 50, 9, 4, 1, 0, 1, 3, 1, 2, 0, 7, 5, 0x82, 3, 4, 0, 8,
        ];
        assert_eq!(
            parse_hid_configuration(&mouse).unwrap().protocol,
            Protocol::Mouse
        );
    }

    #[test]
    fn hid_keyboard_translation_preserves_exp_os_shortcuts() {
        assert_eq!(hid_key(0x04, 0), Some(b'a'));
        assert_eq!(hid_key(0x04, 0x02), Some(b'A'));
        assert_eq!(hid_key(0x2C, 0x08), Some(crate::input::KEY_SUPER_LAUNCHER));
        assert_eq!(hid_key(0x4F, 0x08), Some(crate::input::KEY_SUPER_RIGHT));
    }
}
