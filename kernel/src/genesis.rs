//! Genesis installation manifest and the first bootable-disk constructor.
//!
//! Genesis deliberately writes a standards-based GPT + FAT32 EFI System
//! Partition, while ExpFS remains the CFC-owned transactional database in the
//! reserved pre-partition area. Basic installations use a random per-CFC
//! storage key, Argon2id key wrapping and XChaCha20-Poly1305 authenticated
//! ExpFS snapshots; Architect retains an explicit unencrypted option.

#![cfg_attr(not(any(test, feature = "genesis-installer")), allow(dead_code))]

use crate::{session, storage};
use expos_core::{CfcFin, Fin, Text};

const MANIFEST_LBA: u32 = 40;
const MANIFEST_BACKUP_LBA: u32 = 41;
const MANIFEST_MAGIC: [u8; 8] = *b"EXGEN001";
const MANIFEST_VERSION: u16 = 2;
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
    VerificationFailed,
    Storage(storage::StorageError),
}

impl From<storage::StorageError> for GenesisError {
    fn from(error: storage::StorageError) -> Self {
        Self::Storage(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenesisConfig {
    pub cfc_fin: CfcFin,
    pub cfc_name: Text,
    pub primary_fin: Fin,
    pub primary_name: Text,
    pub operator: session::StoredGenesisAccount,
    pub storage_key: Option<[u8; crate::crypto::STORAGE_KEY_LEN]>,
    storage_envelope: Option<StorageEnvelope>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenesisLoadError {
    ManifestDamaged,
}

impl GenesisLoadError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::ManifestDamaged => "Genesis manifest copies are damaged or disagree",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManifestSelection {
    None,
    Primary(GenesisConfig),
    PrimaryDegraded(GenesisConfig),
    Backup(GenesisConfig),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StorageEnvelope {
    salt: [u8; crate::crypto::STORAGE_SALT_LEN],
    nonce: [u8; crate::crypto::STORAGE_NONCE_LEN],
    wrapped_key: [u8; crate::crypto::STORAGE_KEY_LEN],
    tag: [u8; crate::crypto::STORAGE_TAG_LEN],
}

trait BlockDevice {
    fn sectors(&self) -> u32;
    fn read(&self, lba: u32, output: &mut [u8; storage::SECTOR_SIZE]) -> Result<(), GenesisError>;
    fn write(&mut self, lba: u32, input: &[u8; storage::SECTOR_SIZE]) -> Result<(), GenesisError>;
    fn flush(&mut self) -> Result<(), GenesisError>;
}

impl BlockDevice for storage::Device {
    fn sectors(&self) -> u32 {
        (*self).sectors()
    }

    fn read(&self, lba: u32, output: &mut [u8; storage::SECTOR_SIZE]) -> Result<(), GenesisError> {
        (*self).read_sector(lba, output).map_err(Into::into)
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
pub fn load(input: &mut crate::input::Input) -> Result<Option<GenesisConfig>, GenesisLoadError> {
    let device = match storage::initialize() {
        Ok(device) => device,
        Err(_) => return Ok(None),
    };
    let mut primary = [0_u8; storage::SECTOR_SIZE];
    let mut backup = [0_u8; storage::SECTOR_SIZE];
    device
        .read_sector(MANIFEST_LBA, &mut primary)
        .map_err(|_| GenesisLoadError::ManifestDamaged)?;
    device
        .read_sector(MANIFEST_BACKUP_LBA, &mut backup)
        .map_err(|_| GenesisLoadError::ManifestDamaged)?;
    let mut config = match select_manifest_copies(&primary, &backup)? {
        ManifestSelection::None => return Ok(None),
        ManifestSelection::Primary(config) => config,
        ManifestSelection::PrimaryDegraded(config) => {
            if !sector_blank(&backup) {
                crate::println!("[warn] Genesis backup manifest is damaged");
            }
            crate::slog!("EXPOS_GENESIS_MANIFEST_DEGRADED source=primary copies=1\r\n");
            config
        }
        ManifestSelection::Backup(config) => {
            crate::println!("[warn] Genesis manifest recovered from its backup copy");
            crate::slog!("EXPOS_GENESIS_MANIFEST_RECOVERED source=backup\r\n");
            config
        }
    };
    let Some(envelope) = config.storage_envelope else {
        return Ok(Some(config));
    };
    crate::println!("Encrypted CFC storage detected: {}", config.cfc_name);
    loop {
        let mut password = read_line(input, "Unlock password: ", true);
        let mut kek = match crate::crypto::storage_kek(line_bytes(&password), &envelope.salt) {
            Ok(key) => key,
            Err(()) => {
                crate::crypto::wipe(&mut password);
                crate::println!("Storage-key derivation failed.");
                continue;
            }
        };
        let mut storage_key = envelope.wrapped_key;
        let aad = storage_envelope_aad(config.cfc_fin, config.primary_fin);
        let opened = crate::crypto::open_storage(
            &kek,
            &envelope.nonce,
            &aad,
            &mut storage_key,
            &envelope.tag,
        )
        .is_ok();
        crate::crypto::wipe(&mut kek);
        crate::crypto::wipe(&mut password);
        if opened {
            config.storage_key = Some(storage_key);
            crate::slog!(
                "EXPOS_CFC_UNLOCKED cfc={} suite=xchacha20poly1305\r\n",
                config.cfc_fin
            );
            return Ok(Some(config));
        }
        crate::crypto::wipe(&mut storage_key);
        crate::println!("Wrong password or damaged CFC key envelope.");
    }
}

fn sector_blank(sector: &[u8; storage::SECTOR_SIZE]) -> bool {
    sector.iter().all(|byte| *byte == 0)
}

fn select_manifest_copies(
    primary: &[u8; storage::SECTOR_SIZE],
    backup: &[u8; storage::SECTOR_SIZE],
) -> Result<ManifestSelection, GenesisLoadError> {
    match (decode_manifest(primary), decode_manifest(backup)) {
        (Some(primary_config), Some(backup_config)) if primary_config == backup_config => {
            Ok(ManifestSelection::Primary(primary_config))
        }
        (Some(_), Some(_)) => Err(GenesisLoadError::ManifestDamaged),
        (Some(config), None) => Ok(ManifestSelection::PrimaryDegraded(config)),
        (None, Some(config)) => Ok(ManifestSelection::Backup(config)),
        (None, None) if sector_blank(primary) && sector_blank(backup) => {
            Ok(ManifestSelection::None)
        }
        (None, None) => Err(GenesisLoadError::ManifestDamaged),
    }
}

#[cfg(feature = "genesis-installer")]
const RUNTIME_UEFI_APP: &[u8] = include_bytes!("../../build/genesis/runtime-BOOTX64.EFI");

#[cfg(feature = "genesis-installer")]
pub fn run() -> ! {
    use crate::{crypto, input::Input, port, println, slog};

    let graphical = crate::graphics_console::enable_genesis();
    slog!(
        "EXPOS_GENESIS_DISPLAY visible={} backend={}\r\n",
        graphical,
        if graphical { "bochs-vbe" } else { "serial" }
    );
    println!();
    println!("ExpOS Genesis Engine 2");
    println!("Construct a verified Central Inflation Fabric");
    println!("------------------------------------------------");
    println!("1. Basic      (encrypted CFC storage)");
    println!("2. Architect  (whole disk, unencrypted preview)");
    let mut input = Input::new();
    let basic = loop {
        let mode = read_line(&mut input, "Installation mode [1/2]: ", false);
        if line(&mode) == "1" {
            break true;
        }
        if line(&mode) == "2" {
            break false;
        }
        println!("Choose 1 or 2.");
    };

    let mut device = match storage::initialize() {
        Ok(device) => device,
        Err(error) => fatal(GenesisError::Storage(error)),
    };
    println!(
        "Target: ATA primary master, {} MiB ({} sectors)",
        device.sectors() / 2048,
        device.sectors()
    );
    println!("[1/6] Preflight: checking disk geometry and UEFI payload...");
    if let Err(error) = FatGeometry::new(device.sectors(), RUNTIME_UEFI_APP.len()) {
        fatal(error);
    }
    println!("[ok] target can hold the CFC database and UEFI runtime");

    let cfc_name = prompt_text(&mut input, "CFC name: ");
    let primary_name = prompt_text(&mut input, "Primary Dimension name: ");
    let cfc_fin = CfcFin::from_u128(random_identity(*b"CFC!", b"genesis-cfc"));
    let primary_fin = Fin::from_u128(random_identity(*b"DIM!", b"genesis-primary"));
    let (operator, storage_key, storage_envelope) = loop {
        let mut password = read_line(&mut input, "Operator password (8-23 characters): ", true);
        let mut confirmation = read_line(&mut input, "Confirm Operator password: ", true);
        if line_bytes(&password) != line_bytes(&confirmation) {
            crypto::wipe(&mut password);
            crypto::wipe(&mut confirmation);
            println!("Passwords do not match; try again.");
            continue;
        }
        crypto::wipe(&mut confirmation);
        match session::genesis_operator(line_bytes(&password)) {
            Ok(record) => {
                let (storage_key, storage_envelope) = if basic {
                    println!("Deriving Argon2id key envelope (64 MiB, 3 passes)...");
                    match create_storage_envelope(cfc_fin, primary_fin, line_bytes(&password)) {
                        Ok((key, envelope)) => (Some(key), Some(envelope)),
                        Err(()) => {
                            crypto::wipe(&mut password);
                            println!("Could not create encrypted CFC key envelope.");
                            continue;
                        }
                    }
                } else {
                    (None, None)
                };
                crypto::wipe(&mut password);
                break (record, storage_key, storage_envelope);
            }
            Err(error) => {
                crypto::wipe(&mut password);
                println!("Invalid password: {}", error.message());
            }
        }
    };

    let config = GenesisConfig {
        cfc_fin,
        cfc_name,
        primary_fin,
        primary_name,
        operator: session::StoredGenesisAccount(operator),
        storage_key,
        storage_envelope,
    };

    println!();
    println!("Installation plan");
    println!(
        "  Mode: {}",
        if basic {
            "Basic encrypted"
        } else {
            "Architect preview"
        }
    );
    println!("  CFC: {}", cfc_name);
    println!("  Primary Dimension: {}", primary_name);
    println!("  Target: ATA primary master (whole disk)");
    println!("WARNING: the existing partition map and accessible data will be replaced.");
    println!("This operation is destructive, but it is not a forensic secure erase.");
    if line(&read_line(
        &mut input,
        "Type ERASE to construct this CFC: ",
        false,
    )) != "ERASE"
    {
        println!("Installation cancelled; no disk writes were made.");
        port::shutdown();
    }

    println!("[2/6] Creating primary and backup GPT metadata...");
    println!("[3/6] Building the EFI System Partition...");
    println!("[4/6] Installing the native UEFI runtime...");
    println!("[5/6] Writing redundant Genesis manifests...");
    if let Err(error) = install(&mut device, RUNTIME_UEFI_APP, config) {
        fatal(error);
    }
    println!("[6/6] Read-back verification complete.");
    slog!(
        "EXPOS_GENESIS_INSTALLED cfc={} dimension={} uefi_bytes={}\r\n",
        cfc_fin,
        primary_fin,
        RUNTIME_UEFI_APP.len()
    );
    println!("[ok] CFC {}", cfc_fin);
    println!("[ok] Primary Dimension {}", primary_fin);
    println!(
        "[ok] ExpFS reserved and Operator seed written ({})",
        if basic { "encrypted" } else { "unencrypted" }
    );
    println!("[ok] EFI/BOOT/BOOTX64.EFI installed");
    println!("[ok] primary/backup manifests and boot payload verified");
    println!();
    println!("Installation complete. Remove the USB, then press Enter to reboot.");
    let _ = read_line(&mut input, "", false);
    port::reboot();
}

#[cfg(feature = "genesis-installer")]
fn random_identity(prefix: [u8; 4], domain: &[u8]) -> u128 {
    let mut bytes = crate::crypto::random_material::<16>(domain);
    bytes[..4].copy_from_slice(&prefix);
    u128::from_be_bytes(bytes)
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
    device.write(MANIFEST_BACKUP_LBA, &sector)?;
    device.write(MANIFEST_LBA, &sector)?;
    device.flush()?;
    verify_install(device, geometry, boot_app, config)?;
    Ok(())
}

fn verify_install<D: BlockDevice>(
    device: &D,
    geometry: FatGeometry,
    boot_app: &[u8],
    config: GenesisConfig,
) -> Result<(), GenesisError> {
    let mut sector = [0_u8; storage::SECTOR_SIZE];
    for lba in [1, geometry.total_sectors - 1] {
        device.read(lba, &mut sector)?;
        if sector[..8] != *b"EFI PART" {
            return Err(GenesisError::VerificationFailed);
        }
        let stored_crc = get_u32(&sector, 16);
        let mut header = sector;
        put_u32(&mut header, 16, 0);
        if crc32(&header[..92]) != stored_crc {
            return Err(GenesisError::VerificationFailed);
        }
    }
    device.read(geometry.partition_start, &mut sector)?;
    if sector[82..90] != *b"FAT32   " || sector[510..512] != [0x55, 0xAA] {
        return Err(GenesisError::VerificationFailed);
    }
    for lba in [MANIFEST_LBA, MANIFEST_BACKUP_LBA] {
        device.read(lba, &mut sector)?;
        let decoded = decode_manifest(&sector).ok_or(GenesisError::VerificationFailed)?;
        if decoded.cfc_fin != config.cfc_fin
            || decoded.cfc_name != config.cfc_name
            || decoded.primary_fin != config.primary_fin
            || decoded.primary_name != config.primary_name
            || decoded.operator.0 != config.operator.0
            || decoded.storage_envelope != config.storage_envelope
        {
            return Err(GenesisError::VerificationFailed);
        }
    }
    for (index, expected) in boot_app.chunks(storage::SECTOR_SIZE).enumerate() {
        device.read(geometry.cluster_lba(5 + index as u32), &mut sector)?;
        if sector[..expected.len()] != *expected
            || (expected.len() < storage::SECTOR_SIZE
                && sector[expected.len()..].iter().any(|byte| *byte != 0))
        {
            return Err(GenesisError::VerificationFailed);
        }
    }
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
            let required = ((data + 2) * 4).div_ceil(512);
            if required == fat_sectors {
                break data;
            }
            fat_sectors = required;
        };
        if clusters < FAT32_MIN_CLUSTERS {
            return Err(GenesisError::DiskTooSmall);
        }
        let file_clusters = (file_bytes as u64).div_ceil(512) as u32;
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
        1..=4 => 0x0FFF_FFFF,
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

fn storage_envelope_aad(cfc: CfcFin, primary: Fin) -> [u8; 40] {
    let mut aad = [0_u8; 40];
    aad[..8].copy_from_slice(b"EXPCFCK1");
    aad[8..24].copy_from_slice(&cfc.bytes());
    aad[24..40].copy_from_slice(&primary.bytes());
    aad
}

#[cfg(feature = "genesis-installer")]
fn create_storage_envelope(
    cfc: CfcFin,
    primary: Fin,
    password: &[u8],
) -> Result<([u8; crate::crypto::STORAGE_KEY_LEN], StorageEnvelope), ()> {
    let salt = crate::crypto::random_material::<{ crate::crypto::STORAGE_SALT_LEN }>(b"kdf-salt");
    let nonce = crate::crypto::random_material::<{ crate::crypto::STORAGE_NONCE_LEN }>(b"key-wrap");
    let storage_key =
        crate::crypto::random_material::<{ crate::crypto::STORAGE_KEY_LEN }>(b"cfc-key");
    let mut wrapped_key = storage_key;
    let mut kek = crate::crypto::storage_kek(password, &salt)?;
    let tag = crate::crypto::seal_storage(
        &kek,
        &nonce,
        &storage_envelope_aad(cfc, primary),
        &mut wrapped_key,
    )?;
    crate::crypto::wipe(&mut kek);
    Ok((
        storage_key,
        StorageEnvelope {
            salt,
            nonce,
            wrapped_key,
            tag,
        },
    ))
}

fn encode_manifest(config: GenesisConfig) -> [u8; 512] {
    let mut sector = [0_u8; 512];
    sector[..8].copy_from_slice(&MANIFEST_MAGIC);
    put_u16(&mut sector, 8, MANIFEST_VERSION);
    sector[10] = if config.storage_envelope.is_some() {
        1
    } else {
        2
    };
    sector[11] = u8::from(config.storage_envelope.is_some());
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
    if let Some(envelope) = config.storage_envelope {
        sector[193..209].copy_from_slice(&envelope.salt);
        sector[209..233].copy_from_slice(&envelope.nonce);
        sector[233..265].copy_from_slice(&envelope.wrapped_key);
        sector[265..281].copy_from_slice(&envelope.tag);
        put_u32(&mut sector, 281, crate::crypto::STORAGE_ARGON2_MEMORY_KIB);
        put_u32(&mut sector, 285, crate::crypto::STORAGE_ARGON2_PASSES);
        sector[289] = crate::crypto::STORAGE_ARGON2_LANES as u8;
        sector[290] = 1; // XChaCha20-Poly1305 suite
    }
    let checksum = crc32(&sector[..508]);
    put_u32(&mut sector, 508, checksum);
    sector
}

fn decode_manifest(sector: &[u8; 512]) -> Option<GenesisConfig> {
    let version = u16::from_le_bytes([sector[8], sector[9]]);
    if sector[..8] != MANIFEST_MAGIC
        || !(1..=MANIFEST_VERSION).contains(&version)
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
    let storage_envelope = if version >= 2 && sector[11] == 1 {
        if sector[10] != 1
            || get_u32(sector, 281) != crate::crypto::STORAGE_ARGON2_MEMORY_KIB
            || get_u32(sector, 285) != crate::crypto::STORAGE_ARGON2_PASSES
            || sector[289] != crate::crypto::STORAGE_ARGON2_LANES as u8
            || sector[290] != 1
        {
            return None;
        }
        Some(StorageEnvelope {
            salt: sector[193..209].try_into().ok()?,
            nonce: sector[209..233].try_into().ok()?,
            wrapped_key: sector[233..265].try_into().ok()?,
            tag: sector[265..281].try_into().ok()?,
        })
    } else if sector[11] == 0 {
        None
    } else {
        return None;
    };
    Some(GenesisConfig {
        cfc_fin,
        cfc_name,
        primary_fin,
        primary_name,
        operator: session::StoredGenesisAccount(account),
        storage_key: None,
        storage_envelope,
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

        fn read(&self, lba: u32, output: &mut [u8; 512]) -> Result<(), GenesisError> {
            let start = lba as usize * 512;
            output.copy_from_slice(&self.bytes[start..start + 512]);
            Ok(())
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
            storage_key: None,
            storage_envelope: None,
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
    fn redundant_manifest_configs_must_agree() {
        let first = config();
        let mut second = config();
        second.primary_fin = Fin::from_u128(99);
        let first = encode_manifest(first);
        let second = encode_manifest(second);
        assert_eq!(
            select_manifest_copies(&first, &second),
            Err(GenesisLoadError::ManifestDamaged)
        );
        assert_eq!(
            select_manifest_copies(&[0; storage::SECTOR_SIZE], &[0; storage::SECTOR_SIZE]),
            Ok(ManifestSelection::None)
        );
        assert!(matches!(
            select_manifest_copies(&first, &[0; storage::SECTOR_SIZE]),
            Ok(ManifestSelection::PrimaryDegraded(_))
        ));
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
        let backup = MANIFEST_BACKUP_LBA as usize * 512;
        assert_eq!(
            &disk.bytes[manifest..manifest + 512],
            &disk.bytes[backup..backup + 512]
        );
    }

    #[test]
    fn encrypted_manifest_preserves_versioned_key_envelope_without_plaintext_key() {
        let mut original = config();
        original.storage_key = Some([0xAA; crate::crypto::STORAGE_KEY_LEN]);
        original.storage_envelope = Some(StorageEnvelope {
            salt: [1; crate::crypto::STORAGE_SALT_LEN],
            nonce: [2; crate::crypto::STORAGE_NONCE_LEN],
            wrapped_key: [3; crate::crypto::STORAGE_KEY_LEN],
            tag: [4; crate::crypto::STORAGE_TAG_LEN],
        });
        let encoded = encode_manifest(original);
        assert_eq!(encoded[10], 1);
        assert_eq!(encoded[11], 1);
        assert!(!encoded
            .windows(crate::crypto::STORAGE_KEY_LEN)
            .any(|w| w == [0xAA; 32]));
        let decoded = decode_manifest(&encoded).unwrap();
        assert!(decoded.storage_key.is_none());
        let envelope = decoded.storage_envelope.unwrap();
        assert_eq!(envelope.salt, [1; crate::crypto::STORAGE_SALT_LEN]);
        assert_eq!(envelope.wrapped_key, [3; crate::crypto::STORAGE_KEY_LEN]);
    }

    #[test]
    fn fat32_rejects_tiny_targets() {
        assert_eq!(
            FatGeometry::new(8192, 1024).unwrap_err(),
            GenesisError::DiskTooSmall
        );
    }
}
