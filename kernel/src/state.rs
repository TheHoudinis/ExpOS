//! Account/settings schema adapter for the ExpFS system database.
//!
//! Native commits use typed records in the CFC's transactional ExpFS snapshot.
//! The older EXPOST03 dual-slot decoder remains read-only for migration.

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

// The remaining 18 bytes in the preference record carry a tagged, compact
// customization extension. Keeping it inside the old reservation preserves
// the EXPOST03 slot format and lets older disks load without migration.
const CUSTOMIZATION_TAG: [u8; 3] = *b"CX1";
const CUSTOMIZATION_TAG_OFFSET: usize = PREFERENCES_RESERVED_OFFSET;
const CUSTOMIZATION_DATA_OFFSET: usize = CUSTOMIZATION_TAG_OFFSET + CUSTOMIZATION_TAG.len();
const FONT_FACE_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET;
const FONT_WEIGHT_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 1;
const WINDOW_CORNER_RADIUS_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 2;
const WINDOW_BORDER_WIDTH_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 3;
const TITLEBAR_DENSITY_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 4;
const WINDOW_LAYOUT_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 5;
const TASKBAR_SIZE_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 6;
const WINDOW_OFFSCREEN_ALLOWANCE_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 7;
const WINDOW_SNAP_DISTANCE_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 8;
const MENU_DENSITY_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 9;
const ANIMATION_LEVEL_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 10;
const UI_SCALE_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 11;
const SCROLL_SPEED_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 12;
const CUSTOMIZATION_FLAGS_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 13;
const WINDOW_OPACITY_OFFSET: usize = CUSTOMIZATION_DATA_OFFSET + 14;

const TASKBAR_PLACEMENT_SHIFT: u8 = 0;
const TASKBAR_ALIGNMENT_SHIFT: u8 = 2;
const FOCUS_POLICY_SHIFT: u8 = 4;
const TWO_BIT_FIELD_MASK: u8 = 0b11;

const CUSTOM_TASKBAR_AUTOHIDE: u8 = 1 << 0;
const CUSTOM_TASKBAR_TRANSLUCENT: u8 = 1 << 1;
const CUSTOM_TASKBAR_LABELS: u8 = 1 << 2;
const CUSTOM_CLOCK_SECONDS: u8 = 1 << 3;
const CUSTOM_WINDOW_SNAP: u8 = 1 << 4;
const CUSTOM_TOOLTIPS: u8 = 1 << 5;
const CUSTOM_CURSOR_SHADOW: u8 = 1 << 6;
const CUSTOM_NOTIFICATION_ANIMATIONS: u8 = 1 << 7;

/// Concrete values for every numeric customization ID. Indexes are stable
/// persisted IDs and must not be reordered after release. The baseline lives
/// at index zero except for regular font weight, whose framebuffer wire ID is
/// intentionally one.
pub const FONT_FACE_NAMES: [&str; 5] = ["System", "Rounded", "Serif", "Compact", "Slanted"];
pub const FONT_WEIGHT_NAMES: [&str; 3] = ["Light", "Regular", "Bold"];
pub const WINDOW_CORNER_RADII_PX: [u8; 9] = [0, 2, 4, 6, 8, 10, 12, 16, 20];
pub const WINDOW_BORDER_WIDTHS_PX: [u8; 7] = [0, 1, 2, 3, 4, 6, 8];
pub const TITLEBAR_HEIGHTS_PX: [u8; 5] = [24, 28, 32, 36, 40];
pub const TASKBAR_PLACEMENT_NAMES: [&str; 4] = ["Bottom", "Top", "Left", "Right"];
pub const TASKBAR_SIZES_PX: [u8; 9] = [28, 32, 36, 40, 44, 48, 52, 60, 72];
pub const TASKBAR_ALIGNMENT_NAMES: [&str; 3] = ["Start", "Center", "End"];
pub const WINDOW_OFFSCREEN_ALLOWANCES_PX: [u16; 9] = [0, 8, 16, 32, 64, 96, 128, 192, 256];
pub const WINDOW_SNAP_DISTANCES_PX: [u8; 9] = [0, 4, 8, 12, 16, 24, 32, 48, 64];
pub const MENU_ROW_HEIGHTS_PX: [u8; 5] = [20, 24, 28, 32, 38];
pub const ANIMATION_DURATIONS_MS: [u16; 5] = [0, 80, 120, 180, 260];
pub const UI_SCALE_PERCENT: [u16; 7] = [100, 80, 90, 110, 125, 150, 200];
pub const SCROLL_STEPS: [u8; 7] = [3, 1, 2, 4, 6, 8, 12];
pub const FOCUS_POLICY_NAMES: [&str; 3] = ["Click", "Sloppy", "Pointer"];
pub const WINDOW_OPACITY_ALPHA: [u8; 6] = [255, 244, 232, 216, 192, 160];

