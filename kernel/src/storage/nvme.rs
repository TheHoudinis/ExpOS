//! NVMe 1.x namespace transport using real submission/completion queues.
//!
//! Queue memory is statically bounded, page aligned and identity-mapped by the
//! current x86_64 bootstrap.  Completion is polled for now; the queue layout is
//! already compatible with the MSI-X work started in `interrupts`.

use super::{StorageError, SECTOR_SIZE};
use crate::{pci, sync::SpinMutex};
use core::sync::atomic::{compiler_fence, Ordering};

const CAP: usize = 0x00;
const INTMS: usize = 0x0C;
const CC: usize = 0x14;
const CSTS: usize = 0x1C;
const AQA: usize = 0x24;
const ASQ: usize = 0x28;
const ACQ: usize = 0x30;
const DOORBELL_BASE: usize = 0x1000;
const CC_ENABLE: u32 = 1;
const CSTS_READY: u32 = 1;
const CSTS_FATAL: u32 = 1 << 1;
const ADMIN_CREATE_IO_SQ: u8 = 0x01;
const ADMIN_CREATE_IO_CQ: u8 = 0x05;
const ADMIN_IDENTIFY: u8 = 0x06;
const NVM_FLUSH: u8 = 0x00;
const NVM_WRITE: u8 = 0x01;
const NVM_READ: u8 = 0x02;
const QUEUE_DEPTH: usize = 16;
const MAX_POLLS: usize = 12_000_000;

#[repr(C, align(4096))]
struct QueuePage([u32; 1024]);

#[repr(C, align(4096))]
struct DataPage([u8; 4096]);

