//! VirtIO legacy PCI block backend using the shared split-queue transport.

use super::{DeviceIdentity, StorageError, SECTOR_SIZE};
use crate::{
    pci,
    sync::SpinMutex,
    virtio::{self, Descriptor, LegacyTransport, QueueMemory, SplitQueue},
};
use core::sync::atomic::{AtomicU16, AtomicU64, AtomicU8, Ordering};

const REQUEST_IN: u32 = 0;
const REQUEST_OUT: u32 = 1;
const REQUEST_FLUSH: u32 = 4;
const FEATURE_FLUSH: u32 = 1 << 9;
const MAX_POLLS: usize = 12_000_000;
const NO_IRQ: u8 = u8::MAX;

#[repr(C, align(16))]
struct RequestHeader {
    kind: u32,
    reserved: u32,
    sector: u64,
}

#[repr(C, align(4096))]
struct SectorBuffer([u8; SECTOR_SIZE]);

static mut QUEUE_MEMORY: QueueMemory = QueueMemory::new();
static mut HEADER: RequestHeader = RequestHeader {
    kind: 0,
    reserved: 0,
    sector: 0,
};
static mut DATA: SectorBuffer = SectorBuffer([0; SECTOR_SIZE]);
static mut REQUEST_STATUS: u8 = 0xFF;
static IRQ_PORT: AtomicU16 = AtomicU16::new(0);
static IRQ_LINE: AtomicU8 = AtomicU8::new(NO_IRQ);
static IRQ_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Device {
    sectors: u32,
    identity: DeviceIdentity,
}

impl Device {
    pub const fn sectors(self) -> u32 {
        self.sectors
    }

    pub const fn identity(self) -> DeviceIdentity {
        self.identity
    }

    pub fn read_sector(self, lba: u32, output: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        if lba >= self.sectors {
            return Err(StorageError::OutOfRange);
        }
        let mut controller = CONTROLLER.lock();
        controller.request(REQUEST_IN, lba, false)?;
        unsafe {
            core::ptr::copy_nonoverlapping(
                core::ptr::addr_of!(DATA.0).cast::<u8>(),
                output.as_mut_ptr(),
                SECTOR_SIZE,
            );
        }
        Ok(())
    }

    pub fn write_sector(self, lba: u32, input: &[u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        self.write_sector_unflushed(lba, input)?;
        self.flush()
    }

    pub fn write_sector_unflushed(
        self,
        lba: u32,
        input: &[u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        if lba >= self.sectors {
            return Err(StorageError::OutOfRange);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                input.as_ptr(),
                core::ptr::addr_of_mut!(DATA.0).cast::<u8>(),
                SECTOR_SIZE,
            );
        }
        CONTROLLER.lock().request(REQUEST_OUT, lba, true)
    }

    pub fn flush(self) -> Result<(), StorageError> {
        CONTROLLER.lock().request(REQUEST_FLUSH, 0, false)
    }
}

struct Controller {
    transport: Option<LegacyTransport>,
    queue: Option<SplitQueue>,
    sectors: u32,
    identity: DeviceIdentity,
}

impl Controller {
    const fn new() -> Self {
        Self {
            transport: None,
            queue: None,
            sectors: 0,
            identity: DeviceIdentity::new(
                "virtio-block",
                None,
                0,
                ([0; 40], 0),
                ([0; 20], 0),
                false,
            ),
        }
    }

    fn request(&mut self, kind: u32, sector: u32, write: bool) -> Result<(), StorageError> {
        let queue = self.queue.as_mut().ok_or(StorageError::NoDevice)?;
        unsafe {
            HEADER.kind = kind;
            HEADER.reserved = 0;
            HEADER.sector = sector as u64;
            REQUEST_STATUS = 0xFF;
            queue
                .set_descriptor(
                    0,
                    Descriptor {
                        address: core::ptr::addr_of!(HEADER) as u64,
                        length: core::mem::size_of::<RequestHeader>() as u32,
                        flags: virtio::DESC_NEXT,
                        next: 1,
                    },
                )
                .map_err(map_queue)?;
            if kind == REQUEST_FLUSH {
                queue
                    .set_descriptor(
                        1,
                        Descriptor {
                            address: core::ptr::addr_of!(REQUEST_STATUS) as u64,
                            length: 1,
                            flags: virtio::DESC_WRITE,
                            next: 0,
                        },
                    )
                    .map_err(map_queue)?;
            } else {
                queue
                    .set_descriptor(
                        1,
                        Descriptor {
                            address: core::ptr::addr_of!(DATA.0) as u64,
                            length: SECTOR_SIZE as u32,
                            flags: virtio::DESC_NEXT | if write { 0 } else { virtio::DESC_WRITE },
                            next: 2,
                        },
                    )
                    .map_err(map_queue)?;
                queue
                    .set_descriptor(
                        2,
                        Descriptor {
                            address: core::ptr::addr_of!(REQUEST_STATUS) as u64,
                            length: 1,
                            flags: virtio::DESC_WRITE,
                            next: 0,
                        },
                    )
                    .map_err(map_queue)?;
            }
            let _ = queue.submit(0).map_err(map_queue)?;
        }
        for _ in 0..MAX_POLLS {
            if queue.pop_used().is_some() {
                let status =
                    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(REQUEST_STATUS)) };
                return if status == 0 {
                    Ok(())
                } else {
                    Err(StorageError::ControllerFault(status as u32))
                };
            }
            core::hint::spin_loop();
        }
        Err(StorageError::Timeout)
    }
}

