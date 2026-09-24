//! Genesis installation manifest and the first bootable-disk constructor.
//!
//! Genesis deliberately writes a standards-based GPT + FAT32 EFI System
//! Partition, while ExpFS remains the CFC-owned transactional database in the
//! reserved pre-partition area. The initial vertical slice supports an
//! explicitly unencrypted Architect installation. Basic stays gated until its
//! mandatory Argon2id + AEAD storage protection is implemented.

#![cfg_attr(not(any(test, feature = "genesis-installer")), allow(dead_code))]

use crate::{session, storage};
use expos_core::{CfcFin, Fin, Text};

const MANIFEST_LBA: u32 = 40;
const MANIFEST_MAGIC: [u8; 8] = *b"EXGEN001";
const MANIFEST_VERSION: u16 = 1;
const EFI_PARTITION_START: u32 = 2048;
const GPT_ENTRY_SECTORS: u32 = 32;
const FAT_RESERVED_SECTORS: u32 = 32;
const FAT_COUNT: u32 = 2;
const FAT32_MIN_CLUSTERS: u32 = 65_525;
const DIRECTORY_CLUSTERS: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenesisError {
    DiskTooSmall,
    PayloadTooLarge,
    Storage(storage::StorageError),
}

impl From<storage::StorageError> for GenesisError {
    fn from(error: storage::StorageError) -> Self {
        Self::Storage(error)
    }
}

#[derive(Clone, Copy)]
pub struct GenesisConfig {
    pub cfc_fin: CfcFin,
    pub cfc_name: Text,
    pub primary_fin: Fin,
    pub primary_name: Text,
    pub operator: session::StoredGenesisAccount,
}

trait BlockDevice {
    fn sectors(&self) -> u32;
    fn write(&mut self, lba: u32, input: &[u8; storage::SECTOR_SIZE]) -> Result<(), GenesisError>;
    fn flush(&mut self) -> Result<(), GenesisError>;
}

impl BlockDevice for storage::Device {
    fn sectors(&self) -> u32 {
        (*self).sectors()
    }

    fn write(&mut self, lba: u32, input: &[u8; storage::SECTOR_SIZE]) -> Result<(), GenesisError> {
        (*self)
            .write_sector_unflushed(lba, input)
            .map_err(Into::into)
    }

    fn flush(&mut self) -> Result<(), GenesisError> {
        (*self).flush().map_err(Into::into)
    }
}

/// Load installation identity before the Form-native bootstrap runs.
pub fn load() -> Option<GenesisConfig> {
    let device = storage::initialize().ok()?;
    let mut sector = [0_u8; storage::SECTOR_SIZE];
    device.read_sector(MANIFEST_LBA, &mut sector).ok()?;
    decode_manifest(&sector)
}

#[cfg(feature = "genesis-installer")]
const RUNTIME_UEFI_APP: &[u8] = include_bytes!("../../build/genesis/runtime-BOOTX64.EFI");

