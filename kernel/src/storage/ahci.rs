//! AHCI 1.x SATA block transport.
//!
//! The first implementation deliberately uses one command slot and bounded
//! polling.  It nevertheless uses the real AHCI DMA command-list/FIS/PRDT
//! protocol, so it works on firmware-configured controllers without ATA I/O
//! ports.  Interrupt delivery can replace the completion poll through the
//! shared MSI layer without changing the storage contract.

use super::{StorageError, SECTOR_SIZE};
use crate::{pci, sync::SpinMutex};
use core::sync::atomic::{compiler_fence, Ordering};

const GHC: usize = 0x04;
const PI: usize = 0x0C;
const BOHC: usize = 0x28;
const GHC_AE: u32 = 1 << 31;
const PORT_BASE: usize = 0x100;
const PORT_STRIDE: usize = 0x80;
const PX_CLB: usize = 0x00;
const PX_CLBU: usize = 0x04;
const PX_FB: usize = 0x08;
const PX_FBU: usize = 0x0C;
const PX_IS: usize = 0x10;
const PX_IE: usize = 0x14;
const PX_CMD: usize = 0x18;
const PX_TFD: usize = 0x20;
const PX_SIG: usize = 0x24;
const PX_SSTS: usize = 0x28;
const PX_SERR: usize = 0x30;
const PX_CI: usize = 0x38;
const CMD_ST: u32 = 1;
const CMD_FRE: u32 = 1 << 4;
const CMD_FR: u32 = 1 << 14;
const CMD_CR: u32 = 1 << 15;
const TFD_ERR: u32 = 1;
const TFD_DRQ: u32 = 1 << 3;
const TFD_BSY: u32 = 1 << 7;
const IS_TFES: u32 = 1 << 30;
const SATA_SIGNATURE: u32 = 0x0000_0101;
const FIS_REG_H2D: u8 = 0x27;
const ATA_IDENTIFY: u8 = 0xEC;
const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE_DMA_EXT: u8 = 0x35;
const ATA_FLUSH_CACHE_EXT: u8 = 0xEA;
const MAX_POLLS: usize = 8_000_000;

#[repr(C, align(1024))]
struct CommandList([u32; 256]);

#[repr(C, align(256))]
struct ReceivedFis([u8; 256]);

#[repr(C, align(128))]
struct CommandTable([u8; 256]);

#[repr(C, align(4096))]
struct DataBuffer([u8; 4096]);

static mut COMMAND_LIST: CommandList = CommandList([0; 256]);
static mut RECEIVED_FIS: ReceivedFis = ReceivedFis([0; 256]);
static mut COMMAND_TABLE: CommandTable = CommandTable([0; 256]);
static mut DATA_BUFFER: DataBuffer = DataBuffer([0; 4096]);
static DEVICE: SpinMutex<Option<Device>> = SpinMutex::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Device {
    abar: usize,
    port: u8,
    sectors: u32,
}

impl Device {
    pub const fn sectors(self) -> u32 {
        self.sectors
    }

