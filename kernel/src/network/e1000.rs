//! Intel 8254x/e1000 PCI Ethernet backend with bounded DMA rings.

use super::NetworkError;
use crate::{asl, pci, sync::SpinMutex};
use core::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};

const VENDOR_INTEL: u16 = 0x8086;
const CTRL: usize = 0x0000;
const STATUS: usize = 0x0008;
const ICR: usize = 0x00C0;
const ICS: usize = 0x00C8;
const IMS: usize = 0x00D0;
const IMC: usize = 0x00D8;
const RCTL: usize = 0x0100;
const TCTL: usize = 0x0400;
const TIPG: usize = 0x0410;
const RDBAL: usize = 0x2800;
const RDBAH: usize = 0x2804;
const RDLEN: usize = 0x2808;
const RDH: usize = 0x2810;
const RDT: usize = 0x2818;
const TDBAL: usize = 0x3800;
const TDBAH: usize = 0x3804;
const TDLEN: usize = 0x3808;
const TDH: usize = 0x3810;
const TDT: usize = 0x3818;
const RAL0: usize = 0x5400;
const RAH0: usize = 0x5404;
const CTRL_RESET: u32 = 1 << 26;
const INT_TXDW: u32 = 1;
const INT_LSC: u32 = 1 << 2;
const INT_RXT0: u32 = 1 << 7;
const RX_COUNT: usize = 32;
const TX_COUNT: usize = 8;
const RX_BYTES: usize = 2048;
const TX_BYTES: usize = 1536;
const MAX_POLLS: usize = 4_000_000;
const NO_IRQ: u8 = u8::MAX;