#[cfg(feature = "genesis-installer")]
pub fn run() -> ! {
    use crate::{crypto, input::Input, port, println, slog};

    println!();
    println!("ExpOS Genesis Engine");
    println!("Construct a Central Inflation Fabric");
    println!("----------------------------------------");
    println!("1. Basic      (requires encrypted storage; not available yet)");
    println!("2. Architect  (whole disk, unencrypted preview)");
    let mut input = Input::new();
    loop {
        let mode = read_line(&mut input, "Installation mode [1/2]: ", false);
        if line(&mode) == "1" {
            println!(
                "Basic is gated: its mandatory Argon2id + AEAD protection is not implemented."
            );
            println!("Genesis will not silently create an insecure Basic installation.");
            continue;
        }
        if line(&mode) == "2" {
            break;
        }
        println!("Choose 1 or 2.");
    }

    let mut device = match storage::initialize() {
        Ok(device) => device,
        Err(error) => fatal(GenesisError::Storage(error)),
    };
    println!(
        "Target: ATA primary master, {} MiB ({} sectors)",
        device.sectors() / 2048,
        device.sectors()
    );
    println!("WARNING: every existing byte on this target will be replaced.");
    if line(&read_line(&mut input, "Type ERASE to continue: ", false)) != "ERASE" {
        println!("Installation cancelled; no disk writes were made.");
        port::shutdown();
    }

    let cfc_name = prompt_text(&mut input, "CFC name: ");
    let primary_name = prompt_text(&mut input, "Primary Dimension name: ");
    let operator = loop {
        let mut password = read_line(&mut input, "Operator password (8-23 characters): ", true);
        match session::genesis_operator(line_bytes(&password)) {
            Ok(record) => {
                crypto::wipe(&mut password);
                break record;
            }
            Err(error) => {
                crypto::wipe(&mut password);
                println!("Invalid password: {}", error.message());
            }
        }
    };

    let entropy = unsafe { core::arch::x86_64::_rdtsc() } as u128;
    let cfc_fin = CfcFin::from_u128(0x4745_4e45_5349_5300_0000_0000_0000_0000 | entropy);
    let primary_fin =
        Fin::from_u128(0x4449_4d45_4e53_494f_0000_0000_0000_0000 | entropy.rotate_left(41));
    let config = GenesisConfig {
        cfc_fin,
        cfc_name,
        primary_fin,
        primary_name,
        operator: session::StoredGenesisAccount(operator),
    };

    println!("Creating GPT and EFI System Partition...");
    if let Err(error) = install(&mut device, RUNTIME_UEFI_APP, config) {
        fatal(error);
    }
    slog!(
        "EXPOS_GENESIS_INSTALLED cfc={} dimension={} uefi_bytes={}\r\n",
        cfc_fin,
        primary_fin,
        RUNTIME_UEFI_APP.len()
    );
    println!("[ok] CFC {}", cfc_fin);
    println!("[ok] Primary Dimension {}", primary_fin);
    println!("[ok] ExpFS reserved and Operator seed written");
    println!("[ok] EFI/BOOT/BOOTX64.EFI installed");
    println!();
    println!("Installation complete. Remove the USB, then press Enter to reboot.");
    let _ = read_line(&mut input, "", false);
    port::reboot();
}

#[cfg(feature = "genesis-installer")]
fn fatal(error: GenesisError) -> ! {
    crate::println!("Genesis failed: {:?}", error);
    crate::slog!("EXPOS_GENESIS_FAILED error={:?}\r\n", error);
    crate::port::shutdown()
}

#[cfg(feature = "genesis-installer")]
fn prompt_text(input: &mut crate::input::Input, prompt: &str) -> Text {
    loop {
        let value = read_line(input, prompt, false);
        if let Ok(text) = Text::new(line(&value)) {
            return text;
        }
        crate::println!("Use 1-32 ASCII characters.");
    }
}

#[cfg(feature = "genesis-installer")]
fn read_line(input: &mut crate::input::Input, prompt: &str, secret: bool) -> [u8; 33] {
    crate::print!("{}", prompt);
    let mut output = [0_u8; 33];
    let mut len = 0;
    loop {
        let Some(key) = input.poll() else {
            core::hint::spin_loop();
            continue;
        };
        match key {
            b'\n' => {
                crate::println!();
                output[32] = len as u8;
                return output;
            }
            0x08 if len != 0 => {
                len -= 1;
                output[len] = 0;
                crate::print!("\x08 \x08");
            }
            byte @ 0x20..=0x7e if len < 32 => {
                output[len] = byte;
                len += 1;
                crate::print!("{}", if secret { '*' } else { byte as char });
            }
            _ => {}
        }
    }
}

#[cfg(feature = "genesis-installer")]
fn line(value: &[u8; 33]) -> &str {
    core::str::from_utf8(line_bytes(value)).unwrap_or("")
}

#[cfg(feature = "genesis-installer")]
fn line_bytes(value: &[u8; 33]) -> &[u8] {
    &value[..value[32] as usize]
}

