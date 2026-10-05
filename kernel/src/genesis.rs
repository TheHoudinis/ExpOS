//! Genesis installation manifest and the first bootable-disk constructor.
//!
//! Genesis deliberately writes a standards-based GPT + FAT32 EFI System
//! Partition, while ExpFS remains the CFC-owned transactional database in the
//! reserved pre-partition area. Basic installations use a random per-CFC
//! storage key, Argon2id key wrapping and XChaCha20-Poly1305 authenticated
//! ExpFS snapshots; Architect retains an explicit unencrypted option.

#![cfg_attr(any(test, not(feature = "genesis-installer")), allow(dead_code))]

use crate::{session, storage};
use expos_core::{CfcFin, Fin, Text};

const MANIFEST_LBA: u32 = 40;
const MANIFEST_BACKUP_LBA: u32 = 41;
const PLAN_LBA: u32 = 42;
const PLAN_BACKUP_LBA: u32 = 43;
const MANIFEST_MAGIC: [u8; 8] = *b"EXGEN001";
const MANIFEST_VERSION: u16 = 4;
const PLAN_MAGIC: [u8; 8] = *b"EXPLAN02";
const PLAN_VERSION: u16 = 1;
pub const MAX_GENESIS_DIMENSIONS: usize = 5;
pub const MAX_GENESIS_USERS: usize = 2;
pub const COMPONENT_DESKTOP: u8 = 1 << 0;
pub const COMPONENT_BROWSER: u8 = 1 << 1;
pub const COMPONENT_PYTHON: u8 = 1 << 2;
pub const DRIVER_NETWORK: u8 = 1 << 0;
pub const SDK_RUST: u8 = 1 << 0;
pub const SDK_C: u8 = 1 << 1;
pub const SDK_GO: u8 = 1 << 2;
pub const SDK_PYTHON: u8 = 1 << 3;
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
    pub locale: crate::locale::Locale,
    pub plan: GenesisPlan,
    pub storage_key: Option<[u8; crate::crypto::STORAGE_KEY_LEN]>,
    storage_envelope: Option<StorageEnvelope>,
    plan_required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncryptionProfile {
    Unencrypted,
    Easy,
    Paranoid,
}