#[repr(C)]
#[derive(Clone, Copy)]
struct RxDescriptor {
    address: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TxDescriptor {
    address: u64,
    length: u16,
    checksum_offset: u8,
    command: u8,
    status: u8,
    checksum_start: u8,
    special: u16,
}

#[repr(C, align(128))]
struct RxRing([RxDescriptor; RX_COUNT]);
#[repr(C, align(128))]
struct TxRing([TxDescriptor; TX_COUNT]);
#[repr(C, align(4096))]
struct RxBuffers([[u8; RX_BYTES]; RX_COUNT]);
#[repr(C, align(4096))]
struct TxBuffers([[u8; TX_BYTES]; TX_COUNT]);

static mut RX_RING: RxRing = RxRing(
    [RxDescriptor {
        address: 0,
        length: 0,
        checksum: 0,
        status: 0,
        errors: 0,
        special: 0,
    }; RX_COUNT],
);
static mut TX_RING: TxRing = TxRing(
    [TxDescriptor {
        address: 0,
        length: 0,
        checksum_offset: 0,
        command: 0,
        status: 1,
        checksum_start: 0,
        special: 0,
    }; TX_COUNT],
);
static mut RX_BUFFERS: RxBuffers = RxBuffers([[0; RX_BYTES]; RX_COUNT]);
static mut TX_BUFFERS: TxBuffers = TxBuffers([[0; TX_BYTES]; TX_COUNT]);

struct State {
    ready: bool,
    base: usize,
    mac: [u8; 6],
    rx: usize,
    tx: usize,
}

impl State {
    const fn new() -> Self {
        Self {
            ready: false,
            base: 0,
            mac: [0; 6],
            rx: 0,
            tx: 0,
        }
    }
}

static STATE: SpinMutex<State> = SpinMutex::new(State::new());
static IRQ_BASE: AtomicUsize = AtomicUsize::new(0);
static IRQ_LINE: AtomicU8 = AtomicU8::new(NO_IRQ);
static IRQ_COUNT: AtomicU64 = AtomicU64::new(0);

pub fn initialize() -> Option<[u8; 6]> {
    let mut state = STATE.lock();
    if state.ready {
        return Some(state.mac);
    }
    let function = pci::find(|function| {
        function.vendor_id == VENDOR_INTEL
            && matches!(function.device_id, 0x100E | 0x100F | 0x10D3 | 0x153A)
    })?;
    let bar = function.memory_bar(0)?;
    if bar.address < 0x10_0000 || bar.address.checked_add(0x20_000)? > 0x1_0000_0000 {
        return None;
    }
    let irq = function.interrupt_line()?;
    function.enable_memory_bus_master();
    function.write(0x04, function.read(0x04) & !(1 << 10));
    let base = bar.address as usize;
    unsafe {
        write(base, IMC, u32::MAX);
        write(base, CTRL, read(base, CTRL) | CTRL_RESET);
    }
    let mut reset = false;
    for _ in 0..MAX_POLLS {
        if unsafe { read(base, CTRL) } & CTRL_RESET == 0 {
            reset = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !reset {
        return None;
    }
    let low = unsafe { read(base, RAL0) };
    let high = unsafe { read(base, RAH0) };
    let mac = [
        low as u8,
        (low >> 8) as u8,
        (low >> 16) as u8,
        (low >> 24) as u8,
        high as u8,
        (high >> 8) as u8,
    ];
    if mac == [0; 6] || mac == [0xFF; 6] {
        return None;
    }
    unsafe {
        for index in 0..RX_COUNT {
            RX_RING.0[index] = RxDescriptor {
                address: core::ptr::addr_of!(RX_BUFFERS.0[index]) as u64,
                length: 0,
                checksum: 0,
                status: 0,
                errors: 0,
                special: 0,
            };
        }
        for index in 0..TX_COUNT {
            TX_RING.0[index] = TxDescriptor {
                address: core::ptr::addr_of!(TX_BUFFERS.0[index]) as u64,
                length: 0,
                checksum_offset: 0,
                command: 0,
                status: 1,
                checksum_start: 0,
                special: 0,
            };
        }
        let rx_address = core::ptr::addr_of!(RX_RING.0) as u64;
        let tx_address = core::ptr::addr_of!(TX_RING.0) as u64;
        if rx_address >= 0x1_0000_0000 || tx_address >= 0x1_0000_0000 {
            return None;
        }
        write(base, RDBAL, rx_address as u32);
        write(base, RDBAH, 0);
        write(base, RDLEN, core::mem::size_of::<RxRing>() as u32);
        write(base, RDH, 0);
        write(base, RDT, (RX_COUNT - 1) as u32);
        write(base, TDBAL, tx_address as u32);
        write(base, TDBAH, 0);
        write(base, TDLEN, core::mem::size_of::<TxRing>() as u32);
        write(base, TDH, 0);
        write(base, TDT, 0);
        write(base, TIPG, 10 | (8 << 10) | (6 << 20));
        write(base, TCTL, (1 << 1) | (1 << 3) | (0x10 << 4) | (0x40 << 12));
        write(base, RCTL, (1 << 1) | (1 << 15) | (1 << 26));
        let _ = read(base, ICR);
        write(base, IMS, INT_TXDW | INT_LSC | INT_RXT0);
    }
    IRQ_BASE.store(base, Ordering::Release);
    IRQ_LINE.store(irq, Ordering::Release);
    IRQ_COUNT.store(0, Ordering::Release);
    // QEMU exposes a stable firmware INTx pin for both 82540EM and 82574L.
    // Route it through the I/O APIC so completion works for both models; the
    // generic MSI layer remains available to devices whose tables are proven.
    let message_interrupt = false;
    if !message_interrupt && !crate::interrupts::enable_legacy_irq(irq) {
        unsafe { write(base, IMC, u32::MAX) };
        return None;
    }
    // Assert one software interrupt cause after routing is live. The normal
    // TX/RX causes use the same ICR/IMS completion path; this also catches a
    // firmware route that silently points at an unusable IRQ.
    unsafe { write(base, ICS, INT_LSC) };
    state.ready = true;
    state.base = base;
    state.mac = mac;
    state.rx = 0;
    state.tx = 0;
    let _ = asl::claim(function, asl::Owner::ExpNetwork);
    crate::slog!(
        "EXPOS_E1000_READY device={:04x} family=e1000/e1000e rx_descriptors={} tx_descriptors={} completion={} irq={} dma_bound=below-4g\r\n",
        function.device_id,
        RX_COUNT,
        TX_COUNT,
        if message_interrupt { "msi" } else { "intx" },
        irq
    );
    Some(mac)
}

pub fn link_up() -> bool {
    let state = STATE.lock();
    state.ready && unsafe { read(state.base, STATUS) } & 2 != 0
}

pub fn send(frame: &[u8]) -> Result<(), NetworkError> {
    if frame.len() > TX_BYTES {
        return Err(NetworkError::PacketTooLarge);
    }
    let mut state = STATE.lock();
    if !state.ready {
        return Err(NetworkError::NoDevice);
    }
    let index = state.tx;
    let descriptor = unsafe { core::ptr::addr_of_mut!(TX_RING.0[index]) };
    if unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) } & 1 == 0 {
        return Err(NetworkError::TransmitFailed);
    }
    let length = frame.len().max(60);
    unsafe {
        TX_BUFFERS.0[index][..length].fill(0);
        TX_BUFFERS.0[index][..frame.len()].copy_from_slice(frame);
        (*descriptor).length = length as u16;
        (*descriptor).command = 1 | 2 | 8;
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*descriptor).status), 0);
        core::sync::atomic::fence(Ordering::Release);
        write(state.base, TDT, ((index + 1) % TX_COUNT) as u32);
    }
    // Submission is asynchronous. INTx publishes completion without trapping
    // the Ring 3 service inside a kernel call with interrupts masked; the next
    // reuse of this descriptor is gated by its DD status above.
    state.tx = (index + 1) % TX_COUNT;
    Ok(())
}

