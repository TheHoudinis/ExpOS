//! Versioned, checksummed, disk-backed ExpOS state.
//!
//! Two independent slots are alternated on every commit. At boot, the newest
//! valid generation wins; an interrupted write therefore leaves the previous
//! slot usable. This is intentionally a compact state journal, not a claim of
//! a general-purpose filesystem.

use crate::{println, slog, storage};

pub const MAX_STORED_ACCOUNTS: usize = 12;
pub const ACCOUNT_NAME_CAPACITY: usize = 24;

pub const PREF_PURE_BLACK_APPS: u16 = 1 << 0;
pub const PREF_ROUNDED_CONTROLS: u16 = 1 << 1;
pub const PREF_TASKBAR_VISIBLE: u16 = 1 << 2;
pub const PREF_STATUS_VISIBLE: u16 = 1 << 3;
pub const PREF_WINDOW_BORDERS: u16 = 1 << 4;
pub const PREF_HIGH_CONTRAST: u16 = 1 << 5;
pub const PREF_NETWORK_ENABLED: u16 = 1 << 6;
pub const PREF_BLUETOOTH_ENABLED: u16 = 1 << 7;
pub const PREF_WIFI_ENABLED: u16 = 1 << 8;
/// Draw a translucent offset behind windows during full compositor repaints.
/// Kept opt-in because it adds a framebuffer blend over every window.
pub const PREF_WINDOW_SHADOWS: u16 = 1 << 9;
/// Enable procedural wallpaper rendering instead of the inexpensive solid fill.
pub const PREF_WALLPAPER_EFFECTS: u16 = 1 << 10;
/// Let damaged commits bypass the software cadence wait. Hardware
/// VSync remains independently controlled by `vsync`.
pub const PREF_RESPONSIVE_PRESENTATION: u16 = 1 << 11;

const MAGIC: [u8; 8] = *b"EXPOST03";
const FORMAT_VERSION: u16 = 3;
const HEADER_LEN: usize = 32;
const PAYLOAD_LEN: usize = 1024;
const SLOT_SECTORS: usize = 4;
const SLOT_LEN: usize = SLOT_SECTORS * storage::SECTOR_SIZE;
const SLOT_A_LBA: u32 = 8;
const SLOT_B_LBA: u32 = 16;
const MINIMUM_DISK_SECTORS: u32 = SLOT_B_LBA + SLOT_SECTORS as u32;
const ACCOUNT_RECORD_LEN: usize = 80;
const ACCOUNT_RECORDS_OFFSET: usize = 64;
const FLAG_ACCOUNTS_INITIALIZED: u8 = 1 << 0;
// Bytes 9..14 of the 32-byte preferences record used to be zero-filled
// reservation space. A tag lets version-3 slots written before display timing
// was added keep their safe defaults instead of interpreting reserved bytes as
// user choices.
const DISPLAY_TIMING_TAG: [u8; 3] = *b"HZ1";
const DISPLAY_TIMING_TAG_OFFSET: usize = 9;
const REFRESH_RATE_OFFSET: usize = 12;
const VSYNC_OFFSET: usize = 13;
const PREFERENCES_RESERVED_OFFSET: usize = 14;
const VSYNC_DISABLED_ID: u8 = 0;
const VSYNC_ENABLED_ID: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateError {
    Unavailable,
    Disk(storage::StorageError),
    VerificationFailed,
}

impl StateError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "persistent state disk is unavailable",
            Self::Disk(_) => "persistent state disk I/O failed",
            Self::VerificationFailed => "persistent state verification failed",
        }
    }
}

/// Refresh-rate selection with stable on-disk identifiers.
///
/// The discriminants are part of the EXPOST03 preference wire format and must
/// not be reordered or renumbered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RefreshRate {
    Hz60 = 0,
    Hz75 = 1,
    Hz120 = 2,
    Hz144 = 3,
}