/// Number of stable values accepted for each persisted customization control.
pub const FONT_FACE_CHOICES: u8 = FONT_FACE_NAMES.len() as u8;
pub const FONT_WEIGHT_CHOICES: u8 = FONT_WEIGHT_NAMES.len() as u8;
/// Persisted weight ID used by a new or pre-extension state record.
pub const DEFAULT_FONT_WEIGHT: u8 = 1;
pub const WINDOW_CORNER_RADIUS_CHOICES: u8 = WINDOW_CORNER_RADII_PX.len() as u8;
pub const WINDOW_BORDER_WIDTH_CHOICES: u8 = WINDOW_BORDER_WIDTHS_PX.len() as u8;
pub const TITLEBAR_DENSITY_CHOICES: u8 = TITLEBAR_HEIGHTS_PX.len() as u8;
pub const TASKBAR_PLACEMENT_CHOICES: u8 = TASKBAR_PLACEMENT_NAMES.len() as u8;
pub const TASKBAR_SIZE_CHOICES: u8 = TASKBAR_SIZES_PX.len() as u8;
pub const TASKBAR_ALIGNMENT_CHOICES: u8 = TASKBAR_ALIGNMENT_NAMES.len() as u8;
pub const WINDOW_OFFSCREEN_ALLOWANCE_CHOICES: u8 = WINDOW_OFFSCREEN_ALLOWANCES_PX.len() as u8;
pub const WINDOW_SNAP_DISTANCE_CHOICES: u8 = WINDOW_SNAP_DISTANCES_PX.len() as u8;
pub const MENU_DENSITY_CHOICES: u8 = MENU_ROW_HEIGHTS_PX.len() as u8;
pub const ANIMATION_LEVEL_CHOICES: u8 = ANIMATION_DURATIONS_MS.len() as u8;
pub const UI_SCALE_CHOICES: u8 = UI_SCALE_PERCENT.len() as u8;
pub const SCROLL_SPEED_CHOICES: u8 = SCROLL_STEPS.len() as u8;
pub const FOCUS_POLICY_CHOICES: u8 = FOCUS_POLICY_NAMES.len() as u8;
pub const WINDOW_OPACITY_CHOICES: u8 = WINDOW_OPACITY_ALPHA.len() as u8;
pub const CUSTOMIZATION_BOOLEAN_CONTROLS: usize = 8;

/// Total number of distinct selectable values represented by the customization
/// extension. This counts each value of a selector and both states of every
/// boolean control; it deliberately excludes the older display/theme fields.
pub const CUSTOMIZATION_SELECTABLE_VALUES: usize = FONT_FACE_CHOICES as usize
    + FONT_WEIGHT_CHOICES as usize
    + WINDOW_CORNER_RADIUS_CHOICES as usize
    + WINDOW_BORDER_WIDTH_CHOICES as usize
    + TITLEBAR_DENSITY_CHOICES as usize
    + TASKBAR_PLACEMENT_CHOICES as usize
    + TASKBAR_SIZE_CHOICES as usize
    + TASKBAR_ALIGNMENT_CHOICES as usize
    + WINDOW_OFFSCREEN_ALLOWANCE_CHOICES as usize
    + WINDOW_SNAP_DISTANCE_CHOICES as usize
    + MENU_DENSITY_CHOICES as usize
    + ANIMATION_LEVEL_CHOICES as usize
    + UI_SCALE_CHOICES as usize
    + SCROLL_SPEED_CHOICES as usize
    + FOCUS_POLICY_CHOICES as usize
    + WINDOW_OPACITY_CHOICES as usize
    + CUSTOMIZATION_BOOLEAN_CONTROLS * 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateError {
    Disk(storage::StorageError),
    ExpFs(crate::expfs_store::StoreError),
    VerificationFailed,
}