static CONTROLLER: SpinMutex<Controller> = SpinMutex::new(Controller::new());

pub fn initialize() -> Result<Device, StorageError> {
    let mut controller = CONTROLLER.lock();
    if controller.queue.is_some() {
        return Ok(Device {
            sectors: controller.sectors,
            identity: controller.identity,
        });
    }
    let function = pci::find(|candidate| {
        candidate.vendor_id == virtio::VENDOR_ID
            && candidate.device_id == virtio::LEGACY_BLOCK_DEVICE
    })
    .ok_or(StorageError::NoDevice)?;
    let transport =
        LegacyTransport::claim(function, FEATURE_FLUSH).ok_or(StorageError::UnsupportedDevice)?;
    let queue = transport
        .configure_queue(0, core::ptr::addr_of_mut!(QUEUE_MEMORY))
        .map_err(map_queue)?;
    let capacity = transport.config_u32(0) as u64 | ((transport.config_u32(4) as u64) << 32);
    if capacity == 0 {
        transport.fail();
        return Err(StorageError::UnsupportedDevice);
    }
    let sectors = capacity.min(u32::MAX as u64) as u32;
    let (model, model_len) = identity_text::<40>(b"QEMU VirtIO Block");
    let mut serial_source = [0_u8; 20];
    let location = function.location;
    let template = b"virtio-00:00.0";
    serial_source[..template.len()].copy_from_slice(template);
    serial_source[7] = hex(location.bus >> 4);
    serial_source[8] = hex(location.bus);
    serial_source[10] = hex(location.slot >> 4);
    serial_source[11] = hex(location.slot);
    serial_source[13] = hex(location.function);
    let (serial, serial_len) = identity_text::<20>(&serial_source[..14]);
    let identity = DeviceIdentity::new(
        "virtio-block",
        Some(location),
        sectors,
        (model, model_len),
        (serial, serial_len),
        false,
    );
    controller.transport = Some(transport);
    controller.queue = Some(queue);
    controller.sectors = sectors;
    controller.identity = identity;
    transport.finish();
    let _ = crate::asl::claim(function, crate::asl::Owner::ExpStorage);
    if let Some(irq) = function.interrupt_line() {
        IRQ_PORT.store(transport.io_base(), Ordering::Release);
        IRQ_LINE.store(irq, Ordering::Release);
        let _ = crate::interrupts::enable_legacy_irq(irq);
    }
    crate::slog!(
        "EXPOS_VIRTIO_BLOCK_READY transport=legacy-pci queue={} sectors={} dma_bytes={} irq={:?}\r\n",
        controller.queue.as_ref().map_or(0, SplitQueue::size),
        sectors,
        virtio::QUEUE_BYTES,
        function.interrupt_line()
    );
    Ok(Device { sectors, identity })
}

pub fn handle_legacy_interrupt(irq: u8) {
    if IRQ_LINE.load(Ordering::Acquire) != irq {
        return;
    }
    let base = IRQ_PORT.load(Ordering::Acquire);
    if base != 0 {
        let status = unsafe { crate::port::inb(base + 0x13) };
        if status != 0 {
            IRQ_COUNT.fetch_add(1, Ordering::AcqRel);
        }
    }
}

fn map_queue(error: virtio::QueueError) -> StorageError {
    match error {
        virtio::QueueError::DmaAddress
        | virtio::QueueError::UnsupportedSize(_)
        | virtio::QueueError::NotifyAddress => StorageError::UnsupportedDevice,
        virtio::QueueError::DescriptorOutOfRange => StorageError::ControllerFault(0xFFFF_FFFE),
    }
}

fn identity_text<const N: usize>(source: &[u8]) -> ([u8; N], u8) {
    let mut output = [0_u8; N];
    let length = source.len().min(N);
    output[..length].copy_from_slice(&source[..length]);
    (output, length as u8)
}

const fn hex(value: u8) -> u8 {
    match value & 0xF {
        0..=9 => b'0' + (value & 0xF),
        nibble => b'a' + nibble - 10,
    }
}