fn install<D: BlockDevice>(
    device: &mut D,
    boot_app: &[u8],
    config: GenesisConfig,
) -> Result<(), GenesisError> {
    let geometry = FatGeometry::new(device.sectors(), boot_app.len())?;
    write_gpt(device, geometry)?;
    write_fat32(device, geometry, boot_app)?;
    let sector = encode_manifest(config);
    device.write(MANIFEST_LBA, &sector)?;
    device.flush()?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct FatGeometry {
    total_sectors: u32,
    partition_start: u32,
    partition_sectors: u32,
    fat_sectors: u32,
    clusters: u32,
    file_clusters: u32,
}

impl FatGeometry {
    fn new(total_sectors: u32, file_bytes: usize) -> Result<Self, GenesisError> {
        if total_sectors <= EFI_PARTITION_START + 34 {
            return Err(GenesisError::DiskTooSmall);
        }
        let partition_sectors = total_sectors - EFI_PARTITION_START - 33;
        let mut fat_sectors = 1;
        let clusters = loop {
            let data = partition_sectors
                .checked_sub(FAT_RESERVED_SECTORS + FAT_COUNT * fat_sectors)
                .ok_or(GenesisError::DiskTooSmall)?;
            let required = ((data + 2) * 4 + 511) / 512;
            if required == fat_sectors {
                break data;
            }
            fat_sectors = required;
        };
        if clusters < FAT32_MIN_CLUSTERS {
            return Err(GenesisError::DiskTooSmall);
        }
        let file_clusters = ((file_bytes as u64 + 511) / 512) as u32;
        if file_clusters + DIRECTORY_CLUSTERS > clusters {
            return Err(GenesisError::PayloadTooLarge);
        }
        Ok(Self {
            total_sectors,
            partition_start: EFI_PARTITION_START,
            partition_sectors,
            fat_sectors,
            clusters,
            file_clusters,
        })
    }

    fn data_start(self) -> u32 {
        self.partition_start + FAT_RESERVED_SECTORS + FAT_COUNT * self.fat_sectors
    }

    fn cluster_lba(self, cluster: u32) -> u32 {
        self.data_start() + cluster - 2
    }
}

fn write_gpt<D: BlockDevice>(device: &mut D, geometry: FatGeometry) -> Result<(), GenesisError> {
    let mut sector = [0_u8; 512];
    sector[446] = 0;
    sector[450] = 0xEE;
    put_u32(&mut sector, 454, 1);
    put_u32(&mut sector, 458, geometry.total_sectors.saturating_sub(1));
    sector[510] = 0x55;
    sector[511] = 0xAA;
    device.write(0, &sector)?;

    let mut entries = [0_u8; 512];
    entries[..16].copy_from_slice(&[
        0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9,
        0x3B,
    ]);
    entries[16..32].copy_from_slice(b"ExpOSGenesisESP1");
    put_u64(&mut entries, 32, geometry.partition_start as u64);
    put_u64(
        &mut entries,
        40,
        (geometry.partition_start + geometry.partition_sectors - 1) as u64,
    );
    for (index, unit) in "ExpOS EFI System".encode_utf16().enumerate() {
        entries[56 + index * 2..58 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    // Reserve the pre-ESP region as an explicit ExpOS partition so generic
    // partitioning tools do not treat the Genesis manifest and ExpFS journal
    // as disposable free space.
    entries[128..144].copy_from_slice(b"ExpOSCfcDatabase");
    entries[144..160].copy_from_slice(b"ExpOSGenesisCFC1");
    put_u64(&mut entries, 160, 34);
    put_u64(&mut entries, 168, (EFI_PARTITION_START - 1) as u64);
    for (index, unit) in "ExpOS CFC Database".encode_utf16().enumerate() {
        entries[184 + index * 2..186 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    let mut entries_crc = !0_u32;
    entries_crc = crc32_feed(entries_crc, &entries);
    let zero = [0_u8; 512];
    for _ in 1..GPT_ENTRY_SECTORS {
        entries_crc = crc32_feed(entries_crc, &zero);
    }
    entries_crc = !entries_crc;

    for index in 0..GPT_ENTRY_SECTORS {
        device.write(2 + index, if index == 0 { &entries } else { &zero })?;
        let backup_lba = geometry.total_sectors - 33 + index;
        device.write(backup_lba, if index == 0 { &entries } else { &zero })?;
    }
    let primary = gpt_header(
        1,
        geometry.total_sectors as u64 - 1,
        2,
        geometry,
        entries_crc,
    );
    let backup = gpt_header(
        geometry.total_sectors as u64 - 1,
        1,
        geometry.total_sectors as u64 - 33,
        geometry,
        entries_crc,
    );
    device.write(1, &primary)?;
    device.write(geometry.total_sectors - 1, &backup)?;
    Ok(())
}

fn gpt_header(
    current: u64,
    alternate: u64,
    entries_lba: u64,
    geometry: FatGeometry,
    entries_crc: u32,
) -> [u8; 512] {
    let mut sector = [0_u8; 512];
    sector[..8].copy_from_slice(b"EFI PART");
    put_u32(&mut sector, 8, 0x0001_0000);
    put_u32(&mut sector, 12, 92);
    put_u64(&mut sector, 24, current);
    put_u64(&mut sector, 32, alternate);
    put_u64(&mut sector, 40, 34);
    put_u64(&mut sector, 48, geometry.total_sectors as u64 - 34);
    sector[56..72].copy_from_slice(b"ExpOSGenesisDisk");
    put_u64(&mut sector, 72, entries_lba);
    put_u32(&mut sector, 80, 128);
    put_u32(&mut sector, 84, 128);
    put_u32(&mut sector, 88, entries_crc);
    let crc = crc32(&sector[..92]);
    put_u32(&mut sector, 16, crc);
    sector
}

fn write_fat32<D: BlockDevice>(
    device: &mut D,
    geometry: FatGeometry,
    boot_app: &[u8],
) -> Result<(), GenesisError> {
    let mut boot = [0_u8; 512];
    boot[..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    boot[3..11].copy_from_slice(b"EXPOS   ");
    put_u16(&mut boot, 11, 512);
    boot[13] = 1;
    put_u16(&mut boot, 14, FAT_RESERVED_SECTORS as u16);
    boot[16] = FAT_COUNT as u8;
    boot[21] = 0xF8;
    put_u16(&mut boot, 24, 63);
    put_u16(&mut boot, 26, 255);
    put_u32(&mut boot, 28, geometry.partition_start);
    put_u32(&mut boot, 32, geometry.partition_sectors);
    put_u32(&mut boot, 36, geometry.fat_sectors);
    put_u32(&mut boot, 44, 2);
    put_u16(&mut boot, 48, 1);
    put_u16(&mut boot, 50, 6);
    boot[64] = 0x80;
    boot[66] = 0x29;
    put_u32(&mut boot, 67, 0x4558_504f);
    boot[71..82].copy_from_slice(b"EXPOS ESP  ");
    boot[82..90].copy_from_slice(b"FAT32   ");
    boot[510] = 0x55;
    boot[511] = 0xAA;
    device.write(geometry.partition_start, &boot)?;
    device.write(geometry.partition_start + 6, &boot)?;

    let mut fsinfo = [0_u8; 512];
    put_u32(&mut fsinfo, 0, 0x4161_5252);
    put_u32(&mut fsinfo, 484, 0x6141_7272);
    put_u32(
        &mut fsinfo,
        488,
        geometry.clusters - geometry.file_clusters - DIRECTORY_CLUSTERS,
    );
    put_u32(&mut fsinfo, 492, 5 + geometry.file_clusters);
    put_u32(&mut fsinfo, 508, 0xAA55_0000);
    device.write(geometry.partition_start + 1, &fsinfo)?;
    device.write(geometry.partition_start + 7, &fsinfo)?;

    for fat in 0..FAT_COUNT {
        for sector_index in 0..geometry.fat_sectors {
            let mut sector = [0_u8; 512];
            let first_cluster = sector_index * 128;
            for slot in 0..128_u32 {
                let cluster = first_cluster + slot;
                let value = fat_value(cluster, geometry.file_clusters);
                put_u32(&mut sector, slot as usize * 4, value);
            }
            device.write(
                geometry.partition_start
                    + FAT_RESERVED_SECTORS
                    + fat * geometry.fat_sectors
                    + sector_index,
                &sector,
            )?;
        }
    }

    let mut root = [0_u8; 512];
    directory_entry(&mut root, 0, b"EFI        ", 0x10, 3, 0);
    device.write(geometry.cluster_lba(2), &root)?;
    let mut efi = [0_u8; 512];
    directory_entry(&mut efi, 0, b".          ", 0x10, 3, 0);
    directory_entry(&mut efi, 1, b"..         ", 0x10, 2, 0);
    directory_entry(&mut efi, 2, b"BOOT       ", 0x10, 4, 0);
    device.write(geometry.cluster_lba(3), &efi)?;
    let mut boot_dir = [0_u8; 512];
    directory_entry(&mut boot_dir, 0, b".          ", 0x10, 4, 0);
    directory_entry(&mut boot_dir, 1, b"..         ", 0x10, 3, 0);
    directory_entry(
        &mut boot_dir,
        2,
        b"BOOTX64 EFI",
        0x20,
        5,
        boot_app.len() as u32,
    );
    device.write(geometry.cluster_lba(4), &boot_dir)?;
    for (index, chunk) in boot_app.chunks(512).enumerate() {
        let mut sector = [0_u8; 512];
        sector[..chunk.len()].copy_from_slice(chunk);
        device.write(geometry.cluster_lba(5 + index as u32), &sector)?;
    }
    Ok(())
}

fn fat_value(cluster: u32, file_clusters: u32) -> u32 {
    match cluster {
        0 => 0x0FFF_FFF8,
        1 | 2 | 3 | 4 => 0x0FFF_FFFF,
        current if current >= 5 && current < 5 + file_clusters => {
            if current + 1 == 5 + file_clusters {
                0x0FFF_FFFF
            } else {
                current + 1
            }
        }
        _ => 0,
    }
}

fn directory_entry(
    sector: &mut [u8; 512],
    index: usize,
    name: &[u8; 11],
    attributes: u8,
    cluster: u32,
    size: u32,
) {
    let offset = index * 32;
    sector[offset..offset + 11].copy_from_slice(name);
    sector[offset + 11] = attributes;
    put_u16(sector, offset + 20, (cluster >> 16) as u16);
    put_u16(sector, offset + 26, cluster as u16);
    put_u32(sector, offset + 28, size);
}

fn encode_manifest(config: GenesisConfig) -> [u8; 512] {
    let mut sector = [0_u8; 512];
    sector[..8].copy_from_slice(&MANIFEST_MAGIC);
    put_u16(&mut sector, 8, MANIFEST_VERSION);
    sector[10] = 2; // Architect
    sector[11] = 0; // explicitly unencrypted
    sector[16..32].copy_from_slice(&config.cfc_fin.bytes());
    sector[32..48].copy_from_slice(&config.primary_fin.bytes());
    put_text(&mut sector, 48, config.cfc_name);
    put_text(&mut sector, 81, config.primary_name);
    let account = config.operator.0;
    sector[114] = account.name_len;
    sector[115..139].copy_from_slice(&account.name);
    sector[139..155].copy_from_slice(&account.password_salt);
    sector[155..187].copy_from_slice(&account.password_hash);
    put_u32(&mut sector, 187, account.kdf_rounds);
    sector[191] = account.authority;
    sector[192] = u8::from(account.occupied);
    let checksum = crc32(&sector[..508]);
    put_u32(&mut sector, 508, checksum);
    sector
}

fn decode_manifest(sector: &[u8; 512]) -> Option<GenesisConfig> {
    if sector[..8] != MANIFEST_MAGIC
        || u16::from_le_bytes([sector[8], sector[9]]) != MANIFEST_VERSION
        || crc32(&sector[..508]) != get_u32(sector, 508)
    {
        return None;
    }
    let cfc_fin = CfcFin::from_u128(u128::from_be_bytes(sector[16..32].try_into().ok()?));
    let primary_fin = Fin::from_u128(u128::from_be_bytes(sector[32..48].try_into().ok()?));
    if cfc_fin.is_zero() || primary_fin.is_zero() {
        return None;
    }
    let cfc_name = get_text(sector, 48)?;
    let primary_name = get_text(sector, 81)?;
    let mut account = crate::state::StoredAccount::EMPTY;
    account.name_len = sector[114];
    account.name.copy_from_slice(&sector[115..139]);
    account.password_salt.copy_from_slice(&sector[139..155]);
    account.password_hash.copy_from_slice(&sector[155..187]);
    account.kdf_rounds = get_u32(sector, 187);
    account.authority = sector[191];
    account.occupied = sector[192] == 1;
    Some(GenesisConfig {
        cfc_fin,
        cfc_name,
        primary_fin,
        primary_name,
        operator: session::StoredGenesisAccount(account),
    })
}

fn put_text(sector: &mut [u8; 512], offset: usize, text: Text) {
    let bytes = text.as_str().as_bytes();
    sector[offset] = bytes.len() as u8;
    sector[offset + 1..offset + 1 + bytes.len()].copy_from_slice(bytes);
}

fn get_text(sector: &[u8; 512], offset: usize) -> Option<Text> {
    let len = sector[offset] as usize;
    if len == 0 || len > 32 {
        return None;
    }
    Text::new(core::str::from_utf8(&sector[offset + 1..offset + 1 + len]).ok()?).ok()
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(input[offset..offset + 4].try_into().unwrap())
}

fn crc32(input: &[u8]) -> u32 {
    !crc32_feed(!0, input)
}

fn crc32_feed(mut crc: u32, input: &[u8]) -> u32 {
    for byte in input {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemoryDisk {
        sectors: u32,
        bytes: Vec<u8>,
    }

    impl MemoryDisk {
        fn new(sectors: u32) -> Self {
            Self {
                sectors,
                bytes: vec![0; sectors as usize * 512],
            }
        }
    }

    impl BlockDevice for MemoryDisk {
        fn sectors(&self) -> u32 {
            self.sectors
        }

        fn write(&mut self, lba: u32, input: &[u8; 512]) -> Result<(), GenesisError> {
            let start = lba as usize * 512;
            self.bytes[start..start + 512].copy_from_slice(input);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), GenesisError> {
            Ok(())
        }
    }

    fn config() -> GenesisConfig {
        GenesisConfig {
            cfc_fin: CfcFin::from_u128(1),
            cfc_name: Text::new("Test CFC").unwrap(),
            primary_fin: Fin::from_u128(2),
            primary_name: Text::new("Primary").unwrap(),
            operator: session::StoredGenesisAccount(
                session::genesis_operator(b"correct-horse").unwrap(),
            ),
        }
    }

    #[test]
    fn manifest_round_trip_keeps_identity_and_hashed_operator() {
        let original = config();
        let decoded = decode_manifest(&encode_manifest(original)).unwrap();
        assert_eq!(decoded.cfc_fin, original.cfc_fin);
        assert_eq!(decoded.cfc_name, original.cfc_name);
        assert_eq!(decoded.primary_fin, original.primary_fin);
        assert_eq!(decoded.primary_name, original.primary_name);
        assert_eq!(decoded.operator.0.name_len, 8);
        assert_ne!(decoded.operator.0.password_hash, [0; 32]);
    }

    #[test]
    fn installer_writes_gpt_fat_fallback_path_and_manifest() {
        let mut disk = MemoryDisk::new(72 * 2048);
        let payload = b"MZ ExpOS UEFI runtime";
        install(&mut disk, payload, config()).unwrap();
        assert_eq!(&disk.bytes[512..520], b"EFI PART");
        assert_eq!(
            &disk.bytes[EFI_PARTITION_START as usize * 512 + 82..][..8],
            b"FAT32   "
        );
        let geometry = FatGeometry::new(disk.sectors, payload.len()).unwrap();
        let boot_dir = geometry.cluster_lba(4) as usize * 512;
        assert_eq!(&disk.bytes[boot_dir + 64..boot_dir + 75], b"BOOTX64 EFI");
        let app = geometry.cluster_lba(5) as usize * 512;
        assert_eq!(&disk.bytes[app..app + payload.len()], payload);
        let manifest = MANIFEST_LBA as usize * 512;
        assert_eq!(&disk.bytes[manifest..manifest + 8], &MANIFEST_MAGIC);
    }

    #[test]
    fn fat32_rejects_tiny_targets() {
        assert_eq!(
            FatGeometry::new(8192, 1024).unwrap_err(),
            GenesisError::DiskTooSmall
        );
    }
}