static mut ADMIN_SQ: QueuePage = QueuePage([0; 1024]);
static mut ADMIN_CQ: QueuePage = QueuePage([0; 1024]);
static mut IO_SQ: QueuePage = QueuePage([0; 1024]);
static mut IO_CQ: QueuePage = QueuePage([0; 1024]);
static mut DATA: DataPage = DataPage([0; 4096]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Device {
    sectors: u32,
}

impl Device {
    pub const fn sectors(self) -> u32 {
        self.sectors
    }

    pub fn read_sector(self, lba: u32, output: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        if lba >= self.sectors {
            return Err(StorageError::OutOfRange);
        }
        let mut state = STATE.lock();
        state.io(NVM_READ, lba, false)?;
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
        STATE.lock().io(NVM_WRITE, lba, true)
    }

    pub fn flush(self) -> Result<(), StorageError> {
        STATE.lock().io(NVM_FLUSH, 0, false)
    }
}

struct Controller {
    ready: bool,
    bar: usize,
    stride: usize,
    queue_depth: u16,
    next_cid: u16,
    admin_sq_tail: u16,
    admin_cq_head: u16,
    admin_phase: bool,
    io_sq_tail: u16,
    io_cq_head: u16,
    io_phase: bool,
    sectors: u32,
}

impl Controller {
    const fn new() -> Self {
        Self {
            ready: false,
            bar: 0,
            stride: 0,
            queue_depth: 0,
            next_cid: 1,
            admin_sq_tail: 0,
            admin_cq_head: 0,
            admin_phase: true,
            io_sq_tail: 0,
            io_cq_head: 0,
            io_phase: true,
            sectors: 0,
        }
    }

    fn initialize(&mut self) -> Result<Device, StorageError> {
        if self.ready {
            return Ok(Device {
                sectors: self.sectors,
            });
        }
        let function = pci::find_class(0x01, 0x08, Some(0x02)).ok_or(StorageError::NoDevice)?;
        let bar = function
            .memory_bar(0)
            .ok_or(StorageError::UnsupportedDevice)?;
        if bar.address < 0x10_0000
            || bar
                .address
                .checked_add(0x2000)
                .is_none_or(|end| end > 0x1_0000_0000)
        {
            return Err(StorageError::UnsupportedDevice);
        }
        function.enable_memory_bus_master();
        self.bar = bar.address as usize;
        let capabilities = unsafe { mmio_read64(self.bar + CAP) };
        if (capabilities >> 48) & 0xF != 0 || (capabilities >> 37) & 1 == 0 {
            return Err(StorageError::UnsupportedDevice);
        }
        self.stride = 4_usize << ((capabilities >> 32) & 0xF);
        self.queue_depth = ((capabilities as u16).wrapping_add(1)).min(QUEUE_DEPTH as u16);
        if self.queue_depth < 2 {
            return Err(StorageError::UnsupportedDevice);
        }
        unsafe {
            mmio_write32(self.bar + INTMS, u32::MAX);
            mmio_write32(self.bar + CC, 0);
        }
        self.wait_ready(false)?;
        unsafe {
            clear_page(core::ptr::addr_of_mut!(ADMIN_SQ.0));
            clear_page(core::ptr::addr_of_mut!(ADMIN_CQ.0));
            clear_page(core::ptr::addr_of_mut!(IO_SQ.0));
            clear_page(core::ptr::addr_of_mut!(IO_CQ.0));
            let admin_sq = dma_address(core::ptr::addr_of!(ADMIN_SQ.0))?;
            let admin_cq = dma_address(core::ptr::addr_of!(ADMIN_CQ.0))?;
            let depth = self.queue_depth as u32 - 1;
            mmio_write32(self.bar + AQA, depth | (depth << 16));
            mmio_write64(self.bar + ASQ, admin_sq);
            mmio_write64(self.bar + ACQ, admin_cq);
            mmio_write32(self.bar + CC, CC_ENABLE | (6 << 16) | (4 << 20));
        }
        self.wait_ready(true)?;
        self.admin_sq_tail = 0;
        self.admin_cq_head = 0;
        self.admin_phase = true;
        self.io_sq_tail = 0;
        self.io_cq_head = 0;
        self.io_phase = true;

        let io_cq = unsafe { dma_address(core::ptr::addr_of!(IO_CQ.0))? };
        let mut command = [0_u32; 16];
        command[0] = ADMIN_CREATE_IO_CQ as u32;
        command[6] = io_cq as u32;
        command[7] = (io_cq >> 32) as u32;
        command[10] = 1 | ((self.queue_depth as u32 - 1) << 16);
        command[11] = 1;
        self.submit_admin(command)?;

        let io_sq = unsafe { dma_address(core::ptr::addr_of!(IO_SQ.0))? };
        command = [0; 16];
        command[0] = ADMIN_CREATE_IO_SQ as u32;
        command[6] = io_sq as u32;
        command[7] = (io_sq >> 32) as u32;
        command[10] = 1 | ((self.queue_depth as u32 - 1) << 16);
        command[11] = 1 | (1 << 16);
        self.submit_admin(command)?;

        unsafe { core::ptr::write_bytes(core::ptr::addr_of_mut!(DATA.0).cast::<u8>(), 0, 4096) };
        let data = unsafe { dma_address(core::ptr::addr_of!(DATA.0))? };
        command = [0; 16];
        command[0] = ADMIN_IDENTIFY as u32;
        command[1] = 1;
        command[6] = data as u32;
        command[7] = (data >> 32) as u32;
        command[10] = 0;
        self.submit_admin(command)?;
        let namespace_size = unsafe {
            u64::from_le(core::ptr::read_unaligned(
                core::ptr::addr_of!(DATA.0).cast::<u64>(),
            ))
        };
        let format =
            unsafe { core::ptr::read(core::ptr::addr_of!(DATA.0).cast::<u8>().add(26)) } & 0xF;
        let lba_shift = unsafe {
            core::ptr::read(
                core::ptr::addr_of!(DATA.0)
                    .cast::<u8>()
                    .add(128 + format as usize * 4 + 2),
            )
        };
        if namespace_size == 0 || lba_shift != 9 {
            return Err(StorageError::UnsupportedDevice);
        }
        self.sectors = namespace_size.min(u32::MAX as u64) as u32;
        self.ready = true;
        let _ = crate::asl::claim(function, crate::asl::Owner::ExpStorage);
        if let Some(binding) = crate::interrupts::prepare(function) {
            crate::slog!(
                "EXPOS_INTERRUPT_PREPARED driver=nvme kind={:?} vector={:#x} armed=false\r\n",
                binding.kind,
                binding.vector
            );
        }
        Ok(Device {
            sectors: self.sectors,
        })
    }

    fn submit_admin(&mut self, mut command: [u32; 16]) -> Result<(), StorageError> {
        let cid = self.allocate_cid();
        command[0] |= (cid as u32) << 16;
        unsafe {
            write_command(
                core::ptr::addr_of_mut!(ADMIN_SQ.0),
                self.admin_sq_tail,
                &command,
            );
        }
        self.admin_sq_tail = increment(self.admin_sq_tail, self.queue_depth);
        compiler_fence(Ordering::Release);
        unsafe { mmio_write32(self.doorbell(0, false), self.admin_sq_tail as u32) };
        let result = unsafe {
            wait_completion(
                core::ptr::addr_of!(ADMIN_CQ.0),
                &mut self.admin_cq_head,
                &mut self.admin_phase,
                self.queue_depth,
                cid,
            )
        };
        unsafe { mmio_write32(self.doorbell(0, true), self.admin_cq_head as u32) };
        result
    }

    fn io(&mut self, opcode: u8, lba: u32, write: bool) -> Result<(), StorageError> {
        if !self.ready {
            return Err(StorageError::NoDevice);
        }
        let cid = self.allocate_cid();
        let mut command = [0_u32; 16];
        command[0] = opcode as u32 | ((cid as u32) << 16);
        command[1] = 1;
        if opcode != NVM_FLUSH {
            let data = unsafe { dma_address(core::ptr::addr_of!(DATA.0))? };
            command[6] = data as u32;
            command[7] = (data >> 32) as u32;
            command[10] = lba;
            command[11] = 0;
            command[12] = if write { 1 << 30 } else { 0 };
        }
        unsafe { write_command(core::ptr::addr_of_mut!(IO_SQ.0), self.io_sq_tail, &command) };
        self.io_sq_tail = increment(self.io_sq_tail, self.queue_depth);
        compiler_fence(Ordering::Release);
        unsafe { mmio_write32(self.doorbell(1, false), self.io_sq_tail as u32) };
        let result = unsafe {
            wait_completion(
                core::ptr::addr_of!(IO_CQ.0),
                &mut self.io_cq_head,
                &mut self.io_phase,
                self.queue_depth,
                cid,
            )
        };
        unsafe { mmio_write32(self.doorbell(1, true), self.io_cq_head as u32) };
        result
    }

    fn wait_ready(&self, expected: bool) -> Result<(), StorageError> {
        for _ in 0..MAX_POLLS {
            let status = unsafe { mmio_read32(self.bar + CSTS) };
            if status & CSTS_FATAL != 0 {
                return Err(StorageError::ControllerFault(status));
            }
            if (status & CSTS_READY != 0) == expected {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(StorageError::Timeout)
    }

    fn allocate_cid(&mut self) -> u16 {
        let cid = self.next_cid;
        self.next_cid = self.next_cid.wrapping_add(1).max(1);
        cid
    }

    const fn doorbell(&self, queue: usize, completion: bool) -> usize {
        self.bar + DOORBELL_BASE + (queue * 2 + completion as usize) * self.stride
    }
}

static STATE: SpinMutex<Controller> = SpinMutex::new(Controller::new());

pub fn initialize() -> Result<Device, StorageError> {
    STATE.lock().initialize()
}

unsafe fn wait_completion(
    queue: *const [u32; 1024],
    head: &mut u16,
    phase: &mut bool,
    depth: u16,
    cid: u16,
) -> Result<(), StorageError> {
    for _ in 0..MAX_POLLS {
        let entry = unsafe { queue.cast::<u32>().add(*head as usize * 4) };
        let status = unsafe { core::ptr::read_volatile(entry.add(3)) };
        if ((status >> 16) & 1 != 0) == *phase {
            compiler_fence(Ordering::Acquire);
            let completed_cid = status as u16;
            let code = status >> 17;
            if completed_cid != cid || code != 0 {
                return Err(StorageError::ControllerFault(status));
            }
            *head += 1;
            if *head == depth {
                *head = 0;
                *phase = !*phase;
            }
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(StorageError::Timeout)
}

unsafe fn write_command(queue: *mut [u32; 1024], tail: u16, command: &[u32; 16]) {
    let destination = queue.cast::<u32>().wrapping_add(tail as usize * 16);
    for (index, value) in command.iter().enumerate() {
        unsafe { core::ptr::write_volatile(destination.add(index), *value) };
    }
}

const fn increment(value: u16, depth: u16) -> u16 {
    if value + 1 == depth {
        0
    } else {
        value + 1
    }
}

unsafe fn clear_page(page: *mut [u32; 1024]) {
    unsafe { core::ptr::write_bytes(page.cast::<u8>(), 0, 4096) }
}

unsafe fn dma_address<T>(pointer: *const T) -> Result<u64, StorageError> {
    let address = pointer as u64;
    if address == 0 || address >= 0x1_0000_0000 || address & 0xFFF != 0 {
        Err(StorageError::UnsupportedDevice)
    } else {
        Ok(address)
    }
}

unsafe fn mmio_read32(address: usize) -> u32 {
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

unsafe fn mmio_write32(address: usize, value: u32) {
    unsafe { core::ptr::write_volatile(address as *mut u32, value) }
}

unsafe fn mmio_read64(address: usize) -> u64 {
    let low = unsafe { mmio_read32(address) } as u64;
    let high = unsafe { mmio_read32(address + 4) } as u64;
    low | (high << 32)
}

unsafe fn mmio_write64(address: usize, value: u64) {
    unsafe {
        mmio_write32(address, value as u32);
        mmio_write32(address + 4, (value >> 32) as u32);
    }
}