impl RefreshRate {
    pub const DEFAULT: Self = Self::Hz60;

    pub const fn hz(self) -> u16 {
        match self {
            Self::Hz60 => 60,
            Self::Hz75 => 75,
            Self::Hz120 => 120,
            Self::Hz144 => 144,
        }
    }

    pub const fn persisted_id(self) -> u8 {
        self as u8
    }

    pub const fn from_persisted_id(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Hz60),
            1 => Some(Self::Hz75),
            2 => Some(Self::Hz120),
            3 => Some(Self::Hz144),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistentPreferences {
    /// 0=480p, 1=720p, 2=1080p.
    pub display_mode: u8,
    pub theme: u8,
    pub wallpaper: u8,
    pub cursor_theme: u8,
    pub accent: u8,
    pub backdrop: u8,
    pub pointer_speed: u8,
    pub flags: u16,
    pub refresh_rate: RefreshRate,
    pub vsync: bool,
    pub reserved: [u8; 18],
}

impl PersistentPreferences {
    pub const fn new() -> Self {
        Self {
            display_mode: 0,
            theme: 0,
            wallpaper: 0,
            cursor_theme: 0,
            accent: 0,
            backdrop: 0,
            pointer_speed: 1,
            flags: PREF_PURE_BLACK_APPS
                | PREF_ROUNDED_CONTROLS
                | PREF_TASKBAR_VISIBLE
                | PREF_STATUS_VISIBLE
                | PREF_WINDOW_BORDERS
                | PREF_NETWORK_ENABLED,
            refresh_rate: RefreshRate::DEFAULT,
            vsync: true,
            reserved: [0; 18],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoredAccount {
    pub name: [u8; ACCOUNT_NAME_CAPACITY],
    pub name_len: u8,
    pub password_salt: [u8; crate::crypto::PASSWORD_SALT_LEN],
    pub password_hash: [u8; crate::crypto::PASSWORD_HASH_LEN],
    pub kdf_rounds: u32,
    /// Stable wire value: 0=Operator, 1=Power, 2=Guest.
    pub authority: u8,
    pub occupied: bool,
}

impl StoredAccount {
    pub const EMPTY: Self = Self {
        name: [0; ACCOUNT_NAME_CAPACITY],
        name_len: 0,
        password_salt: [0; crate::crypto::PASSWORD_SALT_LEN],
        password_hash: [0; crate::crypto::PASSWORD_HASH_LEN],
        kdf_rounds: 0,
        authority: 2,
        occupied: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PersistentData {
    accounts_initialized: bool,
    preferences: PersistentPreferences,
    accounts: [StoredAccount; MAX_STORED_ACCOUNTS],
}

impl PersistentData {
    const fn new() -> Self {
        Self {
            accounts_initialized: false,
            preferences: PersistentPreferences::new(),
            accounts: [StoredAccount::EMPTY; MAX_STORED_ACCOUNTS],
        }
    }
}

#[derive(Clone, Copy)]
struct RuntimeState {
    data: PersistentData,
    device: Option<storage::Device>,
    generation: u64,
    active_slot: u8,
    loaded: bool,
}

impl RuntimeState {
    const fn new() -> Self {
        Self {
            data: PersistentData::new(),
            device: None,
            generation: 0,
            // Treat B as active before the first commit so a blank disk starts
            // by writing slot A.
            active_slot: 1,
            loaded: false,
        }
    }
}

static STATE: crate::sync::SpinMutex<RuntimeState> =
    crate::sync::SpinMutex::new(RuntimeState::new());

#[derive(Clone, Copy)]
struct DecodedSlot {
    data: PersistentData,
    generation: u64,
    slot: u8,
}

/// Probe and load the dedicated state disk. Missing media is non-fatal: login
/// remains available with safe built-ins, while mutations report Unavailable.
pub fn initialize() {
    let device = match storage::initialize() {
        Ok(device) if device.sectors() >= MINIMUM_DISK_SECTORS => device,
        Ok(device) => {
            println!(
                "[warn] state disk too small: {} sectors (need {})",
                device.sectors(),
                MINIMUM_DISK_SECTORS
            );
            slog!("HEXA_STATE_VOLATILE reason=disk-too-small\r\n");
            return;
        }
        Err(error) => {
            println!("[warn] persistent state unavailable: {:?}", error);
            slog!("HEXA_STATE_VOLATILE error={:?}\r\n", error);
            return;
        }
    };

    let first = read_slot(device, 0).ok();
    let second = read_slot(device, 1).ok();
    let selected = newest_slot(first, second);
    let mut state = STATE.lock();
    state.device = Some(device);
    if let Some(slot) = selected {
        state.data = slot.data;
        state.generation = slot.generation;
        state.active_slot = slot.slot;
        state.loaded = true;
        println!(
            "[ok] persistent state generation {} loaded from slot {}",
            slot.generation,
            if slot.slot == 0 { "A" } else { "B" }
        );
        slog!(
            "HEXA_STATE_READY generation={} slot={} loaded=true\r\n",
            slot.generation,
            if slot.slot == 0 { "A" } else { "B" }
        );
    } else {
        println!("[ok] blank state disk detected; defaults will be journaled");
        slog!("HEXA_STATE_READY generation=0 slot=none loaded=false\r\n");
    }
}

pub fn persistent_available() -> bool {
    STATE.lock().device.is_some()
}

pub fn loaded_generation() -> Option<u64> {
    let state = STATE.lock();
    state.loaded.then_some(state.generation)
}

pub fn preferences() -> PersistentPreferences {
    STATE.lock().data.preferences
}

/// Apply preferences to the current session without writing the state disk.
///
/// Recovery commands use this after a persistence failure so safe display
/// choices still take effect for the remainder of the current boot.
pub fn apply_runtime_preferences(preferences: PersistentPreferences) {
    STATE.lock().data.preferences = sanitize_preferences(preferences);
}

pub fn print_diagnostics() {
    let state = STATE.lock();
    let preferences = state.data.preferences;
    println!("STATE DIAGNOSTICS");
    println!(
        "disk: available={} loaded={} generation={} slot={}",
        state.device.is_some(),
        state.loaded,
        state.generation,
        if !state.loaded {
            "none"
        } else if state.active_slot == 0 {
            "A"
        } else {
            "B"
        }
    );
    println!(
        "display: preset-id={} refresh={}Hz vsync={}",
        preferences.display_mode,
        preferences.refresh_rate.hz(),
        if preferences.vsync { "on" } else { "off" }
    );
    println!(
        "appearance: theme={} wallpaper={} cursor={} accent={} pointer-speed={}",
        preferences.theme,
        preferences.wallpaper,
        preferences.cursor_theme,
        preferences.accent,
        preferences.pointer_speed
    );
    println!(
        "render: shadows={} wallpaper-effects={} policy={}",
        if preferences.flags & PREF_WINDOW_SHADOWS != 0 {
            "on"
        } else {
            "off"
        },
        if preferences.flags & PREF_WALLPAPER_EFFECTS != 0 {
            "on"
        } else {
            "off"
        },
        if preferences.flags & PREF_RESPONSIVE_PRESENTATION != 0 {
            "responsive"
        } else {
            "efficient"
        }
    );
}

pub fn save_preferences(preferences: PersistentPreferences) -> Result<(), StateError> {
    let mut state = STATE.lock();
    let mut candidate = state.data;
    candidate.preferences = sanitize_preferences(preferences);
    commit_locked(&mut state, candidate)
}

pub fn load_accounts() -> Option<[StoredAccount; MAX_STORED_ACCOUNTS]> {
    let state = STATE.lock();
    state
        .data
        .accounts_initialized
        .then_some(state.data.accounts)
}

pub fn save_accounts(accounts: [StoredAccount; MAX_STORED_ACCOUNTS]) -> Result<(), StateError> {
    let mut state = STATE.lock();
    let mut candidate = state.data;
    candidate.accounts = accounts;
    candidate.accounts_initialized = true;
    commit_locked(&mut state, candidate)
}

fn commit_locked(state: &mut RuntimeState, candidate: PersistentData) -> Result<(), StateError> {
    let Some(device) = state.device else {
        return Err(StateError::Unavailable);
    };
    let generation = state.generation.wrapping_add(1).max(1);
    let target_slot = state.active_slot ^ 1;
    let mut encoded = [0_u8; SLOT_LEN];
    encode_slot(&candidate, generation, &mut encoded);
    let base = slot_lba(target_slot);
    // Payload tail first, header sector last. Until the final sector lands the
    // target cannot validate as the new generation, which strengthens the
    // dual-slot guarantee across power loss.
    for write_index in 0..SLOT_SECTORS {
        let sector_index = (write_index + 1) % SLOT_SECTORS;
        let mut sector = [0_u8; storage::SECTOR_SIZE];
        let start = sector_index * storage::SECTOR_SIZE;
        sector.copy_from_slice(&encoded[start..start + storage::SECTOR_SIZE]);
        device
            .write_sector(base + sector_index as u32, &sector)
            .map_err(StateError::Disk)?;
    }

    let verified = read_slot(device, target_slot).map_err(|error| match error {
        StateError::Disk(error) => StateError::Disk(error),
        _ => StateError::VerificationFailed,
    })?;
    if verified.generation != generation || verified.data != candidate {
        return Err(StateError::VerificationFailed);
    }

    state.data = candidate;
    state.generation = generation;
    state.active_slot = target_slot;
    state.loaded = true;
    slog!(
        "HEXA_STATE_COMMIT generation={} slot={}\r\n",
        generation,
        if target_slot == 0 { "A" } else { "B" }
    );
    Ok(())
}

fn read_slot(device: storage::Device, slot: u8) -> Result<DecodedSlot, StateError> {
    let mut encoded = [0_u8; SLOT_LEN];
    let base = slot_lba(slot);
    for sector_index in 0..SLOT_SECTORS {
        let mut sector = [0_u8; storage::SECTOR_SIZE];
        device
            .read_sector(base + sector_index as u32, &mut sector)
            .map_err(StateError::Disk)?;
        let start = sector_index * storage::SECTOR_SIZE;
        encoded[start..start + storage::SECTOR_SIZE].copy_from_slice(&sector);
    }
    decode_slot(&encoded, slot).ok_or(StateError::VerificationFailed)
}

const fn slot_lba(slot: u8) -> u32 {
    if slot == 0 {
        SLOT_A_LBA
    } else {
        SLOT_B_LBA
    }
}

fn newest_slot(first: Option<DecodedSlot>, second: Option<DecodedSlot>) -> Option<DecodedSlot> {
    match (first, second) {
        (Some(first), Some(second)) => {
            if generation_is_newer(second.generation, first.generation) {
                Some(second)
            } else {
                Some(first)
            }
        }
        (Some(slot), None) | (None, Some(slot)) => Some(slot),
        (None, None) => None,
    }
}

fn generation_is_newer(candidate: u64, current: u64) -> bool {
    candidate != current && candidate.wrapping_sub(current) < (1_u64 << 63)
}

fn encode_slot(data: &PersistentData, generation: u64, output: &mut [u8; SLOT_LEN]) {
    output.fill(0);
    output[..8].copy_from_slice(&MAGIC);
    put_u16(output, 8, FORMAT_VERSION);
    put_u16(output, 10, HEADER_LEN as u16);
    put_u32(output, 12, PAYLOAD_LEN as u32);
    put_u64(output, 16, generation);

    let payload = &mut output[HEADER_LEN..HEADER_LEN + PAYLOAD_LEN];
    if data.accounts_initialized {
        payload[0] |= FLAG_ACCOUNTS_INITIALIZED;
    }
    encode_preferences(data.preferences, &mut payload[32..64]);
    for (index, account) in data.accounts.iter().copied().enumerate() {
        let start = ACCOUNT_RECORDS_OFFSET + index * ACCOUNT_RECORD_LEN;
        encode_account(account, &mut payload[start..start + ACCOUNT_RECORD_LEN]);
    }
    let payload_crc = crc32(payload);
    put_u32(output, 24, payload_crc);
    let header_crc = crc32(&output[..28]);
    put_u32(output, 28, header_crc);
}

fn decode_slot(input: &[u8; SLOT_LEN], slot: u8) -> Option<DecodedSlot> {
    if input[..8] != MAGIC
        || get_u16(input, 8) != FORMAT_VERSION
        || get_u16(input, 10) as usize != HEADER_LEN
        || get_u32(input, 12) as usize != PAYLOAD_LEN
        || crc32(&input[..28]) != get_u32(input, 28)
    {
        return None;
    }
    let payload = &input[HEADER_LEN..HEADER_LEN + PAYLOAD_LEN];
    if crc32(payload) != get_u32(input, 24) {
        return None;
    }
    let mut data = PersistentData::new();
    data.accounts_initialized = payload[0] & FLAG_ACCOUNTS_INITIALIZED != 0;
    data.preferences = decode_preferences(&payload[32..64]);
    for index in 0..MAX_STORED_ACCOUNTS {
        let start = ACCOUNT_RECORDS_OFFSET + index * ACCOUNT_RECORD_LEN;
        data.accounts[index] = decode_account(&payload[start..start + ACCOUNT_RECORD_LEN])?;
    }
    Some(DecodedSlot {
        data,
        generation: get_u64(input, 16),
        slot,
    })
}

fn encode_preferences(value: PersistentPreferences, output: &mut [u8]) {
    output.fill(0);
    output[0] = value.display_mode;
    output[1] = value.theme;
    output[2] = value.wallpaper;
    output[3] = value.cursor_theme;
    output[4] = value.accent;
    output[5] = value.backdrop;
    output[6] = value.pointer_speed;
    output[7..9].copy_from_slice(&value.flags.to_le_bytes());
    output[DISPLAY_TIMING_TAG_OFFSET..REFRESH_RATE_OFFSET].copy_from_slice(&DISPLAY_TIMING_TAG);
    output[REFRESH_RATE_OFFSET] = value.refresh_rate.persisted_id();
    output[VSYNC_OFFSET] = if value.vsync {
        VSYNC_ENABLED_ID
    } else {
        VSYNC_DISABLED_ID
    };
    output[PREFERENCES_RESERVED_OFFSET..32].copy_from_slice(&value.reserved);
}

fn decode_preferences(input: &[u8]) -> PersistentPreferences {
    let has_display_timing =
        input[DISPLAY_TIMING_TAG_OFFSET..REFRESH_RATE_OFFSET] == DISPLAY_TIMING_TAG;
    let refresh_rate = if has_display_timing {
        RefreshRate::from_persisted_id(input[REFRESH_RATE_OFFSET]).unwrap_or(RefreshRate::DEFAULT)
    } else {
        RefreshRate::DEFAULT
    };
    let vsync = if has_display_timing {
        match input[VSYNC_OFFSET] {
            VSYNC_DISABLED_ID => false,
            VSYNC_ENABLED_ID => true,
            _ => true,
        }
    } else {
        true
    };
    sanitize_preferences(PersistentPreferences {
        display_mode: input[0],
        theme: input[1],
        wallpaper: input[2],
        cursor_theme: input[3],
        accent: input[4],
        backdrop: input[5],
        pointer_speed: input[6],
        flags: u16::from_le_bytes([input[7], input[8]]),
        refresh_rate,
        vsync,
        reserved: {
            let mut reserved = [0_u8; 18];
            reserved.copy_from_slice(&input[PREFERENCES_RESERVED_OFFSET..32]);
            reserved
        },
    })
}

fn sanitize_preferences(mut value: PersistentPreferences) -> PersistentPreferences {
    value.display_mode = match value.display_mode {
        0..=2 => value.display_mode,
        _ => 0,
    };
    value.theme = match value.theme {
        0..=5 => value.theme,
        _ => 0,
    };
    value.wallpaper = match value.wallpaper {
        0..=6 => value.wallpaper,
        _ => 0,
    };
    value.cursor_theme = match value.cursor_theme {
        0..=3 => value.cursor_theme,
        _ => 0,
    };
    value.accent = match value.accent {
        0..=3 => value.accent,
        _ => 0,
    };
    value.backdrop = match value.backdrop {
        0..=2 => value.backdrop,
        _ => 0,
    };
    value.pointer_speed = match value.pointer_speed {
        1..=3 => value.pointer_speed,
        _ => 1,
    };
    value.flags &= PREF_PURE_BLACK_APPS
        | PREF_ROUNDED_CONTROLS
        | PREF_TASKBAR_VISIBLE
        | PREF_STATUS_VISIBLE
        | PREF_WINDOW_BORDERS
        | PREF_HIGH_CONTRAST
        | PREF_NETWORK_ENABLED
        | PREF_BLUETOOTH_ENABLED
        | PREF_WIFI_ENABLED
        | PREF_WINDOW_SHADOWS
        | PREF_WALLPAPER_EFFECTS
        | PREF_RESPONSIVE_PRESENTATION;
    value
}

fn encode_account(account: StoredAccount, output: &mut [u8]) {
    output.fill(0);
    output[0] = u8::from(account.occupied);
    output[1] = account.authority;
    output[2] = account.name_len;
    output[4..28].copy_from_slice(&account.name);
    output[28..44].copy_from_slice(&account.password_salt);
    output[44..76].copy_from_slice(&account.password_hash);
    output[76..80].copy_from_slice(&account.kdf_rounds.to_le_bytes());
}

fn decode_account(input: &[u8]) -> Option<StoredAccount> {
    let occupied = input[0] != 0;
    let authority = input[1];
    let name_len = input[2];
    if input[0] > 1
        || authority > 2
        || name_len as usize > ACCOUNT_NAME_CAPACITY
        || (occupied && (name_len == 0 || get_u32(input, 76) == 0))
    {
        return None;
    }
    let mut account = StoredAccount::EMPTY;
    account.occupied = occupied;
    account.authority = authority;
    account.name_len = name_len;
    account.name.copy_from_slice(&input[4..28]);
    account.password_salt.copy_from_slice(&input[28..44]);
    account.password_hash.copy_from_slice(&input[44..76]);
    account.kdf_rounds = get_u32(input, 76);
    Some(account)
}

fn crc32(input: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in input {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320_u32 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
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

fn get_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([input[offset], input[offset + 1]])
}

fn get_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
    ])
}

fn get_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        input[offset],
        input[offset + 1],
        input[offset + 2],
        input[offset + 3],
        input[offset + 4],
        input[offset + 5],
        input[offset + 6],
        input[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refresh_slot_checksums(encoded: &mut [u8; SLOT_LEN]) {
        let payload_crc = crc32(&encoded[HEADER_LEN..HEADER_LEN + PAYLOAD_LEN]);
        put_u32(encoded, 24, payload_crc);
        let header_crc = crc32(&encoded[..28]);
        put_u32(encoded, 28, header_crc);
    }

    #[test]
    fn crc_matches_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn fresh_preferences_choose_the_lowest_safe_display_settings() {
        let preferences = PersistentPreferences::new();
        assert_eq!(preferences.display_mode, 0);
        assert_eq!(preferences.refresh_rate, RefreshRate::Hz60);
        assert!(preferences.vsync);
        assert_eq!(preferences.theme, 0);
        assert_eq!(preferences.wallpaper, 0);
        assert_eq!(preferences.cursor_theme, 0);
        assert_eq!(preferences.accent, 0);
        assert_eq!(preferences.backdrop, 0);
        assert_eq!(preferences.pointer_speed, 1);
        assert_eq!(preferences.flags & PREF_WINDOW_SHADOWS, 0);
        assert_eq!(preferences.flags & PREF_WALLPAPER_EFFECTS, 0);
        assert_eq!(preferences.flags & PREF_RESPONSIVE_PRESENTATION, 0);
    }

    #[test]
    fn performance_preference_flags_round_trip_and_unknown_bits_are_removed() {
        let mut data = PersistentData::new();
        data.preferences.flags =
            PREF_WINDOW_SHADOWS | PREF_WALLPAPER_EFFECTS | PREF_RESPONSIVE_PRESENTATION | 0xF000;
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 14, &mut encoded);

        let decoded = decode_slot(&encoded, 1).unwrap();
        assert_eq!(
            decoded.data.preferences.flags,
            PREF_WINDOW_SHADOWS | PREF_WALLPAPER_EFFECTS | PREF_RESPONSIVE_PRESENTATION
        );
    }

    #[test]
    fn slot_round_trip_preserves_accounts_and_preferences() {
        let mut data = PersistentData::new();
        data.accounts_initialized = true;
        data.preferences.theme = 3;
        data.preferences.wallpaper = 2;
        data.preferences.refresh_rate = RefreshRate::Hz144;
        data.preferences.vsync = false;
        data.accounts[0].occupied = true;
        data.accounts[0].name_len = 5;
        data.accounts[0].name[..5].copy_from_slice(b"alice");
        data.accounts[0].authority = 1;
        data.accounts[0].kdf_rounds = 25_000;
        data.accounts[0].password_salt = [0xA5; 16];
        data.accounts[0].password_hash = [0x5A; 32];

        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 42, &mut encoded);
        let decoded = decode_slot(&encoded, 1).unwrap();
        assert_eq!(decoded.generation, 42);
        assert_eq!(decoded.slot, 1);
        assert_eq!(decoded.data, data);
    }

    #[test]
    fn refresh_rate_wire_ids_are_stable() {
        assert_eq!(RefreshRate::Hz60.persisted_id(), 0);
        assert_eq!(RefreshRate::Hz75.persisted_id(), 1);
        assert_eq!(RefreshRate::Hz120.persisted_id(), 2);
        assert_eq!(RefreshRate::Hz144.persisted_id(), 3);
        assert_eq!(RefreshRate::Hz60.hz(), 60);
        assert_eq!(RefreshRate::Hz75.hz(), 75);
        assert_eq!(RefreshRate::Hz120.hz(), 120);
        assert_eq!(RefreshRate::Hz144.hz(), 144);
    }

    #[test]
    fn display_timing_wire_values_are_stable() {
        let mut data = PersistentData::new();
        data.preferences.refresh_rate = RefreshRate::Hz75;
        data.preferences.vsync = false;
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 9, &mut encoded);

        let preferences_start = HEADER_LEN + 32;
        assert_eq!(
            &encoded[preferences_start + DISPLAY_TIMING_TAG_OFFSET
                ..preferences_start + REFRESH_RATE_OFFSET],
            &DISPLAY_TIMING_TAG
        );
        assert_eq!(
            encoded[preferences_start + REFRESH_RATE_OFFSET],
            RefreshRate::Hz75.persisted_id()
        );
        assert_eq!(encoded[preferences_start + VSYNC_OFFSET], VSYNC_DISABLED_ID);

        data.preferences.vsync = true;
        encode_slot(&data, 10, &mut encoded);
        assert_eq!(encoded[preferences_start + VSYNC_OFFSET], VSYNC_ENABLED_ID);
    }