    pub fn read_sector(self, lba: u32, output: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        self.check_lba(lba)?;
        unsafe {
            issue_data(self, ATA_READ_DMA_EXT, lba as u64, false, SECTOR_SIZE)?;
            core::ptr::copy_nonoverlapping(
                core::ptr::addr_of!(DATA_BUFFER.0).cast::<u8>(),
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
        self.check_lba(lba)?;
        unsafe {
            core::ptr::copy_nonoverlapping(
                input.as_ptr(),
                core::ptr::addr_of_mut!(DATA_BUFFER.0).cast::<u8>(),
                SECTOR_SIZE,
            );
            issue_data(self, ATA_WRITE_DMA_EXT, lba as u64, true, SECTOR_SIZE)
        }
    }

    pub fn flush(self) -> Result<(), StorageError> {
        unsafe { issue_non_data(self, ATA_FLUSH_CACHE_EXT) }
    }

    fn check_lba(self, lba: u32) -> Result<(), StorageError> {
        if lba >= self.sectors {
            Err(StorageError::OutOfRange)
        } else {
            Ok(())
        }
    }
}

pub fn initialize() -> Result<Device, StorageError> {
    let mut cached = DEVICE.lock();
    if let Some(device) = *cached {
        return Ok(device);
    }
    let device = initialize_controller()?;
    *cached = Some(device);
    Ok(device)
}

fn initialize_controller() -> Result<Device, StorageError> {
    let function = pci::find_class(0x01, 0x06, Some(0x01)).ok_or(StorageError::NoDevice)?;
    let bar = function
        .memory_bar(5)
        .ok_or(StorageError::UnsupportedDevice)?;
    if bar.address < 0x10_0000
        || bar
            .address
            .checked_add(0x1100)
            .is_none_or(|end| end > 0x1_0000_0000)
    {
        return Err(StorageError::UnsupportedDevice);
    }
    function.enable_memory_bus_master();
    let abar = bar.address as usize;
    unsafe {
        let cap2 = mmio_read(abar + 0x24);
        if cap2 & 1 != 0 {
            let mut bohc = mmio_read(abar + BOHC);
            bohc |= 1 << 1;
            mmio_write(abar + BOHC, bohc);
            wait_until(|| mmio_read(abar + BOHC) & 1 == 0)?;
        }
        mmio_write(abar + GHC, mmio_read(abar + GHC) | GHC_AE);
        let implemented = mmio_read(abar + PI);
        for port in 0_u8..32 {
            if implemented & (1_u32 << port) == 0 {
                continue;
            }
            let base = port_base(abar, port);
            let status = mmio_read(base + PX_SSTS);
            if status & 0xF != 3 || (status >> 8) & 0xF != 1 {
                continue;
            }
            let signature = mmio_read(base + PX_SIG);
            if signature != SATA_SIGNATURE {
                continue;
            }
            stop_port(base)?;
            configure_port(base)?;
            start_port(base)?;
            let provisional = Device {
                abar,
                port,
                sectors: u32::MAX,
            };
            issue_identify(provisional)?;
            let sectors = identify_sector_count();
            if sectors == 0 {
                return Err(StorageError::UnsupportedDevice);
            }
            let _ = crate::asl::claim(function, crate::asl::Owner::ExpStorage);
            if let Some(binding) = crate::interrupts::prepare(function) {
                crate::slog!(
                    "EXPOS_INTERRUPT_PREPARED driver=ahci kind={:?} vector={:#x} armed=false\r\n",
                    binding.kind,
                    binding.vector
                );
            }
            return Ok(Device {
                sectors: sectors.min(u32::MAX as u64) as u32,
                ..provisional
            });
        }
    }
    Err(StorageError::NoDevice)
}

unsafe fn configure_port(base: usize) -> Result<(), StorageError> {
    unsafe {
        core::ptr::write_bytes(
            core::ptr::addr_of_mut!(COMMAND_LIST.0).cast::<u8>(),
            0,
            1024,
        );
        core::ptr::write_bytes(core::ptr::addr_of_mut!(RECEIVED_FIS.0).cast::<u8>(), 0, 256);
        core::ptr::write_bytes(
            core::ptr::addr_of_mut!(COMMAND_TABLE.0).cast::<u8>(),
            0,
            256,
        );
        let command_list = dma_address(core::ptr::addr_of!(COMMAND_LIST.0).cast::<u8>())?;
        let received_fis = dma_address(core::ptr::addr_of!(RECEIVED_FIS.0).cast::<u8>())?;
        mmio_write(base + PX_CLB, command_list as u32);
        mmio_write(base + PX_CLBU, (command_list >> 32) as u32);
        mmio_write(base + PX_FB, received_fis as u32);
        mmio_write(base + PX_FBU, (received_fis >> 32) as u32);
        mmio_write(base + PX_IE, 0);
        mmio_write(base + PX_IS, u32::MAX);
        mmio_write(base + PX_SERR, u32::MAX);
    }
    Ok(())
}

unsafe fn stop_port(base: usize) -> Result<(), StorageError> {
    unsafe {
        let mut command = mmio_read(base + PX_CMD);
        command &= !CMD_ST;
        mmio_write(base + PX_CMD, command);
        wait_until(|| mmio_read(base + PX_CMD) & CMD_CR == 0)?;
        command = mmio_read(base + PX_CMD) & !CMD_FRE;
        mmio_write(base + PX_CMD, command);
        wait_until(|| mmio_read(base + PX_CMD) & CMD_FR == 0)
    }
}

unsafe fn start_port(base: usize) -> Result<(), StorageError> {
    unsafe {
        wait_until(|| mmio_read(base + PX_CMD) & CMD_CR == 0)?;
        let command = mmio_read(base + PX_CMD) | CMD_FRE | CMD_ST;
        mmio_write(base + PX_CMD, command);
    }
    Ok(())
}

unsafe fn issue_identify(device: Device) -> Result<(), StorageError> {
    unsafe { issue_data(device, ATA_IDENTIFY, 0, false, SECTOR_SIZE) }
}

unsafe fn issue_data(
    device: Device,
    command: u8,
    lba: u64,
    write: bool,
    bytes: usize,
) -> Result<(), StorageError> {
    unsafe {
        prepare_command(command, lba, write, Some(bytes))?;
        execute(device)
    }
}

unsafe fn issue_non_data(device: Device, command: u8) -> Result<(), StorageError> {
    unsafe {
        prepare_command(command, 0, false, None)?;
        execute(device)
    }
}

unsafe fn prepare_command(
    command: u8,
    lba: u64,
    write: bool,
    data_bytes: Option<usize>,
) -> Result<(), StorageError> {
    unsafe {
        core::ptr::write_bytes(
            core::ptr::addr_of_mut!(COMMAND_LIST.0).cast::<u8>(),
            0,
            1024,
        );
        core::ptr::write_bytes(
            core::ptr::addr_of_mut!(COMMAND_TABLE.0).cast::<u8>(),
            0,
            256,
        );
        let table = dma_address(core::ptr::addr_of!(COMMAND_TABLE.0).cast::<u8>())?;
        let header = core::ptr::addr_of_mut!(COMMAND_LIST.0).cast::<u32>();
        let mut flags = 5_u32;
        if write {
            flags |= 1 << 6;
        }
        if data_bytes.is_some() {
            flags |= 1 << 16;
        }
        core::ptr::write_volatile(header, flags);
        core::ptr::write_volatile(header.add(2), table as u32);
        core::ptr::write_volatile(header.add(3), (table >> 32) as u32);

        let fis = core::ptr::addr_of_mut!(COMMAND_TABLE.0).cast::<u8>();
        core::ptr::write(fis, FIS_REG_H2D);
        core::ptr::write(fis.add(1), 0x80);
        core::ptr::write(fis.add(2), command);
        if matches!(command, ATA_READ_DMA_EXT | ATA_WRITE_DMA_EXT) {
            core::ptr::write(fis.add(4), lba as u8);
            core::ptr::write(fis.add(5), (lba >> 8) as u8);
            core::ptr::write(fis.add(6), (lba >> 16) as u8);
            core::ptr::write(fis.add(7), 1 << 6);
            core::ptr::write(fis.add(8), (lba >> 24) as u8);
            core::ptr::write(fis.add(9), (lba >> 32) as u8);
            core::ptr::write(fis.add(10), (lba >> 40) as u8);
            core::ptr::write(fis.add(12), 1);
        }
        if let Some(bytes) = data_bytes {
            let data = dma_address(core::ptr::addr_of!(DATA_BUFFER.0).cast::<u8>())?;
            let prdt = fis.add(128).cast::<u32>();
            core::ptr::write_volatile(prdt, data as u32);
            core::ptr::write_volatile(prdt.add(1), (data >> 32) as u32);
            core::ptr::write_volatile(prdt.add(2), 0);
            core::ptr::write_volatile(prdt.add(3), (bytes as u32 - 1) | (1 << 31));
        }
    }
    Ok(())
}

unsafe fn execute(device: Device) -> Result<(), StorageError> {
    let base = port_base(device.abar, device.port);
    unsafe {
        wait_until(|| mmio_read(base + PX_TFD) & (TFD_BSY | TFD_DRQ) == 0)?;
        mmio_write(base + PX_IS, u32::MAX);
        compiler_fence(Ordering::Release);
        mmio_write(base + PX_CI, 1);
        for _ in 0..MAX_POLLS {
            let pending = mmio_read(base + PX_CI) & 1 != 0;
            let interrupt = mmio_read(base + PX_IS);
            let task = mmio_read(base + PX_TFD);
            if interrupt & IS_TFES != 0 || task & TFD_ERR != 0 {
                return Err(StorageError::ControllerFault(task));
            }
            if !pending {
                compiler_fence(Ordering::Acquire);
                return Ok(());
            }
            core::hint::spin_loop();
        }
    }
    Err(StorageError::Timeout)
}

fn identify_sector_count() -> u64 {
    let word = |index: usize| unsafe {
        let bytes = core::ptr::addr_of!(DATA_BUFFER.0).cast::<u8>();
        u16::from_le_bytes([
            core::ptr::read(bytes.add(index * 2)),
            core::ptr::read(bytes.add(index * 2 + 1)),
        ])
    };
    if word(83) & (1 << 10) != 0 {
        word(100) as u64
            | ((word(101) as u64) << 16)
            | ((word(102) as u64) << 32)
            | ((word(103) as u64) << 48)
    } else {
        word(60) as u64 | ((word(61) as u64) << 16)
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) -> Result<(), StorageError> {
    for _ in 0..MAX_POLLS {
        if ready() {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(StorageError::Timeout)
}

fn dma_address<T>(pointer: *const T) -> Result<u64, StorageError> {
    let address = pointer as u64;
    if address == 0 || address >= 0x1_0000_0000 {
        Err(StorageError::UnsupportedDevice)
    } else {
        Ok(address)
    }
}

const fn port_base(abar: usize, port: u8) -> usize {
    abar + PORT_BASE + port as usize * PORT_STRIDE
}

unsafe fn mmio_read(address: usize) -> u32 {
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

unsafe fn mmio_write(address: usize, value: u32) {
    unsafe { core::ptr::write_volatile(address as *mut u32, value) }
}
