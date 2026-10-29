//! ExpOS block transport for NVMe, AHCI SATA and legacy ATA-PIO state disks.
//!
//! This driver deliberately exposes sectors rather than a filesystem.  The
//! persistent state layer above it owns versioning, checksums, and atomic slot
//! selection.  Polls are bounded so absent or faulty hardware cannot hang boot.

use crate::{pci, port};

mod ahci;
mod nvme;
mod virtio_block;

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
pub const MAX_STORAGE_DEVICES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    NoDevice,
    UnsupportedDevice,
    OutOfRange,
    Timeout,
    DeviceFault(u8),
    ControllerFault(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceIdentity {
    backend: &'static str,
    controller: Option<pci::Location>,
    sectors: u32,
    model: [u8; 40],
    model_len: u8,
    serial: [u8; 20],
    serial_len: u8,
    removable: bool,
}

#[cfg_attr(not(feature = "genesis-installer"), allow(dead_code))]
impl DeviceIdentity {
    pub(super) const fn new(
        backend: &'static str,
        controller: Option<pci::Location>,
        sectors: u32,
        model: ([u8; 40], u8),
        serial: ([u8; 20], u8),
        removable: bool,
    ) -> Self {
        Self {
            backend,
            controller,
            sectors,
            model: model.0,
            model_len: model.1,
            serial: serial.0,
            serial_len: serial.1,
            removable,
        }
    }

    pub const fn backend(self) -> &'static str {
        self.backend
    }

    pub const fn sectors(self) -> u32 {
        self.sectors
    }

    pub const fn controller(self) -> Option<pci::Location> {
        self.controller
    }

    pub const fn removable(self) -> bool {
        self.removable
    }

    pub fn model(&self) -> &str {
        core::str::from_utf8(&self.model[..self.model_len as usize]).unwrap_or("unknown")
    }

    pub fn serial(&self) -> &str {
        core::str::from_utf8(&self.serial[..self.serial_len as usize]).unwrap_or("unknown")
    }

    pub fn trustworthy(self) -> bool {
        self.model_len != 0
            && self.serial_len != 0
            && self.sectors != 0
            && self.model[..self.model_len as usize]
                .iter()
                .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
            && self.serial[..self.serial_len as usize]
                .iter()
                .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
    }

    /// Stable within a physical device identity. This is a destructive-action
    /// confirmation token, not a cryptographic authentication primitive.
    pub fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in self
            .backend
            .as_bytes()
            .iter()
            .chain(self.sectors.to_le_bytes().iter())
            .chain(self.model[..self.model_len as usize].iter())
            .chain(self.serial[..self.serial_len as usize].iter())
        {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        if let Some(location) = self.controller {
            for byte in [location.bus, location.slot, location.function] {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }
        hash ^= self.removable as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
        hash
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AtaDevice {
    sectors: u32,
    drive: u8,
    identity: DeviceIdentity,
}

impl AtaDevice {
    pub const fn sectors(self) -> u32 {
        self.sectors
    }

    #[cfg_attr(not(feature = "genesis-installer"), allow(dead_code))]
    pub const fn identity(self) -> DeviceIdentity {
        self.identity
    }

    pub fn read_sector(self, lba: u32, output: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        if lba >= self.sectors || lba > 0x0FFF_FFFF {
            return Err(StorageError::OutOfRange);
        }
        select_lba(self.drive, lba)?;
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
        self.write_sector_unflushed(lba, input)?;
        self.flush()
    }

    /// Write one sector without issuing a cache flush. Bulk constructors such
    /// as Genesis use this for every sector, then flush once at the atomic
    /// installation boundary. ExpFS continues to use `write_sector` and keeps
    /// its per-sector durability behavior.
    pub(crate) fn write_sector_unflushed(
        self,
        lba: u32,
        input: &[u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        if lba >= self.sectors || lba > 0x0FFF_FFFF {
            return Err(StorageError::OutOfRange);
        }
        select_lba(self.drive, lba)?;
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
        Ok(())
    }

    pub(crate) fn flush(self) -> Result<(), StorageError> {
        unsafe { port::outb(STATUS_COMMAND, COMMAND_FLUSH) };
        wait_not_busy()?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    Ata(AtaDevice),
    Ahci(ahci::Device),
    Nvme(nvme::Device),
    Virtio(virtio_block::Device),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    backend: Backend,
}

/// Fixed-capacity, allocation-free storage inventory used by Genesis.  Device
/// handles remain fully usable after enumeration; selecting an entry does not
/// re-probe or silently fall back to another disk.
pub struct DeviceSet {
    entries: [Option<Device>; MAX_STORAGE_DEVICES],
    count: usize,
}

impl DeviceSet {
    const fn new() -> Self {
        Self {
            entries: [None; MAX_STORAGE_DEVICES],
            count: 0,
        }
    }

    fn push(&mut self, device: Device) {
        if self.count < self.entries.len() {
            self.entries[self.count] = Some(device);
            self.count += 1;
        }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn get(&self, index: usize) -> Option<Device> {
        self.entries.get(index).copied().flatten()
    }
}

impl Device {
    pub const fn sectors(self) -> u32 {
        match self.backend {
            Backend::Ata(device) => device.sectors(),
            Backend::Ahci(device) => device.sectors(),
            Backend::Nvme(device) => device.sectors(),
            Backend::Virtio(device) => device.sectors(),
        }
    }

    pub const fn backend(self) -> &'static str {
        match self.backend {
            Backend::Ata(_) => "ata-pio",
            Backend::Ahci(_) => "ahci",
            Backend::Nvme(_) => "nvme",
            Backend::Virtio(_) => "virtio-block",
        }
    }

    #[cfg_attr(not(feature = "genesis-installer"), allow(dead_code))]
    pub const fn identity(self) -> DeviceIdentity {
        match self.backend {
            Backend::Ata(device) => device.identity(),
            Backend::Ahci(device) => device.identity(),
            Backend::Nvme(device) => device.identity(),
            Backend::Virtio(device) => device.identity(),
        }
    }

    pub fn read_sector(self, lba: u32, output: &mut [u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        match self.backend {
            Backend::Ata(device) => device.read_sector(lba, output),
            Backend::Ahci(device) => device.read_sector(lba, output),
            Backend::Nvme(device) => device.read_sector(lba, output),
            Backend::Virtio(device) => device.read_sector(lba, output),
        }
    }

    pub fn write_sector(self, lba: u32, input: &[u8; SECTOR_SIZE]) -> Result<(), StorageError> {
        match self.backend {
            Backend::Ata(device) => device.write_sector(lba, input),
            Backend::Ahci(device) => device.write_sector(lba, input),
            Backend::Nvme(device) => device.write_sector(lba, input),
            Backend::Virtio(device) => device.write_sector(lba, input),
        }
    }

    pub(crate) fn write_sector_unflushed(
        self,
        lba: u32,
        input: &[u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        match self.backend {
            Backend::Ata(device) => device.write_sector_unflushed(lba, input),
            Backend::Ahci(device) => device.write_sector_unflushed(lba, input),
            Backend::Nvme(device) => device.write_sector_unflushed(lba, input),
            Backend::Virtio(device) => device.write_sector_unflushed(lba, input),
        }
    }

    pub(crate) fn flush(self) -> Result<(), StorageError> {
        match self.backend {
            Backend::Ata(device) => device.flush(),
            Backend::Ahci(device) => device.flush(),
            Backend::Nvme(device) => device.flush(),
            Backend::Virtio(device) => device.flush(),
        }
    }
}

/// Enumerate every storage target that can be represented by the current
/// bounded drivers.  PCI controllers are listed before the two legacy primary
/// ATA positions so the displayed order is deterministic across boots.
pub fn enumerate() -> Result<DeviceSet, StorageError> {
    let mut devices = DeviceSet::new();
    match nvme::initialize() {
        Ok(device) => devices.push(Device {
            backend: Backend::Nvme(device),
        }),
        Err(StorageError::NoDevice) => {}
        Err(error) => crate::slog!(
            "EXPOS_STORAGE_PROBE_FAILED backend=nvme error={:?}\r\n",
            error
        ),
    }
    match ahci::initialize() {
        Ok(device) => devices.push(Device {
            backend: Backend::Ahci(device),
        }),
        Err(StorageError::NoDevice) => {}
        Err(error) => crate::slog!(
            "EXPOS_STORAGE_PROBE_FAILED backend=ahci error={:?}\r\n",
            error
        ),
    }
    match virtio_block::initialize() {
        Ok(device) => devices.push(Device {
            backend: Backend::Virtio(device),
        }),
        Err(StorageError::NoDevice) => {}
        Err(error) => crate::slog!(
            "EXPOS_STORAGE_PROBE_FAILED backend=virtio-block error={:?}\r\n",
            error
        ),
    }
    for drive in 0_u8..=1 {
        match initialize_ata(drive) {
            Ok(device) => devices.push(Device {
                backend: Backend::Ata(device),
            }),
            Err(StorageError::NoDevice | StorageError::UnsupportedDevice) => {}
            Err(error) => crate::slog!(
                "EXPOS_STORAGE_PROBE_FAILED backend=ata-pio position={} error={:?}\r\n",
                if drive == 0 {
                    "primary-master"
                } else {
                    "primary-slave"
                },
                error
            ),
        }
    }
    if devices.is_empty() {
        return Err(StorageError::NoDevice);
    }
    crate::slog!(
        "EXPOS_STORAGE_ENUMERATED devices={} bounded_max={}\r\n",
        devices.len(),
        MAX_STORAGE_DEVICES
    );
    Ok(devices)
}

pub fn handle_legacy_interrupt(irq: u8) {
    virtio_block::handle_legacy_interrupt(irq);
}

pub fn handle_message_interrupt() {}

/// Probe modern PCI storage first, then fall back to legacy ATA. Runtime boot
/// keeps the historical first-device policy; destructive Genesis installation
/// always uses [`enumerate`] and requires an explicit selection.
pub fn initialize() -> Result<Device, StorageError> {
    let result = enumerate()?.get(0).ok_or(StorageError::NoDevice)?;
    crate::slog!(
        "EXPOS_STORAGE_READY backend={} sectors={} sector_bytes={}\r\n",
        result.backend(),
        result.sectors(),
        SECTOR_SIZE
    );
    Ok(result)
}

fn initialize_ata(drive: u8) -> Result<AtaDevice, StorageError> {
    let selector = if drive == 0 { 0xA0 } else { 0xB0 };
    unsafe {
        port::outb(DRIVE, selector);
        acknowledge_delay();
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
    let (serial, serial_len) = ata_identity_text::<20>(&identify[10..20]);
    let (model, model_len) = ata_identity_text::<40>(&identify[27..47]);
    Ok(AtaDevice {
        sectors,
        drive,
        identity: DeviceIdentity::new(
            "ata-pio",
            None,
            sectors,
            (model, model_len),
            (serial, serial_len),
            identify[0] & (1 << 7) != 0,
        ),
    })
}

pub(super) fn ata_identity_text<const N: usize>(words: &[u16]) -> ([u8; N], u8) {
    let mut output = [0_u8; N];
    let mut length = 0_usize;
    for word in words.iter().take(N / 2) {
        let bytes = word.to_be_bytes();
        output[length] = bytes[0];
        output[length + 1] = bytes[1];
        length += 2;
    }
    while length != 0 && matches!(output[length - 1], 0 | b' ') {
        length -= 1;
    }
    let leading = output[..length]
        .iter()
        .position(|byte| !matches!(*byte, 0 | b' '))
        .unwrap_or(length);
    if leading != 0 && leading != length {
        output.copy_within(leading..length, 0);
        length -= leading;
        output[length..].fill(0);
    }
    (output, length as u8)
}

pub(super) fn nvme_identity_text<const N: usize>(bytes: &[u8]) -> ([u8; N], u8) {
    let mut output = [0_u8; N];
    let amount = N.min(bytes.len());
    output[..amount].copy_from_slice(&bytes[..amount]);
    let mut length = amount;
    while length != 0 && matches!(output[length - 1], 0 | b' ') {
        length -= 1;
    }
    (output, length as u8)
}

fn select_lba(drive: u8, lba: u32) -> Result<(), StorageError> {
    unsafe {
        port::outb(
            DRIVE,
            0xE0 | ((drive & 1) << 4) | ((lba >> 24) as u8 & 0x0F),
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ata_identity_text_swaps_words_and_trims_padding() {
        let words = [
            u16::from_be_bytes(*b"EX"),
            u16::from_be_bytes(*b"PO"),
            u16::from_be_bytes(*b"S "),
        ];
        let (text, length) = ata_identity_text::<8>(&words);
        assert_eq!(&text[..length as usize], b"EXPOS");
    }

    #[test]
    fn disk_fingerprint_binds_backend_geometry_model_serial_and_controller() {
        let mut model = [0_u8; 40];
        model[..10].copy_from_slice(b"Test Drive");
        let mut serial = [0_u8; 20];
        serial[..6].copy_from_slice(b"ABC123");
        let identity = DeviceIdentity::new(
            "nvme",
            Some(pci::Location {
                bus: 2,
                slot: 0,
                function: 0,
            }),
            1_000_000,
            (model, 10),
            (serial, 6),
            false,
        );
        assert!(identity.trustworthy());
        assert_eq!(identity.fingerprint(), identity.fingerprint());
        assert_ne!(
            identity.fingerprint(),
            DeviceIdentity::new("nvme", None, 1_000_000, (model, 10), (serial, 6), false)
                .fingerprint()
        );
    }
}