    #[test]
    fn legacy_expost03_preferences_use_safe_display_timing_defaults() {
        let mut data = PersistentData::new();
        data.preferences.refresh_rate = RefreshRate::Hz144;
        data.preferences.vsync = false;
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 11, &mut encoded);

        // Recreate the reserved zero bytes written by EXPOST03 before the
        // tagged timing extension existed, while retaining valid slot CRCs.
        let preferences_start = HEADER_LEN + 32;
        encoded[preferences_start + DISPLAY_TIMING_TAG_OFFSET
            ..preferences_start + PREFERENCES_RESERVED_OFFSET]
            .fill(0);
        refresh_slot_checksums(&mut encoded);

        let decoded = decode_slot(&encoded, 0).unwrap();
        assert_eq!(decoded.data.preferences.refresh_rate, RefreshRate::Hz60);
        assert!(decoded.data.preferences.vsync);
    }

    #[test]
    fn invalid_checksummed_display_timing_values_fall_back_safely() {
        let mut data = PersistentData::new();
        data.preferences.refresh_rate = RefreshRate::Hz144;
        data.preferences.vsync = false;
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 12, &mut encoded);

        let preferences_start = HEADER_LEN + 32;
        encoded[preferences_start + REFRESH_RATE_OFFSET] = 0xFF;
        encoded[preferences_start + VSYNC_OFFSET] = 0xFF;
        refresh_slot_checksums(&mut encoded);