impl EncryptionProfile {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unencrypted => "Unencrypted (Architect only)",
            Self::Easy => "Easy encrypted",
            Self::Paranoid => "Paranoid hardened",
        }
    }

    pub(crate) const fn persisted(self) -> u8 {
        match self {
            Self::Unencrypted => 0,
            Self::Easy => 1,
            Self::Paranoid => 2,
        }
    }

    const fn from_persisted(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Unencrypted),
            1 => Some(Self::Easy),
            2 => Some(Self::Paranoid),
            _ => None,
        }
    }

    const fn argon2_passes(self) -> u32 {
        match self {
            Self::Paranoid => crate::crypto::STORAGE_PARANOID_ARGON2_PASSES,
            _ => crate::crypto::STORAGE_ARGON2_PASSES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenesisDimension {
    pub fin: Fin,
    pub name: Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenesisPlan {
    pub secondary_dimensions: [Option<GenesisDimension>; MAX_GENESIS_DIMENSIONS],
    pub secondary_accounts: [Option<session::StoredGenesisAccount>; MAX_GENESIS_USERS],
    pub initial_apps: u32,
    pub components: u8,
    pub sdks: u8,
    pub drivers: u8,
    pub encryption: EncryptionProfile,
}

impl GenesisPlan {
    pub const fn standard(encryption: EncryptionProfile) -> Self {
        Self {
            secondary_dimensions: [None; MAX_GENESIS_DIMENSIONS],
            secondary_accounts: [None; MAX_GENESIS_USERS],
            initial_apps: 0,
            components: COMPONENT_DESKTOP | COMPONENT_BROWSER | COMPONENT_PYTHON,
            sdks: 0,
            drivers: DRIVER_NETWORK,
            encryption,
        }
    }

    pub fn network_allowed(self) -> bool {
        self.drivers & DRIVER_NETWORK != 0 && self.encryption != EncryptionProfile::Paranoid
    }

    pub fn python_allowed(self) -> bool {
        self.components & COMPONENT_PYTHON != 0 && self.encryption != EncryptionProfile::Paranoid
    }

    pub fn browser_allowed(self) -> bool {
        self.components & COMPONENT_BROWSER != 0 && self.encryption != EncryptionProfile::Paranoid
    }
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
    memory_kib: u32,
    passes: u32,
    lanes: u32,
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
    if config.plan_required {
        let mut primary_plan = [0_u8; storage::SECTOR_SIZE];
        let mut backup_plan = [0_u8; storage::SECTOR_SIZE];
        device
            .read_sector(PLAN_LBA, &mut primary_plan)
            .map_err(|_| GenesisLoadError::ManifestDamaged)?;
        device
            .read_sector(PLAN_BACKUP_LBA, &mut backup_plan)
            .map_err(|_| GenesisLoadError::ManifestDamaged)?;
        config.plan = match (decode_plan(&primary_plan), decode_plan(&backup_plan)) {
            (Some(primary), Some(backup)) if primary == backup => primary,
            (Some(plan), None) => {
                crate::slog!("EXPOS_GENESIS_PLAN_DEGRADED source=primary copies=1\r\n");
                plan
            }
            (None, Some(plan)) => {
                crate::slog!("EXPOS_GENESIS_PLAN_RECOVERED source=backup\r\n");
                plan
            }
            _ => return Err(GenesisLoadError::ManifestDamaged),
        };
        if config.plan.encryption == EncryptionProfile::Unencrypted
            && config.storage_envelope.is_some()
            || config.plan.encryption != EncryptionProfile::Unencrypted
                && config.storage_envelope.is_none()
        {
            return Err(GenesisLoadError::ManifestDamaged);
        }
    }
    let Some(envelope) = config.storage_envelope else {
        return Ok(Some(config));
    };
    crate::println!("Encrypted CFC storage detected: {}", config.cfc_name);
    let mut failures = 0_u8;
    loop {
        let mut password = read_line(input, "Unlock password: ", true);
        let mut kek = match crate::crypto::storage_kek_with_params(
            line_bytes(&password),
            &envelope.salt,
            envelope.memory_kib,
            envelope.passes,
            envelope.lanes,
        ) {
            Ok(key) => key,
            Err(()) => {
                crate::crypto::wipe(&mut password);
                crate::println!("Storage-key derivation failed.");
                continue;
            }
        };
        let mut storage_key = envelope.wrapped_key;
        let opened = if config.plan_required {
            crate::crypto::open_storage(
                &kek,
                &envelope.nonce,
                &storage_envelope_aad(config.cfc_fin, config.primary_fin, config.plan),
                &mut storage_key,
                &envelope.tag,
            )
        } else {
            crate::crypto::open_storage(
                &kek,
                &envelope.nonce,
                &legacy_storage_envelope_aad(config.cfc_fin, config.primary_fin),
                &mut storage_key,
                &envelope.tag,
            )
        }
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
        failures = failures.saturating_add(1);
        crate::println!(
            "Wrong password or damaged CFC key envelope ({}/5).",
            failures
        );
        if failures == 5 {
            crate::println!("Too many unlock failures. Powering down.");
            crate::slog!("EXPOS_CFC_UNLOCK_LOCKOUT attempts=5\r\n");
            crate::port::shutdown();
        }
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
    use crate::{
        crypto,
        input::Input,
        locale::{Locale, Text as LocalText},
        port, println, slog,
    };

    let graphical = crate::graphics_console::enable_genesis();
    slog!(
        "EXPOS_GENESIS_DISPLAY visible={} backend={}\r\n",
        graphical,
        if graphical { "uefi-gop" } else { "serial" }
    );
    let mut input = Input::new();
    println!();
    println!("ExpOS Genesis");
    for (index, locale) in Locale::ALL.iter().copied().enumerate() {
        println!("{}. {}", index + 1, locale.name());
    }
    let locale = loop {
        let value = read_line(
            &mut input,
            Locale::English.text(LocalText::LanguagePrompt),
            false,
        );
        let Some(choice) = line(&value).as_bytes().first().copied() else {
            continue;
        };
        if (b'1'..=b'5').contains(&choice) {
            break Locale::ALL[(choice - b'1') as usize];
        }
    };
    crate::locale::set_active(locale);
    let mut device = match storage::initialize() {
        Ok(device) => device,
        Err(error) => fatal(GenesisError::Storage(error)),
    };
    show_hardware_overview(&input, device);
    if installed_metadata_present(device) {
        println!();
        println!("Installed CFC metadata found on the target.");
        println!("1. Recovery and troubleshooting");
        println!("2. Replace with a new installation");
        let action = read_choice(&mut input, "Action [1/2]: ", 1, 2);
        if action == 1 {
            run_recovery(&mut input, &mut device);
        }
    }
    println!();
    println!("ExpOS Genesis");
    println!("1. {}", locale.text(LocalText::BasicEncrypted));
    println!("2. {}", locale.text(LocalText::ArchitectPreview));
    let basic = loop {
        let mode = read_line(&mut input, locale.text(LocalText::InstallationMode), false);
        if line(&mode) == "1" {
            break true;
        }
        if line(&mode) == "2" {
            break false;
        }
        println!("Choose 1 or 2.");
    };

    println!(
        "Target: {} block device, {} MiB ({} sectors)",
        device.backend(),
        device.sectors() / 2048,
        device.sectors()
    );
    progress(1, 8, "Preflight", "checking disk geometry and UEFI payload");
    if let Err(error) = FatGeometry::new(device.sectors(), RUNTIME_UEFI_APP.len()) {
        fatal(error);
    }
    println!("[ok] target can hold the CFC database and UEFI runtime");

    let cfc_name = prompt_text(&mut input, locale.text(LocalText::CfcName));
    let primary_name = prompt_text(&mut input, locale.text(LocalText::PrimaryName));
    let cfc_fin = CfcFin::from_u128(random_identity(*b"CFC!", b"genesis-cfc"));
    let primary_fin = Fin::from_u128(random_identity(*b"DIM!", b"genesis-primary"));
    let encryption = prompt_encryption(&mut input, basic);
    let mut plan = prompt_plan(&mut input, encryption);
    prompt_dimensions(&mut input, &mut plan);
    prompt_secondary_users(&mut input, &mut plan);
    let (operator, storage_key, storage_envelope) = loop {
        let mut password = read_line(&mut input, locale.text(LocalText::OperatorPassword), true);
        let mut confirmation = read_line(&mut input, locale.text(LocalText::ConfirmPassword), true);
        if line_bytes(&password) != line_bytes(&confirmation) {
            crypto::wipe(&mut password);
            crypto::wipe(&mut confirmation);
            println!("Passwords do not match; try again.");
            continue;
        }
        crypto::wipe(&mut confirmation);
        match session::genesis_operator(line_bytes(&password)) {
            Ok(record) => {
                let (storage_key, storage_envelope) =
                    if encryption != EncryptionProfile::Unencrypted {
                        println!(
                            "Deriving Argon2id key envelope (64 MiB, {} passes)...",
                            encryption.argon2_passes()
                        );
                        match create_storage_envelope(
                            cfc_fin,
                            primary_fin,
                            line_bytes(&password),
                            encryption,
                            plan,
                        ) {
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
        locale,
        plan,
        storage_key,
        storage_envelope,
        plan_required: true,
    };

    println!();
    println!("{}", locale.text(LocalText::InstallationPlan));
    println!("  Mode: {}", if basic { "Basic" } else { "Architect" });
    println!("  Encryption: {}", encryption.label());
    println!("  CFC: {}", cfc_name);
    println!("  Primary Dimension: {}", primary_name);
    println!(
        "  Extra Dimensions: {}  Users: {}  Apps: {}",
        plan.secondary_dimensions.iter().flatten().count(),
        plan.secondary_accounts.iter().flatten().count(),
        plan.initial_apps.count_ones()
    );
    println!(
        "  Network driver: {}  SDK selection: {:#04x}",
        if plan.network_allowed() {
            "enabled"
        } else {
            "disabled"
        },
        plan.sdks
    );
    println!("  Target: {} block device (whole disk)", device.backend());
    println!("{}", locale.text(LocalText::DestructiveWarning));
    println!("This operation is destructive, but it is not a forensic secure erase.");
    if line(&read_line(
        &mut input,
        locale.text(LocalText::TypeErase),
        false,
    )) != "ERASE"
    {
        println!("{}", locale.text(LocalText::Cancelled));
        port::shutdown();
    }

    progress(2, 8, "Storage", "creating primary and backup GPT metadata");
    progress(3, 8, "Boot", "building the EFI System Partition");
    progress(4, 8, "Runtime", "installing the native UEFI runtime");
    progress(5, 8, "Identity", "writing redundant Genesis manifests");
    progress(
        6,
        8,
        "Plan",
        "writing users, Dimensions, packages and policy",
    );
    if let Err(error) = install(&mut device, RUNTIME_UEFI_APP, config) {
        fatal(error);
    }
    progress(
        7,
        8,
        "Verify",
        "checking GPT, FAT, manifests and runtime bytes",
    );
    progress(8, 8, "Complete", "all installation data verified");
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
        encryption.label()
    );
    println!("[ok] EFI/BOOT/BOOTX64.EFI installed");
    println!("[ok] primary/backup manifests and boot payload verified");
    println!();
    println!("{}", locale.text(LocalText::Complete));
    println!("Remove the USB, then press Enter to reboot.");
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

#[cfg(feature = "genesis-installer")]
fn read_choice(input: &mut crate::input::Input, prompt: &str, minimum: u8, maximum: u8) -> u8 {
    loop {
        let value = read_line(input, prompt, false);
        if let Some(choice) = parse_choice(line(&value), minimum, maximum) {
            return choice;
        }
        crate::println!("Choose {}-{}.", minimum, maximum);
    }
}

fn parse_choice(value: &str, minimum: u8, maximum: u8) -> Option<u8> {
    if value.is_empty() {
        return None;
    }
    let mut parsed = 0_u16;
    for byte in value.bytes() {
        let digit = byte.checked_sub(b'0')?;
        if digit > 9 {
            return None;
        }
        parsed = parsed.checked_mul(10)?.checked_add(digit as u16)?;
    }
    let parsed = u8::try_from(parsed).ok()?;
    (minimum..=maximum).contains(&parsed).then_some(parsed)
}

#[cfg(feature = "genesis-installer")]
fn prompt_encryption(input: &mut crate::input::Input, basic: bool) -> EncryptionProfile {
    crate::println!();
    crate::println!("Storage protection");
    if basic {
        crate::println!("1. Easy encrypted - Argon2id 64 MiB / 3 passes");
        crate::println!("2. Paranoid - 6 passes, network and ExpPython disabled");
        match read_choice(input, "Protection [1/2]: ", 1, 2) {
            2 => EncryptionProfile::Paranoid,
            _ => EncryptionProfile::Easy,
        }
    } else {
        crate::println!("1. Unencrypted - development only");
        crate::println!("2. Easy encrypted - Argon2id 64 MiB / 3 passes");
        crate::println!("3. Paranoid - 6 passes, network and ExpPython disabled");
        match read_choice(input, "Protection [1/3]: ", 1, 3) {
            1 => EncryptionProfile::Unencrypted,
            3 => EncryptionProfile::Paranoid,
            _ => EncryptionProfile::Easy,
        }
    }
}

#[cfg(feature = "genesis-installer")]
fn prompt_plan(input: &mut crate::input::Input, encryption: EncryptionProfile) -> GenesisPlan {
    let mut plan = GenesisPlan::standard(encryption);
    crate::println!();
    crate::println!("Initial packages");
    crate::println!("1. Minimal - core desktop only");
    crate::println!("2. Essentials - Browser, Calculator, Clock, Calendar");
    crate::println!("3. Everything - Browser and all 20 built-in Ayo apps");
    crate::println!("4. Custom - select Browser, ExpPython and individual apps");
    match read_choice(input, "Package set [1/4]: ", 1, 4) {
        1 => plan.components = COMPONENT_DESKTOP,
        2 => {
            plan.components = COMPONENT_DESKTOP | COMPONENT_BROWSER;
            plan.initial_apps = (1 << 0) | (1 << 2) | (1 << 3);
        }
        3 => plan.initial_apps = (1_u32 << crate::apps::PACKAGE_COUNT) - 1,
        4 => customize_packages(input, &mut plan, encryption),
        _ => unreachable!(),
    }

    crate::println!();
    crate::println!("Development components");
    crate::println!("1. None");
    crate::println!("2. Rust + C Form SDK contracts");
    crate::println!("3. Rust + C + Go + Python SDK contracts");
    plan.sdks = match read_choice(input, "SDK set [1/3]: ", 1, 3) {
        2 => SDK_RUST | SDK_C,
        3 => SDK_RUST | SDK_C | SDK_GO | SDK_PYTHON,
        _ => 0,
    };

    if encryption == EncryptionProfile::Paranoid {
        plan.components &= !(COMPONENT_BROWSER | COMPONENT_PYTHON);
        plan.drivers &= !DRIVER_NETWORK;
        crate::println!("Paranoid policy: network, Browser and ExpPython are disabled.");
    } else if read_choice(
        input,
        "Enable the detected network driver? [1 yes / 2 no]: ",
        1,
        2,
    ) == 2
    {
        plan.drivers &= !DRIVER_NETWORK;
    }
    plan
}

#[cfg(feature = "genesis-installer")]
fn customize_packages(
    input: &mut crate::input::Input,
    plan: &mut GenesisPlan,
    encryption: EncryptionProfile,
) {
    plan.components = COMPONENT_DESKTOP;
    plan.initial_apps = 0;
    if encryption != EncryptionProfile::Paranoid {
        if read_choice(input, "Install Browser? [1 yes / 2 no]: ", 1, 2) == 1 {
            plan.components |= COMPONENT_BROWSER;
        }
        if read_choice(input, "Install ExpPython? [1 yes / 2 no]: ", 1, 2) == 1 {
            plan.components |= COMPONENT_PYTHON;
        }
    }
    crate::println!("Available Ayo apps");
    for index in 0..crate::apps::PACKAGE_COUNT {
        crate::println!(
            "  {}. {}",
            index + 1,
            crate::apps::package_name(index).unwrap_or("Unknown")
        );
    }
    crate::println!("Enter each app number to add it; enter 0 when finished.");
    loop {
        let choice = read_choice(
            input,
            "Add app [0 done / 1-20]: ",
            0,
            crate::apps::PACKAGE_COUNT as u8,
        );
        if choice == 0 {
            break;
        }
        let bit = 1_u32 << (choice - 1);
        if plan.initial_apps & bit == 0 {
            plan.initial_apps |= bit;
            crate::println!(
                "[ok] {} selected",
                crate::apps::package_name(choice as usize - 1).unwrap_or("App")
            );
        } else {
            crate::println!("Already selected.");
        }
    }
}

#[cfg(feature = "genesis-installer")]
fn prompt_dimensions(input: &mut crate::input::Input, plan: &mut GenesisPlan) {
    let count = read_choice(input, "Additional Dimensions [0-5]: ", 0, 5) as usize;
    for index in 0..count {
        let name = prompt_text(input, "Dimension name: ");
        plan.secondary_dimensions[index] = Some(GenesisDimension {
            fin: Fin::from_u128(random_identity(*b"DIM!", b"genesis-secondary")),
            name,
        });
    }
}

#[cfg(feature = "genesis-installer")]
fn prompt_secondary_users(input: &mut crate::input::Input, plan: &mut GenesisPlan) {
    let count = read_choice(input, "Additional users [0-2]: ", 0, 2) as usize;
    for index in 0..count {
        loop {
            let name = read_line(input, "User name: ", false);
            let authority = match read_choice(input, "Role [1 Power / 2 Guest]: ", 1, 2) {
                1 => expos_core::Authority::Power,
                _ => expos_core::Authority::Guest,
            };
            let mut password = read_line(input, "User password: ", true);
            let mut confirmation = read_line(input, "Confirm password: ", true);
            if line_bytes(&password) != line_bytes(&confirmation) {
                crate::crypto::wipe(&mut password);
                crate::crypto::wipe(&mut confirmation);
                crate::println!("Passwords do not match; try again.");
                continue;
            }
            crate::crypto::wipe(&mut confirmation);
            match session::genesis_account(line(&name), line_bytes(&password), authority) {
                Ok(account) => {
                    crate::crypto::wipe(&mut password);
                    if plan.secondary_accounts[..index]
                        .iter()
                        .flatten()
                        .any(|existing| existing.name() == account.name())
                    {
                        crate::println!("That user is already in the plan.");
                        continue;
                    }
                    plan.secondary_accounts[index] = Some(account);
                    break;
                }
                Err(error) => {
                    crate::crypto::wipe(&mut password);
                    crate::println!("Invalid user: {}", error.message());
                }
            }
        }
    }
}

#[cfg(feature = "genesis-installer")]
fn progress(step: usize, total: usize, title: &str, detail: &str) {
    let filled = step * 20 / total;
    crate::print!("[");
    for index in 0..20 {
        crate::print!("{}", if index < filled { '#' } else { '.' });
    }
    crate::println!("] {}/{} {}: {}", step, total, title, detail);
}

#[cfg(feature = "genesis-installer")]
fn show_hardware_overview(input: &crate::input::Input, device: storage::Device) {
    let maximum_extended = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    let extended = if maximum_extended >= 0x8000_0001 {
        Some(core::arch::x86_64::__cpuid(0x8000_0001))
    } else {
        None
    };
    let basic = core::arch::x86_64::__cpuid(1);
    let maximum_basic = core::arch::x86_64::__cpuid(0).eax;
    let smep = maximum_basic >= 7 && core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 7) != 0;
    let long_mode = extended.is_some_and(|leaf| leaf.edx & (1 << 29) != 0);
    let nx = extended.is_some_and(|leaf| leaf.edx & (1 << 20) != 0);
    let rdrand = basic.ecx & (1 << 30) != 0;
    let usb = crate::usb::status();
    let input_ready = input.ps2_keyboard_ready() || usb.keyboards != 0;
    let display = crate::boot::firmware_framebuffer().is_some();
    crate::println!();
    crate::println!("Hardware compatibility");
    crate::println!(
        "  CPU: x86_64={} NX={} SMEP={} hardware entropy={}",
        yes_no(long_mode),
        yes_no(nx),
        yes_no(smep),
        yes_no(rdrand)
    );
    crate::println!(
        "  Storage: {} {} MiB - compatible",
        device.backend(),
        device.sectors() / 2048
    );
    crate::println!(
        "  Input: PS/2={} USB keyboards={} - {}",
        yes_no(input.ps2_keyboard_ready()),
        usb.keyboards,
        if input_ready {
            "compatible"
        } else {
            "not detected"
        }
    );
    crate::println!(
        "  Display: UEFI GOP={} - {}",
        yes_no(display),
        if display {
            "compatible"
        } else {
            "console fallback"
        }
    );
    crate::println!("  Ethernet: RTL8139 only; other adapters remain unsupported");
    crate::println!(
        "  Overall: {}",
        if long_mode && nx && smep && rdrand && input_ready {
            "compatible with encrypted Genesis"
        } else {
            "limited; review unsupported items before install"
        }
    );
}

#[cfg(feature = "genesis-installer")]
const fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

#[cfg(feature = "genesis-installer")]
fn installed_metadata_present(device: storage::Device) -> bool {
    let mut sector = [0_u8; storage::SECTOR_SIZE];
    [MANIFEST_LBA, MANIFEST_BACKUP_LBA, PLAN_LBA, PLAN_BACKUP_LBA]
        .into_iter()
        .any(|lba| {
            device.read_sector(lba, &mut sector).is_ok()
                && (sector[..8] == MANIFEST_MAGIC || sector[..8] == PLAN_MAGIC)
        })
}

#[cfg(feature = "genesis-installer")]
fn run_recovery(input: &mut crate::input::Input, device: &mut storage::Device) -> ! {
    crate::clear_console();
    crate::println!("ExpOS Genesis Recovery");
    crate::println!("1. Troubleshoot (read only)");
    crate::println!("2. Repair redundant Genesis metadata");
    crate::println!("3. Restore protected installation baseline");
    crate::println!("4. Annihilate this ExpOS installation");
    match read_choice(input, "Recovery action [1-4]: ", 1, 4) {
        1 => {
            diagnose_installation(*device);
            let _ = read_line(input, "Press Enter to power down.", false);
        }
        2 => {
            match repair_metadata(device) {
                Ok(repaired) if repaired => crate::println!("Repair complete and verified."),
                Ok(_) => crate::println!("Both metadata copies are already healthy."),
                Err(error) => crate::println!("Repair refused: {}", error),
            }
            let _ = read_line(input, "Press Enter to power down.", false);
        }
        3 => {
            match load(input) {
                Ok(Some(config)) => {
                    crate::expfs_store::initialize(
                        config.cfc_fin,
                        config.cfc_name,
                        config.primary_fin,
                        config.storage_key,
                    );
                    match crate::expfs_store::restore_baseline() {
                        Ok(generation) => crate::println!(
                            "Baseline restored and verified as generation {}.",
                            generation
                        ),
                        Err(error) => {
                            crate::println!("Baseline restore failed: {}", error.message())
                        }
                    }
                }
                Ok(None) => crate::println!("No installed CFC manifest was found."),
                Err(error) => crate::println!("Cannot unlock recovery: {}", error.message()),
            }
            let _ = read_line(input, "Press Enter to power down.", false);
        }
        4 => annihilate(input, device),
        _ => unreachable!(),
    }
    crate::port::shutdown()
}

#[cfg(feature = "genesis-installer")]
fn diagnose_installation(device: storage::Device) {
    let mut primary = [0_u8; storage::SECTOR_SIZE];
    let mut backup = [0_u8; storage::SECTOR_SIZE];
    let mut plan_primary = [0_u8; storage::SECTOR_SIZE];
    let mut plan_backup = [0_u8; storage::SECTOR_SIZE];
    let manifest_reads = device.read_sector(MANIFEST_LBA, &mut primary).is_ok()
        && device.read_sector(MANIFEST_BACKUP_LBA, &mut backup).is_ok();
    let plan_reads = device.read_sector(PLAN_LBA, &mut plan_primary).is_ok()
        && device
            .read_sector(PLAN_BACKUP_LBA, &mut plan_backup)
            .is_ok();
    let primary_config = decode_manifest(&primary);
    let backup_config = decode_manifest(&backup);
    crate::println!("Recovery diagnostics");
    crate::println!(
        "  Disk: {} MiB via {}",
        device.sectors() / 2048,
        device.backend()
    );
    crate::println!(
        "  Manifest: readable={} primary={} backup={} agree={}",
        yes_no(manifest_reads),
        yes_no(primary_config.is_some()),
        yes_no(backup_config.is_some()),
        yes_no(primary_config.is_some() && primary_config == backup_config)
    );
    crate::println!(
        "  Plan: readable={} primary={} backup={} agree={}",
        yes_no(plan_reads),
        yes_no(decode_plan(&plan_primary).is_some()),
        yes_no(decode_plan(&plan_backup).is_some()),
        yes_no(
            decode_plan(&plan_primary).is_some()
                && decode_plan(&plan_primary) == decode_plan(&plan_backup)
        )
    );
    if let Some(config) = primary_config.or(backup_config) {
        crate::println!("  CFC: {} ({})", config.cfc_name, config.cfc_fin);
        crate::println!(
            "  Encryption: {}",
            if config.storage_envelope.is_some() {
                "yes"
            } else {
                "no"
            }
        );
    }
    crate::println!("  No write was performed.");
}

#[cfg(any(test, feature = "genesis-installer"))]
fn repair_metadata<D: BlockDevice>(device: &mut D) -> Result<bool, &'static str> {
    let mut primary = [0_u8; storage::SECTOR_SIZE];
    let mut backup = [0_u8; storage::SECTOR_SIZE];
    device
        .read(MANIFEST_LBA, &mut primary)
        .map_err(|_| "cannot read primary manifest")?;
    device
        .read(MANIFEST_BACKUP_LBA, &mut backup)
        .map_err(|_| "cannot read backup manifest")?;
    let (manifest, manifest_target) = match (decode_manifest(&primary), decode_manifest(&backup)) {
        (Some(left), Some(right)) if left == right => (None, 0),
        (Some(_), Some(_)) => return Err("valid manifest copies disagree"),
        (Some(_), None) => (Some(primary), MANIFEST_BACKUP_LBA),
        (None, Some(_)) => (Some(backup), MANIFEST_LBA),
        _ => return Err("neither manifest copy is valid"),
    };
    let config = decode_manifest(if decode_manifest(&primary).is_some() {
        &primary
    } else {
        &backup
    })
    .ok_or("no usable manifest")?;

    let mut plan_primary = [0_u8; storage::SECTOR_SIZE];
    let mut plan_backup = [0_u8; storage::SECTOR_SIZE];
    device
        .read(PLAN_LBA, &mut plan_primary)
        .map_err(|_| "cannot read primary plan")?;
    device
        .read(PLAN_BACKUP_LBA, &mut plan_backup)
        .map_err(|_| "cannot read backup plan")?;
    let (plan, plan_target) = if config.plan_required {
        match (decode_plan(&plan_primary), decode_plan(&plan_backup)) {
            (Some(left), Some(right)) if left == right => (None, 0),
            (Some(_), Some(_)) => return Err("valid plan copies disagree"),
            (Some(_), None) => (Some(plan_primary), PLAN_BACKUP_LBA),
            (None, Some(_)) => (Some(plan_backup), PLAN_LBA),
            _ => return Err("neither Genesis plan copy is valid"),
        }
    } else {
        (None, 0)
    };
    if let Some(bytes) = manifest {
        BlockDevice::write(device, manifest_target, &bytes).map_err(|_| "manifest write failed")?;
    }
    if let Some(bytes) = plan {
        BlockDevice::write(device, plan_target, &bytes).map_err(|_| "plan write failed")?;
    }
    BlockDevice::flush(device).map_err(|_| "flush failed")?;
    let repaired = manifest.is_some() || plan.is_some();
    if repaired {
        BlockDevice::read(device, MANIFEST_LBA, &mut primary)
            .map_err(|_| "cannot verify primary manifest")?;
        BlockDevice::read(device, MANIFEST_BACKUP_LBA, &mut backup)
            .map_err(|_| "cannot verify backup manifest")?;
        if !matches!(
            select_manifest_copies(&primary, &backup),
            Ok(ManifestSelection::Primary(_))
        ) {
            return Err("manifest repair verification failed");
        }
        if config.plan_required {
            BlockDevice::read(device, PLAN_LBA, &mut plan_primary)
                .map_err(|_| "cannot verify primary plan")?;
            BlockDevice::read(device, PLAN_BACKUP_LBA, &mut plan_backup)
                .map_err(|_| "cannot verify backup plan")?;
            if decode_plan(&plan_primary).is_none()
                || decode_plan(&plan_primary) != decode_plan(&plan_backup)
            {
                return Err("plan repair verification failed");
            }
        }
    }
    Ok(repaired)
}

#[cfg(feature = "genesis-installer")]
fn annihilate(input: &mut crate::input::Input, device: &mut storage::Device) -> ! {
    crate::println!("This removes boot, CFC identity, ExpFS state, checkpoints and baseline.");
    crate::println!("It is destructive, immediate, and not a forensic whole-disk erase.");
    if line(&read_line(input, "Type ANNIHILATE: ", false)) != "ANNIHILATE"
        || line(&read_line(input, "Type ERASE: ", false)) != "ERASE"
    {
        crate::println!("Annihilation cancelled.");
        crate::port::shutdown();
    }
    let zero = [0_u8; storage::SECTOR_SIZE];
    let end = device.sectors();
    let ranges = [
        (0, 34),
        (MANIFEST_LBA, PLAN_BACKUP_LBA + 1),
        (64, 328),
        (EFI_PARTITION_START, (EFI_PARTITION_START + 2048).min(end)),
        (end.saturating_sub(33), end),
    ];
    let mut write_failed = false;
    for (start, finish) in ranges {
        for lba in start..finish {
            if BlockDevice::write(device, lba, &zero).is_err() {
                write_failed = true;
                break;
            }
        }
        if write_failed {
            break;
        }
    }
    if write_failed || BlockDevice::flush(device).is_err() {
        crate::println!("Annihilation stopped after a storage write failure.");
        crate::slog!("EXPOS_GENESIS_ANNIHILATED verified=false error=write\r\n");
        crate::port::shutdown();
    }
    let mut check = [1_u8; storage::SECTOR_SIZE];
    let verified = [0, MANIFEST_LBA, PLAN_BACKUP_LBA, 64, EFI_PARTITION_START]
        .into_iter()
        .all(|lba| device.read_sector(lba, &mut check).is_ok() && sector_blank(&check));
    crate::slog!("EXPOS_GENESIS_ANNIHILATED verified={}\r\n", verified);
    crate::println!(
        "Annihilation {}.",
        if verified {
            "complete"
        } else {
            "completed with verification errors"
        }
    );
    crate::port::shutdown()
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
    let plan = encode_plan(config.plan);
    device.write(PLAN_BACKUP_LBA, &plan)?;
    device.write(PLAN_LBA, &plan)?;
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
    for lba in [PLAN_LBA, PLAN_BACKUP_LBA] {
        device.read(lba, &mut sector)?;
        if decode_plan(&sector) != Some(config.plan) {
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

fn legacy_storage_envelope_aad(cfc: CfcFin, primary: Fin) -> [u8; 40] {
    let mut aad = [0_u8; 40];
    aad[..8].copy_from_slice(b"EXPCFCK1");
    aad[8..24].copy_from_slice(&cfc.bytes());
    aad[24..40].copy_from_slice(&primary.bytes());
    aad
}

/// Bind the complete Genesis plan to the password-wrapped storage key. A
/// sector editor can still damage an encrypted installation, but cannot turn
/// network, Python, packages, SDKs, users or Dimensions back on and then
/// successfully unlock it.
fn storage_envelope_aad(cfc: CfcFin, primary: Fin, plan: GenesisPlan) -> [u8; 552] {
    let mut aad = [0_u8; 552];
    aad[..8].copy_from_slice(b"EXPCFCK2");
    aad[8..24].copy_from_slice(&cfc.bytes());
    aad[24..40].copy_from_slice(&primary.bytes());
    aad[40..].copy_from_slice(&encode_plan(plan));
    aad
}

#[cfg(feature = "genesis-installer")]
fn create_storage_envelope(
    cfc: CfcFin,
    primary: Fin,
    password: &[u8],
    profile: EncryptionProfile,
    plan: GenesisPlan,
) -> Result<([u8; crate::crypto::STORAGE_KEY_LEN], StorageEnvelope), ()> {
    if !crate::crypto::hardware_entropy_available() {
        return Err(());
    }
    let salt = crate::crypto::random_material::<{ crate::crypto::STORAGE_SALT_LEN }>(b"kdf-salt");
    let nonce = crate::crypto::random_material::<{ crate::crypto::STORAGE_NONCE_LEN }>(b"key-wrap");
    let storage_key =
        crate::crypto::random_material::<{ crate::crypto::STORAGE_KEY_LEN }>(b"cfc-key");
    let mut wrapped_key = storage_key;
    let memory_kib = crate::crypto::STORAGE_ARGON2_MEMORY_KIB;
    let passes = profile.argon2_passes();
    let lanes = crate::crypto::STORAGE_ARGON2_LANES;
    let mut kek =
        crate::crypto::storage_kek_with_params(password, &salt, memory_kib, passes, lanes)?;
    let tag = crate::crypto::seal_storage(
        &kek,
        &nonce,
        &storage_envelope_aad(cfc, primary, plan),
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
            memory_kib,
            passes,
            lanes,
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
    sector[12] = config.locale.persisted();
    sector[13] = u8::from(config.plan_required);
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
        put_u32(&mut sector, 281, envelope.memory_kib);
        put_u32(&mut sector, 285, envelope.passes);
        sector[289] = envelope.lanes as u8;
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
        let memory_kib = get_u32(sector, 281);
        let passes = get_u32(sector, 285);
        let lanes = sector[289] as u32;
        if sector[10] != 1
            || memory_kib != crate::crypto::STORAGE_ARGON2_MEMORY_KIB
            || !matches!(
                passes,
                crate::crypto::STORAGE_ARGON2_PASSES
                    | crate::crypto::STORAGE_PARANOID_ARGON2_PASSES
            )
            || lanes != crate::crypto::STORAGE_ARGON2_LANES
            || sector[290] != 1
        {
            return None;
        }
        Some(StorageEnvelope {
            salt: sector[193..209].try_into().ok()?,
            nonce: sector[209..233].try_into().ok()?,
            wrapped_key: sector[233..265].try_into().ok()?,
            tag: sector[265..281].try_into().ok()?,
            memory_kib,
            passes,
            lanes,
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
        locale: if version >= 3 {
            crate::locale::Locale::from_persisted(sector[12])
        } else {
            crate::locale::Locale::English
        },
        plan: GenesisPlan::standard(if storage_envelope.is_some() {
            EncryptionProfile::Easy
        } else {
            EncryptionProfile::Unencrypted
        }),
        storage_key: None,
        storage_envelope,
        plan_required: version >= 4 && sector[13] == 1,
    })
}

const PLAN_DIMENSION_OFFSET: usize = 32;
const PLAN_DIMENSION_BYTES: usize = 49;
const PLAN_ACCOUNT_OFFSET: usize = 288;
const PLAN_ACCOUNT_BYTES: usize = 79;

fn encode_plan(plan: GenesisPlan) -> [u8; 512] {
    let mut sector = [0_u8; 512];
    sector[..8].copy_from_slice(&PLAN_MAGIC);
    put_u16(&mut sector, 8, PLAN_VERSION);
    sector[10] = plan.encryption.persisted();
    sector[11] = plan.components;
    sector[12] = plan.drivers;
    sector[13] = plan.sdks;
    sector[14] = plan.secondary_dimensions.iter().flatten().count() as u8;
    sector[15] = plan.secondary_accounts.iter().flatten().count() as u8;
    put_u32(&mut sector, 16, plan.initial_apps);
    for (index, dimension) in plan.secondary_dimensions.into_iter().flatten().enumerate() {
        let offset = PLAN_DIMENSION_OFFSET + index * PLAN_DIMENSION_BYTES;
        sector[offset..offset + 16].copy_from_slice(&dimension.fin.bytes());
        put_text(&mut sector, offset + 16, dimension.name);
    }
    for (index, account) in plan.secondary_accounts.into_iter().flatten().enumerate() {
        encode_plan_account(
            &mut sector[PLAN_ACCOUNT_OFFSET + index * PLAN_ACCOUNT_BYTES
                ..PLAN_ACCOUNT_OFFSET + (index + 1) * PLAN_ACCOUNT_BYTES],
            account.0,
        );
    }
    let checksum = crc32(&sector[..508]);
    put_u32(&mut sector, 508, checksum);
    sector
}

fn decode_plan(sector: &[u8; 512]) -> Option<GenesisPlan> {
    if sector[..8] != PLAN_MAGIC
        || u16::from_le_bytes([sector[8], sector[9]]) != PLAN_VERSION
        || crc32(&sector[..508]) != get_u32(sector, 508)
    {
        return None;
    }
    let encryption = EncryptionProfile::from_persisted(sector[10])?;
    let components = sector[11];
    let drivers = sector[12];
    let sdks = sector[13];
    let dimension_count = sector[14] as usize;
    let account_count = sector[15] as usize;
    let initial_apps = get_u32(sector, 16);
    if dimension_count > MAX_GENESIS_DIMENSIONS
        || account_count > MAX_GENESIS_USERS
        || components & !(COMPONENT_DESKTOP | COMPONENT_BROWSER | COMPONENT_PYTHON) != 0
        || drivers & !DRIVER_NETWORK != 0
        || sdks & !(SDK_RUST | SDK_C | SDK_GO | SDK_PYTHON) != 0
        || initial_apps >> crate::apps::PACKAGE_COUNT != 0
        || encryption == EncryptionProfile::Paranoid
            && (drivers & DRIVER_NETWORK != 0
                || components & (COMPONENT_BROWSER | COMPONENT_PYTHON) != 0)
    {
        return None;
    }
    let mut dimensions: [Option<GenesisDimension>; MAX_GENESIS_DIMENSIONS] =
        [None; MAX_GENESIS_DIMENSIONS];
    for index in 0..dimension_count {
        let offset = PLAN_DIMENSION_OFFSET + index * PLAN_DIMENSION_BYTES;
        let fin = Fin::from_u128(u128::from_be_bytes(
            sector[offset..offset + 16].try_into().ok()?,
        ));
        if fin.is_zero() {
            return None;
        }
        let name = get_text(sector, offset + 16)?;
        if dimensions[..index]
            .iter()
            .flatten()
            .any(|dimension| dimension.fin == fin || dimension.name == name)
        {
            return None;
        }
        dimensions[index] = Some(GenesisDimension { fin, name });
    }
    let mut accounts: [Option<session::StoredGenesisAccount>; MAX_GENESIS_USERS] =
        [None; MAX_GENESIS_USERS];
    for index in 0..account_count {
        let start = PLAN_ACCOUNT_OFFSET + index * PLAN_ACCOUNT_BYTES;
        let account = decode_plan_account(&sector[start..start + PLAN_ACCOUNT_BYTES])?;
        let wrapped = session::StoredGenesisAccount(account);
        if accounts[..index]
            .iter()
            .flatten()
            .any(|existing| existing.name() == wrapped.name())
        {
            return None;
        }
        accounts[index] = Some(wrapped);
    }
    Some(GenesisPlan {
        secondary_dimensions: dimensions,
        secondary_accounts: accounts,
        initial_apps,
        components,
        sdks,
        drivers,
        encryption,
    })
}

fn encode_plan_account(output: &mut [u8], account: crate::state::StoredAccount) {
    output.fill(0);
    output[0] = account.name_len;
    output[1..25].copy_from_slice(&account.name);
    output[25..41].copy_from_slice(&account.password_salt);
    output[41..73].copy_from_slice(&account.password_hash);
    output[73..77].copy_from_slice(&account.kdf_rounds.to_le_bytes());
    output[77] = account.authority;
    output[78] = u8::from(account.occupied);
}

fn decode_plan_account(input: &[u8]) -> Option<crate::state::StoredAccount> {
    if input.len() != PLAN_ACCOUNT_BYTES {
        return None;
    }
    let name_len = input[0] as usize;
    let name = &input[1..1 + name_len.min(24)];
    let kdf_rounds = u32::from_le_bytes(input[73..77].try_into().ok()?);
    if !(2..=16).contains(&name_len)
        || !name
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(byte))
        || name == b"operator"
        || !(1_000..=250_000).contains(&kdf_rounds)
        || !matches!(input[77], 1 | 2)
        || input[78] != 1
    {
        return None;
    }
    let mut account = crate::state::StoredAccount::EMPTY;
    account.name_len = name_len as u8;
    account.name.copy_from_slice(&input[1..25]);
    account.password_salt.copy_from_slice(&input[25..41]);
    account.password_hash.copy_from_slice(&input[41..73]);
    account.kdf_rounds = kdf_rounds;
    account.authority = input[77];
    account.occupied = true;
    Some(account)
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
            locale: crate::locale::Locale::English,
            plan: GenesisPlan::standard(EncryptionProfile::Unencrypted),
            storage_key: None,
            storage_envelope: None,
            plan_required: true,
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
        assert_eq!(decoded.locale, original.locale);
        assert_eq!(decoded.operator.0.name_len, 8);
        assert_ne!(decoded.operator.0.password_hash, [0; 32]);
    }

    #[test]
    fn plan_round_trip_preserves_dimensions_users_packages_and_policy() {
        let mut plan = GenesisPlan::standard(EncryptionProfile::Easy);
        plan.secondary_dimensions[0] = Some(GenesisDimension {
            fin: Fin::from_u128(44),
            name: Text::new("Work").unwrap(),
        });
        plan.secondary_accounts[0] = Some(
            session::genesis_account("analyst", b"analyst-pass", expos_core::Authority::Guest)
                .unwrap(),
        );
        plan.initial_apps = (1 << 0) | (1 << 2) | (1 << 3);
        plan.components = COMPONENT_DESKTOP | COMPONENT_BROWSER;
        plan.sdks = SDK_RUST | SDK_C;
        let decoded = decode_plan(&encode_plan(plan)).unwrap();
        assert_eq!(decoded, plan);
        assert_eq!(
            decoded.secondary_dimensions[0].unwrap().name.as_str(),
            "Work"
        );
        assert_eq!(decoded.secondary_accounts[0].unwrap().name(), "analyst");
        assert!(!decoded.python_allowed());
        assert!(decoded.browser_allowed());
    }

    #[test]
    fn chooser_parses_complete_bounded_numbers() {
        assert_eq!(parse_choice("0", 0, 20), Some(0));
        assert_eq!(parse_choice("10", 0, 20), Some(10));
        assert_eq!(parse_choice("20", 0, 20), Some(20));
        assert_eq!(parse_choice("21", 0, 20), None);
        assert_eq!(parse_choice("2x", 0, 20), None);
        assert_eq!(parse_choice("", 0, 20), None);
    }

    #[test]
    fn recovery_repairs_opposite_redundant_metadata_copies_and_verifies_them() {
        let original = config();
        let manifest = encode_manifest(original);
        let plan = encode_plan(original.plan);
        let mut disk = MemoryDisk::new(128);
        disk.write(MANIFEST_LBA, &manifest).unwrap();
        disk.write(MANIFEST_BACKUP_LBA, &manifest).unwrap();
        disk.write(PLAN_LBA, &plan).unwrap();
        disk.write(PLAN_BACKUP_LBA, &plan).unwrap();
        disk.bytes[MANIFEST_LBA as usize * 512] ^= 0xFF;
        disk.bytes[PLAN_BACKUP_LBA as usize * 512] ^= 0xFF;

        assert_eq!(repair_metadata(&mut disk), Ok(true));

        let mut primary = [0_u8; 512];
        let mut backup = [0_u8; 512];
        disk.read(MANIFEST_LBA, &mut primary).unwrap();
        disk.read(MANIFEST_BACKUP_LBA, &mut backup).unwrap();
        assert_eq!(primary, backup);
        assert!(decode_manifest(&primary).is_some());
        disk.read(PLAN_LBA, &mut primary).unwrap();
        disk.read(PLAN_BACKUP_LBA, &mut backup).unwrap();
        assert_eq!(primary, backup);
        assert_eq!(decode_plan(&primary), Some(original.plan));
        assert_eq!(repair_metadata(&mut disk), Ok(false));
    }

    #[test]
    fn encrypted_key_envelope_aad_authenticates_the_genesis_plan() {
        let original = GenesisPlan::standard(EncryptionProfile::Easy);
        let mut changed = original;
        changed.drivers = 0;
        assert_ne!(
            storage_envelope_aad(CfcFin::from_u128(1), Fin::from_u128(2), original),
            storage_envelope_aad(CfcFin::from_u128(1), Fin::from_u128(2), changed)
        );
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
        let plan = PLAN_LBA as usize * 512;
        let plan_backup = PLAN_BACKUP_LBA as usize * 512;
        assert_eq!(&disk.bytes[plan..plan + 8], &PLAN_MAGIC);
        assert_eq!(
            &disk.bytes[plan..plan + 512],
            &disk.bytes[plan_backup..plan_backup + 512]
        );
    }

    #[test]
    fn encrypted_manifest_preserves_versioned_key_envelope_without_plaintext_key() {
        let mut original = config();
        original.storage_key = Some([0xAA; crate::crypto::STORAGE_KEY_LEN]);
        original.plan = GenesisPlan::standard(EncryptionProfile::Easy);
        original.storage_envelope = Some(StorageEnvelope {
            salt: [1; crate::crypto::STORAGE_SALT_LEN],
            nonce: [2; crate::crypto::STORAGE_NONCE_LEN],
            wrapped_key: [3; crate::crypto::STORAGE_KEY_LEN],
            tag: [4; crate::crypto::STORAGE_TAG_LEN],
            memory_kib: crate::crypto::STORAGE_ARGON2_MEMORY_KIB,
            passes: crate::crypto::STORAGE_ARGON2_PASSES,
            lanes: crate::crypto::STORAGE_ARGON2_LANES,
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