impl StateError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Disk(_) => "persistent state disk I/O failed",
            Self::ExpFs(error) => error.message(),
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
    /// Stable face ID. Zero is the normal built-in face.
    pub font_face: u8,
    /// Stable weight ID: 0=Light, 1=Regular, 2=Bold.
    pub font_weight: u8,
    /// Window decoration geometry IDs; zero selects the cheapest square/thin
    /// baseline in each case.
    pub window_corner_radius: u8,
    pub window_border_width: u8,
    pub titlebar_density: u8,
    /// 0=bottom, 1=top, 2=left, 3=right.
    pub taskbar_placement: u8,
    pub taskbar_size: u8,
    /// 0=start, 1=center, 2=end.
    pub taskbar_alignment: u8,
    pub taskbar_autohide: bool,
    pub taskbar_translucent: bool,
    pub taskbar_labels: bool,
    pub clock_seconds: bool,
    /// How far a movable window may extend beyond a display edge.
    pub window_offscreen_allowance: u8,
    pub window_snap_distance: u8,
    pub window_snap: bool,
    pub menu_density: u8,
    /// Zero disables animation; higher IDs progressively add motion.
    pub animation_level: u8,
    pub ui_scale: u8,
    pub scroll_speed: u8,
    /// 0=click, 1=sloppy, 2=focus-follows-pointer.
    pub focus_policy: u8,
    /// Zero is opaque; higher IDs opt into increasing translucency.
    pub window_opacity: u8,
    pub tooltips: bool,
    pub cursor_shadow: bool,
    pub notification_animations: bool,
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
                | PREF_NETWORK_ENABLED,
            refresh_rate: RefreshRate::DEFAULT,
            vsync: true,
            font_face: 0,
            font_weight: DEFAULT_FONT_WEIGHT,
            window_corner_radius: 0,
            window_border_width: 0,
            titlebar_density: 0,
            taskbar_placement: 0,
            taskbar_size: 0,
            taskbar_alignment: 0,
            taskbar_autohide: false,
            taskbar_translucent: false,
            taskbar_labels: false,
            clock_seconds: false,
            window_offscreen_allowance: 0,
            window_snap_distance: 0,
            window_snap: false,
            menu_density: 0,
            animation_level: 0,
            ui_scale: 0,
            scroll_speed: 0,
            focus_policy: 0,
            window_opacity: 0,
            tooltips: false,
            cursor_shadow: false,
            notification_animations: false,
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
    expfs_source: bool,
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
            expfs_source: false,
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
            slog!("EXPOS_STATE_VOLATILE reason=disk-too-small\r\n");
            return;
        }
        Err(error) => {
            println!("[warn] persistent state unavailable: {:?}", error);
            slog!("EXPOS_STATE_VOLATILE error={:?}\r\n", error);
            return;
        }
    };

    let first = read_slot(device, 0).ok();
    let second = read_slot(device, 1).ok();
    let selected = newest_slot(first, second);
    let expfs_data = decode_expfs_records(crate::expfs_store::system_records());
    let mut state = STATE.lock();
    state.device = Some(device);
    if let Some(data) = expfs_data {
        state.data = data;
        state.generation = crate::expfs_store::generation().unwrap_or(1);
        state.loaded = true;
        state.expfs_source = true;
        println!(
            "[ok] accounts/settings loaded from ExpFS generation {}",
            state.generation
        );
        slog!(
            "EXPOS_STATE_READY generation={} slot=expfs loaded=true\r\n",
            state.generation
        );
    } else if let Some(slot) = selected {
        state.data = slot.data;
        state.generation = slot.generation;
        state.active_slot = slot.slot;
        state.loaded = true;
        state.expfs_source = false;
        println!(
            "[ok] persistent state generation {} loaded from slot {}",
            slot.generation,
            if slot.slot == 0 { "A" } else { "B" }
        );
        slog!(
            "EXPOS_STATE_READY generation={} slot={} loaded=true\r\n",
            slot.generation,
            if slot.slot == 0 { "A" } else { "B" }
        );
    } else {
        println!("[ok] blank state disk detected; defaults will be journaled");
        slog!("EXPOS_STATE_READY generation=0 slot=none loaded=false\r\n");
    }
}

