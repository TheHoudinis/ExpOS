//! Minimal primary-master ATA PIO block transport for the ExpOS state disk.
//!
//! This driver deliberately exposes sectors rather than a filesystem.  The
//! persistent state layer above it owns versioning, checksums, and atomic slot
//! selection.  Polls are bounded so absent or faulty hardware cannot hang boot.

use crate::port;

pub const SECTOR_SIZE: usize = 512;

const DATA: u16 = 0x1F0;
const ERROR: u16 = 0x1F1;
const SECTOR_COUNT: u16 = 0x1F2;
const LBA_LOW: u16 = 0x1F3;
const LBA_MID: u16 = 0x1F4;
const LBA_HIGH: u16 = 0x1F5;
const DRIVE: u16 = 0x1F6;
const STATUS_COMMAND: u16 = 0x1F7;
const ALT_STATUS: u16 = 0x3F6;

const STATUS_ERROR: u8 = 1 << 0;
const STATUS_DATA_REQUEST: u8 = 1 << 3;
const STATUS_DEVICE_FAULT: u8 = 1 << 5;
const STATUS_READY: u8 = 1 << 6;
const STATUS_BUSY: u8 = 1 << 7;

const COMMAND_READ: u8 = 0x20;
const COMMAND_WRITE: u8 = 0x30;
const COMMAND_FLUSH: u8 = 0xE7;
const COMMAND_IDENTIFY: u8 = 0xEC;
// PIO normally completes immediately in QEMU, but a heavily loaded host can
// deschedule the device thread while the guest is polling. Keep the operation
// bounded while leaving enough headroom for parallel integration tests.
const MAX_POLLS: usize = 12_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    NoDevice,
    UnsupportedDevice,
    OutOfRange,
    Timeout,
    DeviceFault(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    sectors: u32,
}

impl Device {
    pub const fn sectors(self) -> u32 {
        self.sectors
    }

    pub fn read_sector(self, lba: u32, output: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        if lba >= self.sectors || lba > 0x0FFF_FFFF {
            return Err(StorageError::OutOfRange);
        }
        select_lba(lba)?;
        unsafe { port::outb(STATUS_COMMAND, COMMAND_READ) };
        wait_for_data()?;
        for index in 0..SECTOR_SIZE / 2 {
            let word = unsafe { port::inw(DATA) }.to_le_bytes();
            output[index * 2] = word[0];
            output[index * 2 + 1] = word[1];
        }
        acknowledge_delay();
        Ok(())
    }

    pub fn write_sector(self, lba: u32, input: &[u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        if lba >= self.sectors || lba > 0x0FFF_FFFF {
            return Err(StorageError::OutOfRange);
        }
        select_lba(lba)?;
        unsafe { port::outb(STATUS_COMMAND, COMMAND_WRITE) };
        wait_for_data()?;
        for index in 0..SECTOR_SIZE / 2 {
            unsafe {
                port::outw(
                    DATA,
                    u16::from_le_bytes([input[index * 2], input[index * 2 + 1]]),
                )
            };
        }
        wait_not_busy()?;
        unsafe { port::outb(STATUS_COMMAND, COMMAND_FLUSH) };
        wait_not_busy()?;
        Ok(())
    }
}

/// Probe the primary ATA master. The dedicated ExpOS state image must be
/// attached there; optical media remains on a different IDE unit.
pub fn initialize() -> Result<Device, StorageError> {
    unsafe {
        port::outb(DRIVE, 0xA0);
        port::outb(SECTOR_COUNT, 0);
        port::outb(LBA_LOW, 0);
        port::outb(LBA_MID, 0);
        port::outb(LBA_HIGH, 0);
        port::outb(STATUS_COMMAND, COMMAND_IDENTIFY);
    }
    let initial = unsafe { port::inb(STATUS_COMMAND) };
    if initial == 0 || initial == 0xFF {
        return Err(StorageError::NoDevice);
    }
    wait_not_busy()?;
    let signature_mid = unsafe { port::inb(LBA_MID) };
    let signature_high = unsafe { port::inb(LBA_HIGH) };
    if signature_mid != 0 || signature_high != 0 {
        return Err(StorageError::UnsupportedDevice);
    }
    wait_for_data()?;
    let mut identify = [0_u16; 256];
    for word in &mut identify {
        *word = unsafe { port::inw(DATA) };
    }
    let sectors = (identify[61] as u32) << 16 | identify[60] as u32;
    if sectors == 0 {
        return Err(StorageError::UnsupportedDevice);
    }
    Ok(Device { sectors })
}

fn select_lba(lba: u32) -> Result<(), StorageError> {
    unsafe {
        port::outb(DRIVE, 0xE0 | ((lba >> 24) as u8 & 0x0F));
    }
    acknowledge_delay();
    wait_ready()?;
    unsafe {
        port::outb(SECTOR_COUNT, 1);
        port::outb(LBA_LOW, lba as u8);
        port::outb(LBA_MID, (lba >> 8) as u8);
        port::outb(LBA_HIGH, (lba >> 16) as u8);
    }
    acknowledge_delay();
    Ok(())
}

fn wait_ready() -> Result<(), StorageError> {
    for _ in 0..MAX_POLLS {
        let status = unsafe { port::inb(STATUS_COMMAND) };
        if status & (STATUS_ERROR | STATUS_DEVICE_FAULT) != 0 {
            return Err(StorageError::DeviceFault(unsafe { port::inb(ERROR) }));
        }
        if status & STATUS_BUSY == 0 && status & STATUS_READY != 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(StorageError::Timeout)
}

fn wait_not_busy() -> Result<(), StorageError> {
    for _ in 0..MAX_POLLS {
        let status = unsafe { port::inb(STATUS_COMMAND) };
        if status & STATUS_BUSY == 0 {
            if status & (STATUS_ERROR | STATUS_DEVICE_FAULT) != 0 {
                return Err(StorageError::DeviceFault(unsafe { port::inb(ERROR) }));
            }
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(StorageError::Timeout)
}

fn wait_for_data() -> Result<(), StorageError> {
    for _ in 0..MAX_POLLS {
        let status = unsafe { port::inb(STATUS_COMMAND) };
        if status & (STATUS_ERROR | STATUS_DEVICE_FAULT) != 0 {
            return Err(StorageError::DeviceFault(unsafe { port::inb(ERROR) }));
        }
        if status & STATUS_BUSY == 0 && status & STATUS_DATA_REQUEST != 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(StorageError::Timeout)
}

fn acknowledge_delay() {
    // Four alternate-status reads provide the ATA-required 400 ns settling
    // delay without acknowledging a pending interrupt.
    for _ in 0..4 {
        let _ = unsafe { port::inb(ALT_STATUS) };
    }
}