        let decoded = decode_slot(&encoded, 1).unwrap();
        assert_eq!(decoded.data.preferences.refresh_rate, RefreshRate::Hz60);
        assert!(decoded.data.preferences.vsync);
    }

    #[test]
    fn invalid_checksummed_preference_ids_fall_back_to_lowest_choices() {
        let data = PersistentData::new();
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 13, &mut encoded);

        let preferences_start = HEADER_LEN + 32;
        encoded[preferences_start..preferences_start + 7].fill(0xFF);
        refresh_slot_checksums(&mut encoded);

        let decoded = decode_slot(&encoded, 0).unwrap();
        let preferences = decoded.data.preferences;
        assert_eq!(preferences.display_mode, 0);
        assert_eq!(preferences.theme, 0);
        assert_eq!(preferences.wallpaper, 0);
        assert_eq!(preferences.cursor_theme, 0);
        assert_eq!(preferences.accent, 0);
        assert_eq!(preferences.backdrop, 0);
        assert_eq!(preferences.pointer_speed, 1);
    }

    #[test]
    fn payload_corruption_invalidates_only_that_slot() {
        let data = PersistentData::new();
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 7, &mut encoded);
        encoded[HEADER_LEN + 91] ^= 0x80;
        assert!(decode_slot(&encoded, 0).is_none());
    }

    #[test]
    fn newest_valid_generation_wins_across_wrap() {
        let data = PersistentData::new();
        let old = DecodedSlot {
            data,
            generation: u64::MAX,
            slot: 0,
        };
        let new = DecodedSlot {
            data,
            generation: 1,
            slot: 1,
        };
        assert_eq!(newest_slot(Some(old), Some(new)).unwrap().slot, 1);
    }

    #[test]
    fn serialized_state_never_contains_plaintext_password() {
        let password = b"correct-horse";
        let salt = [1_u8; 16];
        let mut data = PersistentData::new();
        data.accounts_initialized = true;
        data.accounts[0] = StoredAccount {
            name: [0; ACCOUNT_NAME_CAPACITY],
            name_len: 5,
            password_salt: salt,
            password_hash: crate::crypto::password_hash(password, &salt, 2),
            kdf_rounds: 2,
            authority: 1,
            occupied: true,
        };
        data.accounts[0].name[..5].copy_from_slice(b"alice");
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 1, &mut encoded);
        assert!(!encoded
            .windows(password.len())
            .any(|window| window == password));
    }
}