pub fn persistent_available() -> bool {
    crate::expfs_store::available()
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
        } else if state.expfs_source {
            "expfs"
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
    println!(
        "customization: face={} weight={} corners={} border={} titlebar={} opacity={}",
        preferences.font_face,
        preferences.font_weight,
        preferences.window_corner_radius,
        preferences.window_border_width,
        preferences.titlebar_density,
        preferences.window_opacity
    );
    println!(
        "taskbar: placement={} size={} alignment={} autohide={} translucent={} labels={}",
        preferences.taskbar_placement,
        preferences.taskbar_size,
        preferences.taskbar_alignment,
        preferences.taskbar_autohide,
        preferences.taskbar_translucent,
        preferences.taskbar_labels
    );
    println!(
        "interaction: offscreen={} snap={} snap-distance={} menu={} animation={} scale={} scroll={} focus={}",
        preferences.window_offscreen_allowance,
        preferences.window_snap,
        preferences.window_snap_distance,
        preferences.menu_density,
        preferences.animation_level,
        preferences.ui_scale,
        preferences.scroll_speed,
        preferences.focus_policy
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
    let (settings, accounts) = encode_expfs_records(&candidate);
    let generation =
        crate::expfs_store::commit_system_records(settings, accounts).map_err(StateError::ExpFs)?;
    state.data = candidate;
    state.generation = generation;
    state.loaded = true;
    state.expfs_source = true;
    slog!(
        "EXPOS_STATE_COMMIT generation={} slot=expfs\r\n",
        generation
    );
    Ok(())
}

fn encode_expfs_records(
    data: &PersistentData,
) -> (
    [u8; crate::expfs_store::SETTINGS_RECORD_CAPACITY],
    Option<[u8; crate::expfs_store::ACCOUNTS_RECORD_CAPACITY]>,
) {
    let mut settings = [0_u8; crate::expfs_store::SETTINGS_RECORD_CAPACITY];
    encode_preferences(data.preferences, &mut settings);
    let accounts = data.accounts_initialized.then(|| {
        let mut encoded = [0_u8; crate::expfs_store::ACCOUNTS_RECORD_CAPACITY];
        for (index, account) in data.accounts.iter().copied().enumerate() {
            let start = index * ACCOUNT_RECORD_LEN;
            encode_account(account, &mut encoded[start..start + ACCOUNT_RECORD_LEN]);
        }
        encoded
    });
    (settings, accounts)
}

fn decode_expfs_records(
    records: (
        Option<[u8; crate::expfs_store::SETTINGS_RECORD_CAPACITY]>,
        Option<[u8; crate::expfs_store::ACCOUNTS_RECORD_CAPACITY]>,
    ),
) -> Option<PersistentData> {
    let (settings, accounts) = records;
    if settings.is_none() && accounts.is_none() {
        return None;
    }
    let mut data = PersistentData::new();
    if let Some(settings) = settings {
        data.preferences = decode_preferences(&settings);
    }
    if let Some(accounts) = accounts {
        for index in 0..MAX_STORED_ACCOUNTS {
            let start = index * ACCOUNT_RECORD_LEN;
            data.accounts[index] = decode_account(&accounts[start..start + ACCOUNT_RECORD_LEN])?;
        }
        data.accounts_initialized = true;
    }
    Some(data)
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

#[cfg(test)]
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
    output[CUSTOMIZATION_TAG_OFFSET..CUSTOMIZATION_DATA_OFFSET].copy_from_slice(&CUSTOMIZATION_TAG);
    output[FONT_FACE_OFFSET] = value.font_face;
    output[FONT_WEIGHT_OFFSET] = value.font_weight;
    output[WINDOW_CORNER_RADIUS_OFFSET] = value.window_corner_radius;
    output[WINDOW_BORDER_WIDTH_OFFSET] = value.window_border_width;
    output[TITLEBAR_DENSITY_OFFSET] = value.titlebar_density;
    output[WINDOW_LAYOUT_OFFSET] = ((value.taskbar_placement & TWO_BIT_FIELD_MASK)
        << TASKBAR_PLACEMENT_SHIFT)
        | ((value.taskbar_alignment & TWO_BIT_FIELD_MASK) << TASKBAR_ALIGNMENT_SHIFT)
        | ((value.focus_policy & TWO_BIT_FIELD_MASK) << FOCUS_POLICY_SHIFT);
    output[TASKBAR_SIZE_OFFSET] = value.taskbar_size;
    output[WINDOW_OFFSCREEN_ALLOWANCE_OFFSET] = value.window_offscreen_allowance;
    output[WINDOW_SNAP_DISTANCE_OFFSET] = value.window_snap_distance;
    output[MENU_DENSITY_OFFSET] = value.menu_density;
    output[ANIMATION_LEVEL_OFFSET] = value.animation_level;
    output[UI_SCALE_OFFSET] = value.ui_scale;
    output[SCROLL_SPEED_OFFSET] = value.scroll_speed;
    output[CUSTOMIZATION_FLAGS_OFFSET] = customization_flags(value);
    output[WINDOW_OPACITY_OFFSET] = value.window_opacity;
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
    let defaults = PersistentPreferences::new();
    let has_customization =
        input[CUSTOMIZATION_TAG_OFFSET..CUSTOMIZATION_DATA_OFFSET] == CUSTOMIZATION_TAG;
    let window_layout = if has_customization {
        input[WINDOW_LAYOUT_OFFSET]
    } else {
        0
    };
    let customization_flags = if has_customization {
        input[CUSTOMIZATION_FLAGS_OFFSET]
    } else {
        0
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
        font_face: customization_value(
            input,
            has_customization,
            FONT_FACE_OFFSET,
            defaults.font_face,
        ),
        font_weight: customization_value(
            input,
            has_customization,
            FONT_WEIGHT_OFFSET,
            defaults.font_weight,
        ),
        window_corner_radius: customization_value(
            input,
            has_customization,
            WINDOW_CORNER_RADIUS_OFFSET,
            defaults.window_corner_radius,
        ),
        window_border_width: customization_value(
            input,
            has_customization,
            WINDOW_BORDER_WIDTH_OFFSET,
            defaults.window_border_width,
        ),
        titlebar_density: customization_value(
            input,
            has_customization,
            TITLEBAR_DENSITY_OFFSET,
            defaults.titlebar_density,
        ),
        taskbar_placement: (window_layout >> TASKBAR_PLACEMENT_SHIFT) & TWO_BIT_FIELD_MASK,
        taskbar_size: customization_value(
            input,
            has_customization,
            TASKBAR_SIZE_OFFSET,
            defaults.taskbar_size,
        ),
        taskbar_alignment: (window_layout >> TASKBAR_ALIGNMENT_SHIFT) & TWO_BIT_FIELD_MASK,
        taskbar_autohide: customization_flags & CUSTOM_TASKBAR_AUTOHIDE != 0,
        taskbar_translucent: customization_flags & CUSTOM_TASKBAR_TRANSLUCENT != 0,
        taskbar_labels: customization_flags & CUSTOM_TASKBAR_LABELS != 0,
        clock_seconds: customization_flags & CUSTOM_CLOCK_SECONDS != 0,
        window_offscreen_allowance: customization_value(
            input,
            has_customization,
            WINDOW_OFFSCREEN_ALLOWANCE_OFFSET,
            defaults.window_offscreen_allowance,
        ),
        window_snap_distance: customization_value(
            input,
            has_customization,
            WINDOW_SNAP_DISTANCE_OFFSET,
            defaults.window_snap_distance,
        ),
        window_snap: customization_flags & CUSTOM_WINDOW_SNAP != 0,
        menu_density: customization_value(
            input,
            has_customization,
            MENU_DENSITY_OFFSET,
            defaults.menu_density,
        ),
        animation_level: customization_value(
            input,
            has_customization,
            ANIMATION_LEVEL_OFFSET,
            defaults.animation_level,
        ),
        ui_scale: customization_value(input, has_customization, UI_SCALE_OFFSET, defaults.ui_scale),
        scroll_speed: customization_value(
            input,
            has_customization,
            SCROLL_SPEED_OFFSET,
            defaults.scroll_speed,
        ),
        focus_policy: (window_layout >> FOCUS_POLICY_SHIFT) & TWO_BIT_FIELD_MASK,
        window_opacity: customization_value(
            input,
            has_customization,
            WINDOW_OPACITY_OFFSET,
            defaults.window_opacity,
        ),
        tooltips: customization_flags & CUSTOM_TOOLTIPS != 0,
        cursor_shadow: customization_flags & CUSTOM_CURSOR_SHADOW != 0,
        notification_animations: customization_flags & CUSTOM_NOTIFICATION_ANIMATIONS != 0,
    })
}