pub fn receive(output: &mut [u8]) -> Result<usize, NetworkError> {
    let mut state = STATE.lock();
    if !state.ready {
        return Err(NetworkError::NoDevice);
    }
    let index = state.rx;
    let descriptor = unsafe { core::ptr::addr_of_mut!(RX_RING.0[index]) };
    let status = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).status)) };
    if status & 1 == 0 {
        return Ok(0);
    }
    let length =
        unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).length)) } as usize;
    let valid = status & 2 != 0
        && unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*descriptor).errors)) } == 0
        && length <= output.len()
        && length <= RX_BYTES;
    if valid {
        unsafe { output[..length].copy_from_slice(&RX_BUFFERS.0[index][..length]) };
    }
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!((*descriptor).status), 0);
        core::sync::atomic::fence(Ordering::Release);
        write(state.base, RDT, index as u32);
    }
    state.rx = (index + 1) % RX_COUNT;
    if valid {
        Ok(length)
    } else {
        Err(NetworkError::ReceiveFailed)
    }
}

pub fn handle_legacy_interrupt(irq: u8) {
    if IRQ_LINE.load(Ordering::Acquire) != irq {
        return;
    }
    let base = IRQ_BASE.load(Ordering::Acquire);
    if base != 0 {
        let cause = unsafe { read(base, ICR) };
        if cause != 0 {
            IRQ_COUNT.fetch_add(1, Ordering::AcqRel);
        }
    }
}

pub fn handle_message_interrupt() {
    let base = IRQ_BASE.load(Ordering::Acquire);
    if base != 0 {
        let cause = unsafe { read(base, ICR) };
        if cause != 0 {
            IRQ_COUNT.fetch_add(1, Ordering::AcqRel);
        }
    }
}

pub fn interrupt_count() -> u64 {
    IRQ_COUNT.load(Ordering::Acquire)
}

unsafe fn read(base: usize, register: usize) -> u32 {
    unsafe { core::ptr::read_volatile((base + register) as *const u32) }
}

unsafe fn write(base: usize, register: usize, value: u32) {
    unsafe { core::ptr::write_volatile((base + register) as *mut u32, value) }
}