fn customization_value(input: &[u8], present: bool, offset: usize, default: u8) -> u8 {
    if present {
        input[offset]
    } else {
        default
    }
}

fn customization_flags(value: PersistentPreferences) -> u8 {
    let mut flags = 0;
    if value.taskbar_autohide {
        flags |= CUSTOM_TASKBAR_AUTOHIDE;
    }
    if value.taskbar_translucent {
        flags |= CUSTOM_TASKBAR_TRANSLUCENT;
    }
    if value.taskbar_labels {
        flags |= CUSTOM_TASKBAR_LABELS;
    }
    if value.clock_seconds {
        flags |= CUSTOM_CLOCK_SECONDS;
    }
    if value.window_snap {
        flags |= CUSTOM_WINDOW_SNAP;
    }
    if value.tooltips {
        flags |= CUSTOM_TOOLTIPS;
    }
    if value.cursor_shadow {
        flags |= CUSTOM_CURSOR_SHADOW;
    }
    if value.notification_animations {
        flags |= CUSTOM_NOTIFICATION_ANIMATIONS;
    }
    flags
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
    value.font_face = sanitize_choice(value.font_face, FONT_FACE_CHOICES);
    value.font_weight =
        sanitize_choice_or(value.font_weight, FONT_WEIGHT_CHOICES, DEFAULT_FONT_WEIGHT);
    value.window_corner_radius =
        sanitize_choice(value.window_corner_radius, WINDOW_CORNER_RADIUS_CHOICES);
    value.window_border_width =
        sanitize_choice(value.window_border_width, WINDOW_BORDER_WIDTH_CHOICES);
    value.titlebar_density = sanitize_choice(value.titlebar_density, TITLEBAR_DENSITY_CHOICES);
    value.taskbar_placement = sanitize_choice(value.taskbar_placement, TASKBAR_PLACEMENT_CHOICES);
    value.taskbar_size = sanitize_choice(value.taskbar_size, TASKBAR_SIZE_CHOICES);
    value.taskbar_alignment = sanitize_choice(value.taskbar_alignment, TASKBAR_ALIGNMENT_CHOICES);
    value.window_offscreen_allowance = sanitize_choice(
        value.window_offscreen_allowance,
        WINDOW_OFFSCREEN_ALLOWANCE_CHOICES,
    );
    value.window_snap_distance =
        sanitize_choice(value.window_snap_distance, WINDOW_SNAP_DISTANCE_CHOICES);
    value.menu_density = sanitize_choice(value.menu_density, MENU_DENSITY_CHOICES);
    value.animation_level = sanitize_choice(value.animation_level, ANIMATION_LEVEL_CHOICES);
    value.ui_scale = sanitize_choice(value.ui_scale, UI_SCALE_CHOICES);
    value.scroll_speed = sanitize_choice(value.scroll_speed, SCROLL_SPEED_CHOICES);
    value.focus_policy = sanitize_choice(value.focus_policy, FOCUS_POLICY_CHOICES);
    value.window_opacity = sanitize_choice(value.window_opacity, WINDOW_OPACITY_CHOICES);
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

const fn sanitize_choice(value: u8, choices: u8) -> u8 {
    if value < choices {
        value
    } else {
        0
    }
}

const fn sanitize_choice_or(value: u8, choices: u8, fallback: u8) -> u8 {
    if value < choices {
        value
    } else {
        fallback
    }
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

#[cfg(test)]
fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
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
        assert_eq!(preferences.flags & PREF_WINDOW_BORDERS, 0);
        assert_eq!(preferences.font_face, 0);
        assert_eq!(preferences.font_weight, DEFAULT_FONT_WEIGHT);
        assert_eq!(preferences.window_corner_radius, 0);
        assert_eq!(preferences.window_border_width, 0);
        assert_eq!(preferences.titlebar_density, 0);
        assert_eq!(preferences.taskbar_placement, 0);
        assert_eq!(preferences.taskbar_size, 0);
        assert_eq!(preferences.taskbar_alignment, 0);
        assert!(!preferences.taskbar_autohide);
        assert!(!preferences.taskbar_translucent);
        assert!(!preferences.taskbar_labels);
        assert!(!preferences.clock_seconds);
        assert_eq!(preferences.window_offscreen_allowance, 0);
        assert_eq!(preferences.window_snap_distance, 0);
        assert!(!preferences.window_snap);
        assert_eq!(preferences.menu_density, 0);
        assert_eq!(preferences.animation_level, 0);
        assert_eq!(preferences.ui_scale, 0);
        assert_eq!(preferences.scroll_speed, 0);
        assert_eq!(preferences.focus_policy, 0);
        assert_eq!(preferences.window_opacity, 0);
        assert!(!preferences.tooltips);
        assert!(!preferences.cursor_shadow);
        assert!(!preferences.notification_animations);
    }

    #[test]
    fn customization_surface_exposes_more_than_one_hundred_real_values() {
        assert_eq!(CUSTOMIZATION_SELECTABLE_VALUES, 112);
        const { assert!(CUSTOMIZATION_SELECTABLE_VALUES >= 100) };
    }

    #[test]
    fn customization_extension_round_trips_every_control() {
        let mut data = PersistentData::new();
        let preferences = &mut data.preferences;
        preferences.font_face = FONT_FACE_CHOICES - 1;
        preferences.font_weight = FONT_WEIGHT_CHOICES - 1;
        preferences.window_corner_radius = WINDOW_CORNER_RADIUS_CHOICES - 1;
        preferences.window_border_width = WINDOW_BORDER_WIDTH_CHOICES - 1;
        preferences.titlebar_density = TITLEBAR_DENSITY_CHOICES - 1;
        preferences.taskbar_placement = TASKBAR_PLACEMENT_CHOICES - 1;
        preferences.taskbar_size = TASKBAR_SIZE_CHOICES - 1;
        preferences.taskbar_alignment = TASKBAR_ALIGNMENT_CHOICES - 1;
        preferences.taskbar_autohide = true;
        preferences.taskbar_translucent = true;
        preferences.taskbar_labels = true;
        preferences.clock_seconds = true;
        preferences.window_offscreen_allowance = WINDOW_OFFSCREEN_ALLOWANCE_CHOICES - 1;
        preferences.window_snap_distance = WINDOW_SNAP_DISTANCE_CHOICES - 1;
        preferences.window_snap = true;
        preferences.menu_density = MENU_DENSITY_CHOICES - 1;
        preferences.animation_level = ANIMATION_LEVEL_CHOICES - 1;
        preferences.ui_scale = UI_SCALE_CHOICES - 1;
        preferences.scroll_speed = SCROLL_SPEED_CHOICES - 1;
        preferences.focus_policy = FOCUS_POLICY_CHOICES - 1;
        preferences.window_opacity = WINDOW_OPACITY_CHOICES - 1;
        preferences.tooltips = true;
        preferences.cursor_shadow = true;
        preferences.notification_animations = true;

        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 15, &mut encoded);

        let preferences_start = HEADER_LEN + 32;
        assert_eq!(
            &encoded[preferences_start + CUSTOMIZATION_TAG_OFFSET
                ..preferences_start + CUSTOMIZATION_DATA_OFFSET],
            &CUSTOMIZATION_TAG
        );
        let decoded = decode_slot(&encoded, 0).unwrap();
        assert_eq!(decoded.data.preferences, data.preferences);
    }

    #[test]
    fn legacy_expost03_preferences_receive_safe_customization_defaults() {
        let mut data = PersistentData::new();
        data.preferences.font_face = FONT_FACE_CHOICES - 1;
        data.preferences.taskbar_placement = TASKBAR_PLACEMENT_CHOICES - 1;
        data.preferences.taskbar_autohide = true;
        data.preferences.window_offscreen_allowance = WINDOW_OFFSCREEN_ALLOWANCE_CHOICES - 1;
        data.preferences.animation_level = ANIMATION_LEVEL_CHOICES - 1;
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 16, &mut encoded);

        let preferences_start = HEADER_LEN + 32;
        encoded[preferences_start + CUSTOMIZATION_TAG_OFFSET..preferences_start + 32].fill(0);
        refresh_slot_checksums(&mut encoded);

        let decoded = decode_slot(&encoded, 1).unwrap();
        assert_eq!(decoded.data.preferences, PersistentPreferences::new());
    }

    #[test]
    fn invalid_customization_ids_fall_back_to_baselines() {
        let data = PersistentData::new();
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&data, 17, &mut encoded);

        let preferences_start = HEADER_LEN + 32;
        for offset in FONT_FACE_OFFSET..=WINDOW_OPACITY_OFFSET {
            encoded[preferences_start + offset] = 0xFF;
        }
        // Retain the extension tag after corrupting all extension data.
        encoded[preferences_start + CUSTOMIZATION_TAG_OFFSET
            ..preferences_start + CUSTOMIZATION_DATA_OFFSET]
            .copy_from_slice(&CUSTOMIZATION_TAG);
        refresh_slot_checksums(&mut encoded);

        let decoded = decode_slot(&encoded, 0).unwrap();
        let preferences = decoded.data.preferences;
        assert_eq!(preferences.font_face, 0);
        assert_eq!(preferences.font_weight, DEFAULT_FONT_WEIGHT);
        assert_eq!(preferences.window_corner_radius, 0);
        assert_eq!(preferences.window_border_width, 0);
        assert_eq!(preferences.titlebar_density, 0);
        // Placement occupies all four possible two-bit values, so 0xff masks
        // to the valid right-edge ID while the other packed fields sanitize.
        assert_eq!(preferences.taskbar_placement, 3);
        assert_eq!(preferences.taskbar_size, 0);
        assert_eq!(preferences.taskbar_alignment, 0);
        assert_eq!(preferences.window_offscreen_allowance, 0);
        assert_eq!(preferences.window_snap_distance, 0);
        assert_eq!(preferences.menu_density, 0);
        assert_eq!(preferences.animation_level, 0);
        assert_eq!(preferences.ui_scale, 0);
        assert_eq!(preferences.scroll_speed, 0);
        assert_eq!(preferences.focus_policy, 0);
        assert_eq!(preferences.window_opacity, 0);
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
