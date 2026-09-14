use crate::{
    display_timing::{FrameDecision, FramePacer, RefreshRate, TimingConfig, VSyncPolicy},
    framebuffer,
    input::{
        Input, InputEvent, PointerEvent, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_SUPER_BROWSER,
        KEY_SUPER_CLOSE, KEY_SUPER_CYCLE, KEY_SUPER_DOWN, KEY_SUPER_FULLSCREEN, KEY_SUPER_LAUNCHER,
        KEY_SUPER_LEFT, KEY_SUPER_RIGHT, KEY_SUPER_TERMINAL, KEY_SUPER_UP, KEY_UP,
    },
    network, radio, slog, state,
};
use framebuffer::color;
use hexa_core::{
    Authority, BrowserText, BufferFormat, BufferHandle, CapabilityBroker, DisplayServer, Document,
    Fin, NodeKind, Operations, Rect, SurfaceRole, TextAlign,
};

pub const DISPLAY_FIN: Fin = Fin::from_u128(0x4449_5350_4C41_5900_0000_0000_0000_0001);
pub const BROWSER_FIN: Fin = Fin::from_u128(0x4252_4F57_5345_5200_0000_0000_0000_0001);
pub const TERMINAL_FIN: Fin = Fin::from_u128(0x5445_524D_494E_414C_0000_0000_0000_0001);
pub const FORMS_FIN: Fin = Fin::from_u128(0x464F_524D_5300_0000_0000_0000_0000_0001);
pub const PACKAGES_FIN: Fin = Fin::from_u128(0x5041_434B_4147_4553_0000_0000_0000_0001);
pub const SETTINGS_FIN: Fin = Fin::from_u128(0x5345_5454_494E_4753_0000_0000_0000_0001);
pub const SYSTEM_FIN: Fin = Fin::from_u128(0x5359_5354_454D_0000_0000_0000_0000_0001);
pub const GAMES_FIN: Fin = Fin::from_u128(0x4741_4D45_5300_0000_0000_0000_0000_0001);
pub const NOTES_FIN: Fin = Fin::from_u128(0x4E4F_5445_5300_0000_0000_0000_0000_0001);

const APP_COUNT: usize = 8;
const TERMINAL_HISTORY: usize = 24;
const TERMINAL_CAPACITY: usize = 96;
const TERMINAL_SCROLLBACK: usize = 40;
const TERMINAL_OUTPUT_CAPACITY: usize = 112;
const NOTES_CAPACITY: usize = 2048;
const CURSOR_WIDTH: usize = 14;
const CURSOR_HEIGHT: usize = 20;
const TASKBAR_HEIGHT: i16 = 48;
const START_X: i16 = 8;
const TASK_ICON_X: i16 = 48;
const TASK_ICON_STEP: i16 = 36;
const LAUNCHER_X: i16 = 8;
const LAUNCHER_WIDTH: u16 = 250;
const LAUNCHER_HEIGHT: u16 = 318;
const STABLE_FIN: Fin = Fin::from_u128(0x4449_4D00_0000_0000_0000_0000_0000_0001);

fn taskbar_y() -> i16 {
    framebuffer::height() as i16 - TASKBAR_HEIGHT
}

fn launcher_y() -> i16 {
    (taskbar_y() - LAUNCHER_HEIGHT as i16 - 8).max(4)
}

fn app_dimensions() -> (u16, u16) {
    let screen_width = framebuffer::width() as u16;
    let work_height = taskbar_y().max(80) as u16;
    let width = if screen_width <= 640 {
        screen_width.saturating_sub(16)
    } else if screen_width <= 1280 {
        screen_width.saturating_sub(96).min(1040)
    } else {
        1280
    };
    let height = if work_height <= 440 {
        work_height.saturating_sub(12)
    } else if work_height <= 680 {
        work_height.saturating_sub(40).min(610)
    } else {
        800
    };
    (width.max(480), height.max(360))
}

fn settings_compact(rect: Rect) -> bool {
    rect.height < 560 || rect.width < 800
}

fn settings_sidebar_width(rect: Rect) -> i16 {
    if settings_compact(rect) {
        158
    } else {
        206
    }
}

fn settings_category_top(rect: Rect) -> i16 {
    if settings_compact(rect) {
        70
    } else {
        92
    }
}

fn settings_category_step(rect: Rect) -> i16 {
    if settings_compact(rect) {
        38
    } else {
        42
    }
}

fn settings_row_top(rect: Rect) -> i16 {
    if settings_compact(rect) {
        88
    } else {
        112
    }
}

fn settings_row_height(rect: Rect) -> i16 {
    if settings_compact(rect) {
        42
    } else {
        54
    }
}

fn settings_row_step(rect: Rect) -> i16 {
    if settings_compact(rect) {
        47
    } else {
        62
    }
}

fn shift_display_mode(mode: framebuffer::DisplayMode, direction: i8) -> framebuffer::DisplayMode {
    let index = mode.persisted() as usize;
    let next = if direction < 0 {
        (index + framebuffer::DisplayMode::ALL.len() - 1) % framebuffer::DisplayMode::ALL.len()
    } else {
        (index + 1) % framebuffer::DisplayMode::ALL.len()
    };
    framebuffer::DisplayMode::ALL[next]
}

fn shift_refresh_rate(rate: state::RefreshRate, direction: i8) -> state::RefreshRate {
    const RATES: [state::RefreshRate; 4] = [
        state::RefreshRate::Hz60,
        state::RefreshRate::Hz75,
        state::RefreshRate::Hz120,
        state::RefreshRate::Hz144,
    ];
    let index = rate.persisted_id() as usize;
    let next = if direction < 0 {
        (index + RATES.len() - 1) % RATES.len()
    } else {
        (index + 1) % RATES.len()
    };
    RATES[next]
}

const fn refresh_rate_label(rate: state::RefreshRate) -> &'static str {
    match rate {
        state::RefreshRate::Hz60 => "60 Hz",
        state::RefreshRate::Hz75 => "75 Hz",
        state::RefreshRate::Hz120 => "120 Hz",
        state::RefreshRate::Hz144 => "144 Hz",
    }
}

const fn timing_refresh_rate(rate: state::RefreshRate) -> RefreshRate {
    match rate {
        state::RefreshRate::Hz60 => RefreshRate::Hz60,
        state::RefreshRate::Hz75 => RefreshRate::Hz75,
        state::RefreshRate::Hz120 => RefreshRate::Hz120,
        state::RefreshRate::Hz144 => RefreshRate::Hz144,
    }
}

fn timing_config(preferences: DesktopPreferences) -> TimingConfig {
    TimingConfig::new(
        timing_refresh_rate(preferences.refresh_rate),
        VSyncPolicy::from_enabled(preferences.vsync),
        crate::hardware::clock_info().tsc_hz,
    )
    .expect("the kernel TSC clock must resolve every supported refresh rate")
}

const HOME: &str = "<style>h1{color:#74bcc7}.card{background:#151c20;border:1px solid #35433f;padding:8px}button{color:#f0f2f0;background:#365c62;padding:6px}</style><title>Home</title><h1>ExpOS Web</h1><p id='status' class='card'>Starting the bounded web engine</p><button id='demo'>Try JavaScript</button><a href='hexa://about'>About</a><a href='hexa://packages'>Packages</a><a href='hexa://system'>System</a><script>document.title='ExpOS Home';document.getElementById('status').textContent='CSS and JavaScript are active';document.getElementById('demo').onclick=function(){document.getElementById('status').textContent='Button handled locally';}</script>";
const ABOUT: &str = "<title>About</title><h1>Browser</h1><p>A bounded native HTML, CSS, and JavaScript document engine.</p><a href='hexa://home'>Home</a>";
const BROWSER_PACKAGES: &str = "<title>Packages</title><h1>Packages</h1><li>Core tools</li><li>Display</li><li>Notes</li><li>Games</li><a href='hexa://home'>Home</a>";
const BROWSER_SYSTEM: &str = "<title>System</title><h1>System</h1><li>480p / 720p / 1080p display</li><li>60 / 75 / 120 / 144 Hz compositor pacing</li><li>Keyboard and mouse</li><li>RTL8139 network</li><a href='hexa://home'>Home</a>";
const NETWORK_BLOCKED: &str = "<title>Offline</title><h1>Offline</h1><p>The address could not be loaded.</p><a href='hexa://home'>Home</a>";
const NETWORK_ERROR: &str = "<title>Load failed</title><h1>Could not load page</h1><p>Check the address, connection, and certificate.</p><a href='hexa://home'>Home</a>";
const SEARCH_ERROR: &str = "<title>Search failed</title><h1>Search query is too long</h1><p>Use a shorter query in the address bar.</p><a href='hexa://home'>Home</a>";
const SEARCH_PREFIX: &str = "https://duckduckgo.com/html/?q=";
const MAX_BROWSER_REDIRECTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppKind {
    Browser,
    Terminal,
    Forms,
    Packages,
    Settings,
    System,
    Games,
    Notes,
}

impl AppKind {
    const ALL: [Self; APP_COUNT] = [
        Self::Browser,
        Self::Terminal,
        Self::Forms,
        Self::Packages,
        Self::Settings,
        Self::System,
        Self::Games,
        Self::Notes,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Browser => 0,
            Self::Terminal => 1,
            Self::Forms => 2,
            Self::Packages => 3,
            Self::Settings => 4,
            Self::System => 5,
            Self::Games => 6,
            Self::Notes => 7,
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Browser => "BROWSER",
            Self::Terminal => "TERMINAL",
            Self::Forms => "FORMS",
            Self::Packages => "PACKAGES",
            Self::Settings => "SETTINGS",
            Self::System => "SYSTEM SCOPE",
            Self::Games => "PRISM ARCADE",
            Self::Notes => "NOTES",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Browser => "Browser",
            Self::Terminal => "Terminal",
            Self::Forms => "Forms",
            Self::Packages => "Packages",
            Self::Settings => "Settings",
            Self::System => "System",
            Self::Games => "Games",
            Self::Notes => "Notes",
        }
    }

    const fn owner(self) -> Fin {
        match self {
            Self::Browser => BROWSER_FIN,
            Self::Terminal => TERMINAL_FIN,
            Self::Forms => FORMS_FIN,
            Self::Packages => PACKAGES_FIN,
            Self::Settings => SETTINGS_FIN,
            Self::System => SYSTEM_FIN,
            Self::Games => GAMES_FIN,
            Self::Notes => NOTES_FIN,
        }
    }

    const fn shortcut(self) -> &'static str {
        match self {
            Self::Browser => "B",
            Self::Terminal => "T",
            Self::Forms => "F",
            Self::Packages => "P",
            Self::Settings => "S",
            Self::System => "I",
            Self::Games => "G",
            Self::Notes => "N",
        }
    }

    const fn accent(self) -> u32 {
        match self {
            Self::Browser => color::CYAN,
            Self::Terminal => color::GREEN,
            Self::Forms => color::PURPLE,
            Self::Packages => 0x0091_865E,
            Self::Settings => 0x0085_7788,
            Self::System => color::RED,
            Self::Games => 0x0091_7483,
            Self::Notes => 0x0095_8B68,
        }
    }
}

const SETTINGS_CATEGORY_COUNT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsCategory {
    System,
    Appearance,
    Network,
    Bluetooth,
    Display,
    Input,
    Privacy,
    About,
}

impl SettingsCategory {
    const ALL: [Self; SETTINGS_CATEGORY_COUNT] = [
        Self::System,
        Self::Appearance,
        Self::Network,
        Self::Bluetooth,
        Self::Display,
        Self::Input,
        Self::Privacy,
        Self::About,
    ];

    const fn index(self) -> usize {
        match self {
            Self::System => 0,
            Self::Appearance => 1,
            Self::Network => 2,
            Self::Bluetooth => 3,
            Self::Display => 4,
            Self::Input => 5,
            Self::Privacy => 6,
            Self::About => 7,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Appearance => "Appearance",
            Self::Network => "Network & Wi-Fi",
            Self::Bluetooth => "Bluetooth",
            Self::Display => "Display",
            Self::Input => "Mouse & keyboard",
            Self::Privacy => "Privacy",
            Self::About => "About",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::System => "Desktop behavior",
            Self::Appearance => "Colors and window style",
            Self::Network => "Connections and network access",
            Self::Bluetooth => "Nearby wireless devices",
            Self::Display => "HexaDisplay output",
            Self::Input => "Pointer and keyboard",
            Self::Privacy => "Local data and access",
            Self::About => "ExpOS system information",
        }
    }

    const fn row_count(self) -> usize {
        match self {
            Self::System => 2,
            Self::Appearance => 6,
            Self::Network => 4,
            Self::Bluetooth => 2,
            Self::Display => 6,
            Self::Input => 4,
            Self::Privacy => 2,
            Self::About => 4,
        }
    }

    fn shifted(self, direction: i8) -> Self {
        let index = if direction < 0 {
            (self.index() + SETTINGS_CATEGORY_COUNT - 1) % SETTINGS_CATEGORY_COUNT
        } else {
            (self.index() + 1) % SETTINGS_CATEGORY_COUNT
        };
        Self::ALL[index]
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccentChoice {
    Green,
    Cyan,
    Purple,
    Amber,
}

impl AccentChoice {
    const fn color(self) -> u32 {
        match self {
            Self::Green => color::GREEN,
            Self::Cyan => color::CYAN,
            Self::Purple => color::PURPLE,
            Self::Amber => 0x00B6_8B50,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Green => "Forest",
            Self::Cyan => "Ocean",
            Self::Purple => "Violet",
            Self::Amber => "Amber",
        }
    }

    fn shifted(self, direction: i8) -> Self {
        match (self, direction < 0) {
            (Self::Green, false) | (Self::Purple, true) => Self::Cyan,
            (Self::Cyan, false) | (Self::Amber, true) => Self::Purple,
            (Self::Purple, false) | (Self::Green, true) => Self::Amber,
            (Self::Amber, false) | (Self::Cyan, true) => Self::Green,
        }
    }

    const fn from_persisted(value: u8) -> Self {
        match value {
            1 => Self::Cyan,
            2 => Self::Purple,
            3 => Self::Amber,
            _ => Self::Green,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackdropChoice {
    Graphite,
    Midnight,
    Black,
}

impl BackdropChoice {
    const fn color(self) -> u32 {
        match self {
            Self::Graphite => color::BACKGROUND,
            Self::Midnight => 0x0009_1019,
            Self::Black => 0x0000_0000,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Graphite => "Graphite",
            Self::Midnight => "Midnight",
            Self::Black => "Black",
        }
    }

    fn shifted(self, direction: i8) -> Self {
        match (self, direction < 0) {
            (Self::Graphite, false) | (Self::Black, true) => Self::Midnight,
            (Self::Midnight, false) | (Self::Graphite, true) => Self::Black,
            (Self::Black, false) | (Self::Midnight, true) => Self::Graphite,
        }
    }

    const fn from_persisted(value: u8) -> Self {
        match value {
            1 => Self::Midnight,
            2 => Self::Black,
            _ => Self::Graphite,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThemeChoice {
    Obsidian,
    Graphite,
    Nord,
    Forest,
    Aurora,
    Rose,
}

impl ThemeChoice {
    const ALL: [Self; 6] = [
        Self::Obsidian,
        Self::Graphite,
        Self::Nord,
        Self::Forest,
        Self::Aurora,
        Self::Rose,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Obsidian => "Obsidian",
            Self::Graphite => "Graphite",
            Self::Nord => "Nord",
            Self::Forest => "Forest",
            Self::Aurora => "Aurora",
            Self::Rose => "Rose",
        }
    }

    const fn panel(self) -> u32 {
        match self {
            Self::Obsidian => 0x0010_1215,
            Self::Graphite => 0x0020_2225,
            Self::Nord => 0x001B_2430,
            Self::Forest => 0x0014_211C,
            Self::Aurora => 0x0011_1D25,
            Self::Rose => 0x0025_171D,
        }
    }

    const fn chrome(self) -> u32 {
        match self {
            Self::Obsidian => 0x000A_0C0F,
            Self::Graphite => 0x0018_1A1D,
            Self::Nord => 0x0013_1B26,
            Self::Forest => 0x000E_1915,
            Self::Aurora => 0x000B_1720,
            Self::Rose => 0x001D_1016,
        }
    }

    const fn window(self) -> u32 {
        match self {
            Self::Obsidian => 0x0004_0506,
            Self::Graphite => color::WINDOW,
            Self::Nord => 0x000E_1722,
            Self::Forest => 0x000B_1612,
            Self::Aurora => 0x0008_141C,
            Self::Rose => 0x0018_0B11,
        }
    }

    const fn card(self) -> u32 {
        match self {
            Self::Obsidian => 0x000D_1013,
            Self::Graphite => 0x0027_292D,
            Self::Nord => 0x001C_2938,
            Self::Forest => 0x0017_2921,
            Self::Aurora => 0x0014_2930,
            Self::Rose => 0x0030_1A23,
        }
    }

    const fn terminal(self) -> u32 {
        match self {
            Self::Obsidian => 0x0000_0000,
            Self::Graphite => 0x000C_0D0F,
            Self::Nord => 0x0008_101B,
            Self::Forest => 0x0005_100C,
            Self::Aurora => 0x0004_0E14,
            Self::Rose => 0x0010_050A,
        }
    }

    const fn from_persisted(value: u8) -> Self {
        match value {
            1 => Self::Graphite,
            2 => Self::Nord,
            3 => Self::Forest,
            4 => Self::Aurora,
            5 => Self::Rose,
            _ => Self::Obsidian,
        }
    }

    fn shifted(self, direction: i8) -> Self {
        let index = self as usize;
        let next = if direction < 0 {
            (index + Self::ALL.len() - 1) % Self::ALL.len()
        } else {
            (index + 1) % Self::ALL.len()
        };
        Self::ALL[next]
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WallpaperChoice {
    Solid,
    Gradient,
    Horizon,
    Grid,
    Dusk,
    Aurora,
    Mesh,
}

impl WallpaperChoice {
    const ALL: [Self; 7] = [
        Self::Solid,
        Self::Gradient,
        Self::Horizon,
        Self::Grid,
        Self::Dusk,
        Self::Aurora,
        Self::Mesh,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Gradient => "Gradient",
            Self::Horizon => "Horizon",
            Self::Grid => "Grid",
            Self::Dusk => "Dusk",
            Self::Aurora => "Aurora",
            Self::Mesh => "Mesh",
        }
    }

    const fn from_persisted(value: u8) -> Self {
        match value {
            1 => Self::Gradient,
            2 => Self::Horizon,
            3 => Self::Grid,
            4 => Self::Dusk,
            5 => Self::Aurora,
            6 => Self::Mesh,
            _ => Self::Solid,
        }
    }

    fn shifted(self, direction: i8) -> Self {
        let index = self as usize;
        let next = if direction < 0 {
            (index + Self::ALL.len() - 1) % Self::ALL.len()
        } else {
            (index + 1) % Self::ALL.len()
        };
        Self::ALL[next]
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CursorChoice {
    Light,
    Dark,
    Accent,
    Crosshair,
}

impl CursorChoice {
    const fn label(self) -> &'static str {
        match self {
            Self::Light => "Light arrow",
            Self::Dark => "Dark arrow",
            Self::Accent => "Accent arrow",
            Self::Crosshair => "Crosshair",
        }
    }

    const fn from_persisted(value: u8) -> Self {
        match value {
            1 => Self::Dark,
            2 => Self::Accent,
            3 => Self::Crosshair,
            _ => Self::Light,
        }
    }

    fn shifted(self, direction: i8) -> Self {
        match (self, direction < 0) {
            (Self::Light, false) | (Self::Accent, true) => Self::Dark,
            (Self::Dark, false) | (Self::Crosshair, true) => Self::Accent,
            (Self::Accent, false) | (Self::Light, true) => Self::Crosshair,
            (Self::Crosshair, false) | (Self::Dark, true) => Self::Light,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DesktopPreferences {
    theme: ThemeChoice,
    wallpaper: WallpaperChoice,
    cursor: CursorChoice,
    accent: AccentChoice,
    backdrop: BackdropChoice,
    pure_black_apps: bool,
    rounded_controls: bool,
    taskbar_visible: bool,
    status_visible: bool,
    window_borders: bool,
    high_contrast: bool,
    pointer_speed: u8,
    refresh_rate: state::RefreshRate,
    vsync: bool,
}

impl DesktopPreferences {
    const fn window_color(self) -> u32 {
        if self.pure_black_apps {
            0x0000_0000
        } else {
            self.theme.window()
        }
    }

    const fn panel_color(self) -> u32 {
        self.theme.panel()
    }

    const fn chrome_color(self) -> u32 {
        self.theme.chrome()
    }

    const fn card_color(self) -> u32 {
        self.theme.card()
    }

    const fn border_color(self, focused: bool) -> u32 {
        if self.high_contrast {
            if focused {
                color::WHITE
            } else {
                color::MUTED
            }
        } else if focused {
            color::MUTED
        } else {
            color::BORDER
        }
    }

    const fn pointer_speed_label(self) -> &'static str {
        match self.pointer_speed {
            1 => "Normal",
            2 => "Fast",
            _ => "Very fast",
        }
    }

    fn from_persistent(value: state::PersistentPreferences) -> Self {
        let flags = value.flags;
        Self {
            theme: ThemeChoice::from_persisted(value.theme),
            wallpaper: WallpaperChoice::from_persisted(value.wallpaper),
            cursor: CursorChoice::from_persisted(value.cursor_theme),
            accent: AccentChoice::from_persisted(value.accent),
            backdrop: BackdropChoice::from_persisted(value.backdrop),
            pure_black_apps: flags & state::PREF_PURE_BLACK_APPS != 0,
            rounded_controls: flags & state::PREF_ROUNDED_CONTROLS != 0,
            taskbar_visible: flags & state::PREF_TASKBAR_VISIBLE != 0,
            status_visible: flags & state::PREF_STATUS_VISIBLE != 0,
            window_borders: flags & state::PREF_WINDOW_BORDERS != 0,
            high_contrast: flags & state::PREF_HIGH_CONTRAST != 0,
            pointer_speed: value.pointer_speed.clamp(1, 3),
            refresh_rate: value.refresh_rate,
            vsync: value.vsync,
        }
    }

    fn update_persistent(
        self,
        mut value: state::PersistentPreferences,
    ) -> state::PersistentPreferences {
        value.display_mode = framebuffer::requested_mode().persisted();
        value.theme = self.theme as u8;
        value.wallpaper = self.wallpaper as u8;
        value.cursor_theme = self.cursor as u8;
        value.accent = self.accent as u8;
        value.backdrop = self.backdrop as u8;
        value.pointer_speed = self.pointer_speed;
        value.refresh_rate = self.refresh_rate;
        value.vsync = self.vsync;
        let desktop_flags = state::PREF_PURE_BLACK_APPS
            | state::PREF_ROUNDED_CONTROLS
            | state::PREF_TASKBAR_VISIBLE
            | state::PREF_STATUS_VISIBLE
            | state::PREF_WINDOW_BORDERS
            | state::PREF_HIGH_CONTRAST
            | state::PREF_NETWORK_ENABLED
            | state::PREF_WIFI_ENABLED
            | state::PREF_BLUETOOTH_ENABLED;
        value.flags &= !desktop_flags;
        if self.pure_black_apps {
            value.flags |= state::PREF_PURE_BLACK_APPS;
        }
        if self.rounded_controls {
            value.flags |= state::PREF_ROUNDED_CONTROLS;
        }
        if self.taskbar_visible {
            value.flags |= state::PREF_TASKBAR_VISIBLE;
        }
        if self.status_visible {
            value.flags |= state::PREF_STATUS_VISIBLE;
        }
        if self.window_borders {
            value.flags |= state::PREF_WINDOW_BORDERS;
        }
        if self.high_contrast {
            value.flags |= state::PREF_HIGH_CONTRAST;
        }
        let connectivity = radio::snapshot();
        if connectivity.network_enabled {
            value.flags |= state::PREF_NETWORK_ENABLED;
        }
        if connectivity.wifi_requested {
            value.flags |= state::PREF_WIFI_ENABLED;
        }
        if connectivity.bluetooth_requested {
            value.flags |= state::PREF_BLUETOOTH_ENABLED;
        }
        value
    }
}

struct DesktopState {
    server: DisplayServer,
    broker: CapabilityBroker,
    app_surfaces: [u32; APP_COUNT],
    app_handles: [u32; APP_COUNT],
    browser_network_handle: Option<u32>,
    settings_radio_handle: Option<u32>,
    app_open: [bool; APP_COUNT],
    app_ever_opened: [bool; APP_COUNT],
    app_minimized: [bool; APP_COUNT],
    panel_surface: u32,
    launcher_surface: u32,
    active: AppKind,
    launcher_open: bool,
    fullscreen: bool,
    preferences: DesktopPreferences,
    settings_category: SettingsCategory,
    settings_row: usize,
    settings_notice: &'static str,
    document: Document,
    browser_line: [u8; 512],
    browser_len: usize,
    browser_editing: bool,
    terminal_line: [u8; TERMINAL_CAPACITY],
    terminal_len: usize,
    terminal_output: [[u8; TERMINAL_OUTPUT_CAPACITY]; TERMINAL_SCROLLBACK],
    terminal_output_len: [u8; TERMINAL_SCROLLBACK],
    terminal_output_next: usize,
    terminal_output_count: usize,
    terminal_history: [[u8; TERMINAL_CAPACITY]; TERMINAL_HISTORY],
    terminal_history_len: [u8; TERMINAL_HISTORY],
    terminal_history_next: usize,
    terminal_history_count: usize,
    terminal_history_cursor: Option<usize>,
    notes: [u8; NOTES_CAPACITY],
    notes_len: usize,
    games: crate::games::GameHub,
    session: crate::session::Session,
    cursor: PointerCursor,
    dragging: Option<AppKind>,
    should_exit: bool,
    frame_pacer: FramePacer,
    full_redraw_requested: bool,
}

struct PointerCursor {
    x: i16,
    y: i16,
    under: [u32; CURSOR_WIDTH * CURSOR_HEIGHT],
    style: CursorChoice,
    accent: u32,
    drawn: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PointerRender {
    None,
    Cursor(framebuffer::DamageRegion),
    Full,
}

impl PointerCursor {
    fn new(style: CursorChoice, accent: u32) -> Self {
        Self {
            x: (framebuffer::width() / 2) as i16,
            y: (framebuffer::height() / 2) as i16,
            under: [0; CURSOR_WIDTH * CURSOR_HEIGHT],
            style,
            accent,
            drawn: false,
        }
    }

    fn set_style(&mut self, style: CursorChoice, accent: u32) {
        self.restore();
        self.style = style;
        self.accent = accent;
    }

    fn invalidate(&mut self) {
        self.drawn = false;
    }

    fn move_by(&mut self, dx: i16, dy: i16) -> Option<framebuffer::DamageRegion> {
        if dx == 0 && dy == 0 {
            return None;
        }
        let previous = self.damage_region();
        self.restore();
        self.x = (self.x.saturating_add(dx)).clamp(0, framebuffer::width() as i16 - 1);
        self.y = (self.y.saturating_add(dy)).clamp(0, framebuffer::height() as i16 - 1);
        self.draw();
        Some(previous.union(self.damage_region()))
    }

    const fn damage_region(&self) -> framebuffer::DamageRegion {
        framebuffer::DamageRegion::new(
            self.x as i32,
            self.y as i32,
            CURSOR_WIDTH as i32,
            CURSOR_HEIGHT as i32,
        )
    }

    fn restore(&mut self) {
        if !self.drawn {
            return;
        }
        for row in 0..CURSOR_HEIGHT {
            for column in 0..CURSOR_WIDTH {
                framebuffer::pixel(
                    self.x as i32 + column as i32,
                    self.y as i32 + row as i32,
                    self.under[row * CURSOR_WIDTH + column],
                );
            }
        }
        self.drawn = false;
    }

    fn draw(&mut self) {
        if self.style == CursorChoice::Crosshair {
            self.draw_crosshair();
            return;
        }
        const SHAPE: [u16; CURSOR_HEIGHT] = [
            0x0001, 0x0003, 0x0007, 0x000F, 0x001F, 0x003F, 0x007F, 0x00FF, 0x01FF, 0x03FF, 0x01FF,
            0x019F, 0x030F, 0x0606, 0x0C06, 0x0804, 0x0000, 0x0000, 0x0000, 0x0000,
        ];
        for row in 0..CURSOR_HEIGHT {
            for column in 0..CURSOR_WIDTH {
                self.under[row * CURSOR_WIDTH + column] = framebuffer::read_pixel(
                    self.x as i32 + column as i32,
                    self.y as i32 + row as i32,
                );
            }
        }
        for (row, bits) in SHAPE.iter().copied().enumerate() {
            for column in 0..CURSOR_WIDTH {
                if bits & (1 << column) == 0 {
                    continue;
                }
                let edge = column == 0
                    || row == 0
                    || SHAPE
                        .get(row.saturating_sub(1))
                        .is_none_or(|above| above & (1 << column) == 0)
                    || SHAPE
                        .get(row + 1)
                        .is_none_or(|below| below & (1 << column) == 0)
                    || bits & (1 << column.saturating_sub(1)) == 0
                    || bits & (1 << (column + 1)) == 0;
                framebuffer::pixel(
                    self.x as i32 + column as i32,
                    self.y as i32 + row as i32,
                    if edge {
                        match self.style {
                            CursorChoice::Dark => color::WHITE,
                            _ => 0x0012_1822,
                        }
                    } else {
                        match self.style {
                            CursorChoice::Light => color::WHITE,
                            CursorChoice::Dark => 0x0012_1822,
                            CursorChoice::Accent => self.accent,
                            CursorChoice::Crosshair => self.accent,
                        }
                    },
                );
            }
        }
        self.drawn = true;
    }

    fn draw_crosshair(&mut self) {
        for row in 0..CURSOR_HEIGHT {
            for column in 0..CURSOR_WIDTH {
                self.under[row * CURSOR_WIDTH + column] = framebuffer::read_pixel(
                    self.x as i32 + column as i32,
                    self.y as i32 + row as i32,
                );
            }
        }
        for offset in 0..CURSOR_WIDTH as i32 {
            if !(5..=8).contains(&offset) {
                framebuffer::pixel(self.x as i32 + offset, self.y as i32 + 7, self.accent);
            }
        }
        for offset in 0..CURSOR_HEIGHT as i32 {
            if !(5..=9).contains(&offset) {
                framebuffer::pixel(self.x as i32 + 7, self.y as i32 + offset, self.accent);
            }
        }
        framebuffer::outline(self.x as i32 + 5, self.y as i32 + 5, 5, 5, color::WHITE);
        self.drawn = true;
    }
}

impl DesktopState {
    fn new(
        start_app: Option<AppKind>,
        session: crate::session::Session,
        allow_network: bool,
    ) -> Self {
        let active = start_app.unwrap_or(AppKind::Terminal);
        let preferences = DesktopPreferences::from_persistent(state::preferences());
        let frame_pacer = FramePacer::new(timing_config(preferences), crate::hardware::timestamp());
        let mut server = DisplayServer::new();
        let mut broker = CapabilityBroker::new();
        let compositor_handle = broker
            .issue_for(
                DISPLAY_FIN,
                Authority::Operator,
                DISPLAY_FIN,
                STABLE_FIN,
                Operations::DISPLAY
                    .union(Operations::INPUT)
                    .union(Operations::CONFIGURE),
                u64::MAX,
            )
            .expect("compositor capability");
        let background = server
            .create_surface(
                DISPLAY_FIN,
                "Root Canvas",
                SurfaceRole::Background,
                Rect::new(
                    0,
                    0,
                    framebuffer::width() as u16,
                    framebuffer::height() as u16,
                ),
            )
            .expect("desktop background surface");
        let panel = server
            .create_surface(
                DISPLAY_FIN,
                "Panel",
                SurfaceRole::Panel,
                Rect::new(
                    0,
                    taskbar_y(),
                    framebuffer::width() as u16,
                    TASKBAR_HEIGHT as u16,
                ),
            )
            .expect("desktop panel surface");

        let mut app_surfaces = [0; APP_COUNT];
        let mut app_handles = [0; APP_COUNT];
        for (index, app) in AppKind::ALL.iter().copied().enumerate() {
            let rect = default_rect(app);
            let surface = server
                .create_surface(app.owner(), app.title(), SurfaceRole::Window, rect)
                .expect("built-in application surface");
            let _ = server.attach(
                app.owner(),
                surface,
                buffer(
                    (index + 3) as u32,
                    app.owner(),
                    framebuffer::width() as u16,
                    framebuffer::height() as u16,
                ),
            );
            let _ = server.set_visible(app.owner(), surface, start_app == Some(app));
            app_surfaces[index] = surface;
            app_handles[index] = broker
                .delegate(
                    compositor_handle.id,
                    app.owner(),
                    Operations::DISPLAY.union(Operations::INPUT),
                    u64::MAX,
                    0,
                )
                .expect("application display capability")
                .id;
        }
        let browser_network_handle = if allow_network && network::available() {
            broker
                .issue_for(
                    BROWSER_FIN,
                    session.authority(),
                    network::NETWORK_FIN,
                    STABLE_FIN,
                    Operations::NETWORK,
                    u64::MAX,
                )
                .ok()
                .map(|handle| handle.id)
        } else {
            None
        };
        let settings_radio_handle = broker
            .issue_for(
                SETTINGS_FIN,
                session.authority(),
                radio::RADIO_FIN,
                STABLE_FIN,
                Operations::CONFIGURE,
                u64::MAX,
            )
            .ok()
            .map(|handle| handle.id);

        let launcher_surface = server
            .create_surface(
                DISPLAY_FIN,
                "Applications",
                SurfaceRole::Popup,
                Rect::new(LAUNCHER_X, launcher_y(), LAUNCHER_WIDTH, LAUNCHER_HEIGHT),
            )
            .expect("desktop launcher surface");
        let _ = server.attach(
            DISPLAY_FIN,
            background,
            buffer(
                1,
                DISPLAY_FIN,
                framebuffer::width() as u16,
                framebuffer::height() as u16,
            ),
        );
        let _ = server.attach(
            DISPLAY_FIN,
            panel,
            buffer(
                2,
                DISPLAY_FIN,
                framebuffer::width() as u16,
                TASKBAR_HEIGHT as u16,
            ),
        );
        let _ = server.set_visible(DISPLAY_FIN, panel, preferences.taskbar_visible);
        let _ = server.attach(
            DISPLAY_FIN,
            launcher_surface,
            buffer(20, DISPLAY_FIN, LAUNCHER_WIDTH, LAUNCHER_HEIGHT),
        );
        let _ = server.set_visible(DISPLAY_FIN, launcher_surface, false);

        let _ = server.commit(DISPLAY_FIN, background);
        let _ = server.commit(DISPLAY_FIN, panel);
        for app in AppKind::ALL {
            let _ = server.commit(app.owner(), app_surfaces[app.index()]);
        }
        let _ = server.commit(DISPLAY_FIN, launcher_surface);
        if start_app.is_some() {
            let _ = server.focus(app_surfaces[active.index()]);
        }
        let mut state = Self {
            server,
            broker,
            app_surfaces,
            app_handles,
            browser_network_handle,
            settings_radio_handle,
            app_open: core::array::from_fn(|index| start_app == Some(AppKind::ALL[index])),
            app_ever_opened: core::array::from_fn(|index| start_app == Some(AppKind::ALL[index])),
            app_minimized: [false; APP_COUNT],
            panel_surface: panel,
            launcher_surface,
            active,
            launcher_open: false,
            fullscreen: false,
            preferences,
            settings_category: SettingsCategory::System,
            settings_row: 0,
            settings_notice: "Changes are saved locally.",
            document: Document::parse("hexa://home", HOME).expect("built-in home document"),
            browser_line: [0; 512],
            browser_len: 0,
            browser_editing: false,
            terminal_line: [0; TERMINAL_CAPACITY],
            terminal_len: 0,
            terminal_output: [[0; TERMINAL_OUTPUT_CAPACITY]; TERMINAL_SCROLLBACK],
            terminal_output_len: [0; TERMINAL_SCROLLBACK],
            terminal_output_next: 0,
            terminal_output_count: 0,
            terminal_history: [[0; TERMINAL_CAPACITY]; TERMINAL_HISTORY],
            terminal_history_len: [0; TERMINAL_HISTORY],
            terminal_history_next: 0,
            terminal_history_count: 0,
            terminal_history_cursor: None,
            notes: [0; NOTES_CAPACITY],
            notes_len: 0,
            games: crate::games::GameHub::new(),
            session,
            cursor: PointerCursor::new(preferences.cursor, preferences.accent.color()),
            dragging: None,
            should_exit: false,
            frame_pacer,
            full_redraw_requested: false,
        };
        state.terminal_push("ExpOS terminal");
        state.terminal_push("Type help for commands. Up/Down recalls history.");
        state.log_browser_engine();
        state
    }

    fn active_surface(&self) -> u32 {
        self.app_surfaces[self.active.index()]
    }

    fn authorized(&self, app: AppKind, operation: Operations) -> bool {
        self.broker
            .authorize_requester(
                self.app_handles[app.index()],
                app.owner(),
                DISPLAY_FIN,
                STABLE_FIN,
                operation,
                0,
            )
            .is_ok()
    }

    fn network_active(&self) -> bool {
        radio::snapshot().network_enabled && self.browser_network_handle.is_some()
    }

    fn save_preferences(&mut self) {
        let persistent = self.preferences.update_persistent(state::preferences());
        if let Err(error) = state::save_preferences(persistent) {
            self.settings_notice = error.message();
            slog!("HEXA_SETTING_PERSIST_FAILED error={:?}\r\n", error);
        }
    }

    fn reconfigure_presentation(&mut self) {
        self.frame_pacer.reconfigure(
            timing_config(self.preferences),
            crate::hardware::timestamp(),
        );
    }

    fn work_area_bottom(&self) -> i16 {
        if self.preferences.taskbar_visible {
            taskbar_y()
        } else {
            framebuffer::height() as i16
        }
    }

    fn route_key(&mut self, key: u8) {
        if !self.launcher_open
            && self.app_is_visible(self.active)
            && self.authorized(self.active, Operations::INPUT)
        {
            let _ = self.server.route_key(key);
        }
    }

    fn handle_pointer(&mut self, pointer: PointerEvent) -> PointerRender {
        let speed = self.preferences.pointer_speed as i16;
        let motion_x = pointer.dx.saturating_mul(speed);
        let motion_y = pointer.dy.saturating_mul(speed);
        let cursor_damage = self.cursor.move_by(motion_x, motion_y);
        self.route_pointer(pointer);
        if pointer.released & 1 != 0 {
            self.dragging = None;
        }
        if pointer.buttons & 1 != 0
            && self.dragging.is_some()
            && (pointer.dx != 0 || pointer.dy != 0)
        {
            self.drag_active(motion_x, motion_y);
            return PointerRender::Full;
        }
        if pointer.pressed & 1 != 0 {
            return if self.pointer_press(self.cursor.x, self.cursor.y) {
                PointerRender::Full
            } else if let Some(damage) = cursor_damage {
                PointerRender::Cursor(damage)
            } else {
                PointerRender::None
            };
        }
        cursor_damage.map_or(PointerRender::None, PointerRender::Cursor)
    }

    fn route_pointer(&mut self, pointer: PointerEvent) {
        let Some(surface_id) = self.server.hit_test(self.cursor.x, self.cursor.y) else {
            return;
        };
        let Some(app) = AppKind::ALL
            .iter()
            .copied()
            .find(|app| self.app_surfaces[app.index()] == surface_id)
        else {
            return;
        };
        if self.authorized(app, Operations::INPUT) {
            let _ = self.server.route_pointer(
                self.cursor.x,
                self.cursor.y,
                pointer.buttons,
                pointer.pressed | pointer.released,
            );
            while self.server.poll_event(app.owner()).is_some() {}
        }
    }

    fn pointer_press(&mut self, x: i16, y: i16) -> bool {
        if self.preferences.taskbar_visible && y >= taskbar_y() {
            if (START_X..START_X + 32).contains(&x) {
                self.toggle_launcher();
                return true;
            }
            let mut running_index = 0_i16;
            for app in AppKind::ALL {
                if !self.app_open[app.index()] {
                    continue;
                }
                let left = TASK_ICON_X + running_index * TASK_ICON_STEP;
                if (left..left + 32).contains(&x) {
                    if app == self.active && !self.app_minimized[app.index()] {
                        self.minimize_active();
                    } else {
                        self.focus_existing(app);
                    }
                    return true;
                }
                running_index += 1;
            }
            return false;
        }
        if self.launcher_open {
            if (LAUNCHER_X..LAUNCHER_X + LAUNCHER_WIDTH as i16).contains(&x)
                && (launcher_y()..launcher_y() + LAUNCHER_HEIGHT as i16).contains(&y)
            {
                for (index, app) in AppKind::ALL.iter().copied().enumerate() {
                    let left = LAUNCHER_X + 8;
                    let top = launcher_y() + 48 + index as i16 * 32;
                    if (left..left + LAUNCHER_WIDTH as i16 - 16).contains(&x)
                        && (top..top + 28).contains(&y)
                    {
                        self.focus_existing(app);
                        return true;
                    }
                }
                return false;
            }
            self.close_launcher();
            return true;
        }
        let Some(surface_id) = self.server.hit_test(x, y) else {
            return false;
        };
        let Some(app) = AppKind::ALL
            .iter()
            .copied()
            .find(|app| self.app_surfaces[app.index()] == surface_id)
        else {
            return false;
        };
        if !self.authorized(app, Operations::INPUT) {
            return false;
        }
        self.active = app;
        let _ = self.server.focus(surface_id);
        let Some(rect) = self
            .server
            .surface(surface_id)
            .map(|surface| surface.current.rect)
        else {
            return true;
        };
        let right = rect.x.saturating_add_unsigned(rect.width);
        if y < rect.y + 36 {
            if x >= right - 42 {
                self.close_active();
            } else if x >= right - 84 {
                self.toggle_fullscreen();
            } else if x >= right - 126 {
                self.minimize_active();
            } else {
                if self.fullscreen {
                    self.fullscreen = false;
                    self.set_app_geometry(app, default_rect(app));
                }
                self.dragging = Some(app);
            }
            return true;
        }
        if app == AppKind::Games && self.games.handle_click(x, y, rect) {
            return true;
        }
        if app == AppKind::Browser && self.browser_click(x, y, rect) {
            return true;
        }
        if app == AppKind::Settings && self.settings_click(x, y, rect) {
            return true;
        }
        true
    }

    fn drag_active(&mut self, dx: i16, dy: i16) {
        let Some(app) = self.dragging else {
            return;
        };
        let id = self.app_surfaces[app.index()];
        let owner = app.owner();
        let Some(rect) = self.server.surface(id).map(|surface| surface.current.rect) else {
            return;
        };
        let max_x = (framebuffer::width() as i32 - rect.width as i32).max(0);
        let max_y = (self.work_area_bottom() as i32 - rect.height as i32).max(0);
        let next_x = (rect.x as i32 + dx as i32).clamp(0, max_x) as i16;
        let next_y = (rect.y as i32 + dy as i32).clamp(0, max_y) as i16;
        let _ = self.server.set_position(owner, id, next_x, next_y);
        let _ = self.server.commit(owner, id);
    }

    fn app_is_visible(&self, app: AppKind) -> bool {
        self.app_open[app.index()]
            && !self.app_minimized[app.index()]
            && (!self.fullscreen || app == self.active)
    }

    fn has_active_window(&self) -> bool {
        self.app_open[self.active.index()] && !self.app_minimized[self.active.index()]
    }

    fn sync_visibility(&mut self) {
        for app in AppKind::ALL {
            if !self.authorized(app, Operations::DISPLAY) {
                continue;
            }
            let id = self.app_surfaces[app.index()];
            let _ = self
                .server
                .set_visible(app.owner(), id, self.app_is_visible(app));
            let _ = self.server.commit(app.owner(), id);
        }
    }

    fn switch_to(&mut self, next: AppKind) {
        self.normalize_fullscreen();
        let was_closed = !self.app_open[next.index()];
        self.app_open[next.index()] = true;
        self.app_minimized[next.index()] = false;
        self.active = next;
        self.fullscreen = false;
        self.close_launcher();
        if was_closed {
            self.set_app_geometry(next, default_rect(next));
            if self.app_ever_opened[next.index()] {
                slog!("HEXA_APP_REOPENED {}\r\n", next.title());
            } else {
                self.app_ever_opened[next.index()] = true;
                slog!("HEXA_APP_OPENED {}\r\n", next.title());
            }
        }
        self.sync_visibility();
        let _ = self.server.focus(self.active_surface());
    }

    fn focus_existing(&mut self, next: AppKind) {
        if !self.app_open[next.index()] {
            self.switch_to(next);
            return;
        }
        self.normalize_fullscreen();
        self.active = next;
        self.app_minimized[next.index()] = false;
        self.fullscreen = false;
        self.close_launcher();
        self.sync_visibility();
        let _ = self.server.focus(self.active_surface());
    }

    fn cycle_app(&mut self) {
        for offset in 1..=APP_COUNT {
            let next = AppKind::ALL[(self.active.index() + offset) % APP_COUNT];
            if self.app_open[next.index()] && !self.app_minimized[next.index()] {
                self.normalize_fullscreen();
                self.active = next;
                self.sync_visibility();
                let _ = self.server.focus(self.active_surface());
                return;
            }
        }
    }

    fn close_active(&mut self) {
        if !self.has_active_window() {
            return;
        }
        self.app_open[self.active.index()] = false;
        self.app_minimized[self.active.index()] = false;
        slog!("HEXA_APP_CLOSED {}\r\n", self.active.title());
        self.fullscreen = false;
        let replacement = AppKind::ALL
            .iter()
            .copied()
            .find(|app| self.app_open[app.index()] && !self.app_minimized[app.index()]);
        if let Some(next) = replacement {
            self.active = next;
        }
        self.sync_visibility();
        if self.has_active_window() {
            let _ = self.server.focus(self.active_surface());
        }
    }

    fn minimize_active(&mut self) {
        if !self.has_active_window() {
            return;
        }
        if self.fullscreen {
            self.set_app_geometry(self.active, default_rect(self.active));
        }
        self.app_minimized[self.active.index()] = true;
        self.fullscreen = false;
        if let Some(next) = AppKind::ALL
            .iter()
            .copied()
            .find(|app| self.app_open[app.index()] && !self.app_minimized[app.index()])
        {
            self.active = next;
        }
        self.sync_visibility();
        if self.has_active_window() {
            let _ = self.server.focus(self.active_surface());
        }
    }

    fn toggle_fullscreen(&mut self) {
        if !self.has_active_window() {
            return;
        }
        self.fullscreen = !self.fullscreen;
        self.sync_visibility();
        let bottom = self.work_area_bottom();
        let rect = if self.fullscreen {
            Rect::new(10, 10, framebuffer::width() as u16 - 20, bottom as u16 - 20)
        } else {
            default_rect(self.active)
        };
        self.set_app_geometry(self.active, rect);
        let _ = self.server.focus(self.active_surface());
    }

    fn normalize_fullscreen(&mut self) {
        if self.fullscreen {
            let app = self.active;
            self.fullscreen = false;
            self.set_app_geometry(app, default_rect(app));
        }
    }

    fn drain_protocol_events(&mut self) {
        for app in AppKind::ALL {
            while self.server.poll_event(app.owner()).is_some() {}
        }
        while self.server.poll_event(DISPLAY_FIN).is_some() {}
    }

    fn toggle_launcher(&mut self) {
        self.launcher_open = !self.launcher_open;
        let _ = self
            .server
            .set_visible(DISPLAY_FIN, self.launcher_surface, self.launcher_open);
        let _ = self.server.commit(DISPLAY_FIN, self.launcher_surface);
        if self.launcher_open {
            let _ = self.server.focus(self.launcher_surface);
        } else if self.has_active_window() {
            let _ = self.server.focus(self.active_surface());
        }
    }

    fn close_launcher(&mut self) {
        if self.launcher_open {
            self.launcher_open = false;
            let _ = self
                .server
                .set_visible(DISPLAY_FIN, self.launcher_surface, false);
            let _ = self.server.commit(DISPLAY_FIN, self.launcher_surface);
        }
    }

    fn move_active(&mut self, dx: i16, dy: i16) {
        if !self.has_active_window() {
            return;
        }
        let id = self.active_surface();
        let owner = self.active.owner();
        let Some(rect) = self.server.surface(id).map(|surface| surface.current.rect) else {
            return;
        };
        let max_x = (framebuffer::width() as i32 - rect.width as i32 - 8).max(8);
        let max_y = (self.work_area_bottom() as i32 - rect.height as i32).max(8);
        let x = (rect.x as i32 + dx as i32).clamp(8, max_x) as i16;
        let y = (rect.y as i32 + dy as i32).clamp(8, max_y) as i16;
        let _ = self.server.set_position(owner, id, x, y);
        let _ = self.server.commit(owner, id);
    }

    fn set_app_geometry(&mut self, app: AppKind, rect: Rect) {
        if !self.authorized(app, Operations::DISPLAY) {
            return;
        }
        let id = self.app_surfaces[app.index()];
        let _ = self.server.set_geometry(app.owner(), id, rect);
        let _ = self.server.commit(app.owner(), id);
    }

    fn navigate(&mut self, url: &str, source: &str) {
        if let Ok(document) = Document::parse(url, source) {
            self.set_document(document);
        }
    }

    fn set_document(&mut self, document: Document) {
        self.document = document;
        self.log_browser_engine();
        let surface = self.app_surfaces[AppKind::Browser.index()];
        let damage_rect = self
            .server
            .surface(surface)
            .map(|surface| {
                Rect::new(
                    0,
                    0,
                    surface.current.rect.width,
                    surface.current.rect.height,
                )
            })
            .unwrap_or_else(|| {
                let (width, height) = app_dimensions();
                Rect::new(0, 0, width, height)
            });
        let _ = self.server.damage(BROWSER_FIN, surface, damage_rect);
        let _ = self.server.commit(BROWSER_FIN, surface);
    }

    fn log_browser_engine(&self) {
        let report = self.document.script_report();
        slog!(
            "HEXA_BROWSER_ENGINE nodes={} css_rules={} scripts={} executed={} rejected={} handlers={}\r\n",
            self.document.len(),
            self.document.style_rule_count(),
            report.scripts_seen,
            report.scripts_executed,
            report.scripts_rejected,
            report.handlers_registered
        );
    }

    fn navigate_address(&mut self, address: &str) {
        let address = address.trim();
        if address.is_empty() {
            self.navigate("hexa://home", HOME);
            return;
        }
        if address.eq_ignore_ascii_case("hexa://home") || address == "home" {
            self.navigate("hexa://home", HOME);
            return;
        }
        if address.eq_ignore_ascii_case("hexa://about") || address == "about" {
            self.navigate("hexa://about", ABOUT);
            return;
        }
        if address.eq_ignore_ascii_case("hexa://packages") || address == "packages" {
            self.navigate("hexa://packages", BROWSER_PACKAGES);
            return;
        }
        if address.eq_ignore_ascii_case("hexa://system") || address == "system" {
            self.navigate("hexa://system", BROWSER_SYSTEM);
            return;
        }
        if !address.starts_with("http://") && !address.starts_with("https://") {
            let query = address.strip_prefix('?').unwrap_or(address).trim();
            let mut url = [0_u8; 512];
            let Some(length) = encode_search_url(query, &mut url) else {
                self.navigate("hexa://search-error", SEARCH_ERROR);
                return;
            };
            let url = core::str::from_utf8(&url[..length]).unwrap_or("");
            slog!(
                "HEXA_BROWSER_SEARCH query_bytes={} url_bytes={} provider=duckduckgo-html\r\n",
                query.len(),
                length
            );
            self.navigate_address(url);
            return;
        }
        if !self.network_active() {
            self.navigate("hexa://offline", NETWORK_BLOCKED);
            slog!("HEXA_BROWSER_HTTP_ERROR error=CapabilityDenied\r\n");
            return;
        }
        let Some(handle_id) = self.browser_network_handle else {
            return;
        };
        let mut current = [0_u8; 512];
        let Some(current_len) = copy_browser_url(&mut current, address.as_bytes()) else {
            self.navigate("hexa://error", NETWORK_ERROR);
            slog!("HEXA_BROWSER_HTTP_ERROR error=BadUrl\r\n");
            return;
        };
        let mut current_len = current_len;
        for redirect_count in 0..=MAX_BROWSER_REDIRECTS {
            let current_url = core::str::from_utf8(&current[..current_len]).unwrap_or("");
            match network::http_get(
                &self.broker,
                handle_id,
                BROWSER_FIN,
                STABLE_FIN,
                current_url,
            ) {
                Ok(response) => {
                    if is_http_redirect(response.status) {
                        let Some(location) = response.location() else {
                            self.navigate("hexa://error", NETWORK_ERROR);
                            slog!("HEXA_BROWSER_HTTP_ERROR error=RedirectWithoutLocation\r\n");
                            return;
                        };
                        if redirect_count == MAX_BROWSER_REDIRECTS {
                            self.navigate("hexa://error", NETWORK_ERROR);
                            slog!("HEXA_BROWSER_HTTP_ERROR error=TooManyRedirects\r\n");
                            return;
                        }
                        let mut next = [0_u8; 512];
                        let Some(next_len) = resolve_browser_link(current_url, location, &mut next)
                        else {
                            self.navigate("hexa://error", NETWORK_ERROR);
                            slog!("HEXA_BROWSER_HTTP_ERROR error=BadRedirect\r\n");
                            return;
                        };
                        let next_url = core::str::from_utf8(&next[..next_len]).unwrap_or("");
                        if current_url.starts_with("https://") && next_url.starts_with("http://") {
                            self.navigate("hexa://error", NETWORK_ERROR);
                            slog!("HEXA_BROWSER_HTTP_ERROR error=InsecureRedirect\r\n");
                            return;
                        }
                        slog!(
                            "HEXA_BROWSER_REDIRECT status={} hop={} target_bytes={}\r\n",
                            response.status,
                            redirect_count + 1,
                            next_len
                        );
                        current[..next_len].copy_from_slice(&next[..next_len]);
                        current_len = next_len;
                        continue;
                    }
                    let mut sanitized = [0_u8; network::HTTP_BODY_CAPACITY];
                    for (output, byte) in sanitized.iter_mut().zip(response.body().iter().copied())
                    {
                        *output = if byte.is_ascii_graphic()
                            || matches!(byte, b' ' | b'\n' | b'\r' | b'\t')
                        {
                            byte
                        } else {
                            b' '
                        };
                    }
                    let source =
                        core::str::from_utf8(&sanitized[..response.body_len]).unwrap_or("");
                    let projected = Document::parse_duckduckgo_results(current_url, source).ok();
                    let search_results =
                        projected.as_ref().map_or(0, |results| results.result_count);
                    let document = if let Some(results) = projected {
                        Ok(results.document)
                    } else {
                        Document::parse(current_url, source).or_else(|_| {
                            // Script-first sites may not place any supported
                            // nodes in the bounded response. The authenticated
                            // fetch still succeeded, so report that honestly.
                            Document::parse(
                                current_url,
                                "<title>Page loaded</title><h1>Secure response received</h1><p>This page needs external resources, browser APIs, or media features beyond the bounded ExpOS engine.</p>",
                            )
                        })
                    };
                    match document {
                        Ok(document) => {
                            self.set_document(document);
                            if search_results != 0 {
                                slog!("HEXA_SEARCH_RESULTS count={}\r\n", search_results);
                            }
                            slog!(
                                "HEXA_BROWSER_HTTP_OK status={} bytes={} peer={}.{}.{}.{}\r\n",
                                response.status,
                                response.body_len,
                                response.peer[0],
                                response.peer[1],
                                response.peer[2],
                                response.peer[3]
                            );
                        }
                        Err(error) => {
                            self.navigate("hexa://error", NETWORK_ERROR);
                            slog!("HEXA_BROWSER_HTTP_ERROR error={:?}\r\n", error);
                        }
                    }
                }
                Err(error) => {
                    self.navigate("hexa://error", NETWORK_ERROR);
                    slog!("HEXA_BROWSER_HTTP_ERROR error={:?}\r\n", error);
                }
            }
            return;
        }
    }

    fn browser_click(&mut self, x: i16, y: i16, rect: Rect) -> bool {
        let local_x = x - rect.x;
        let local_y = y - rect.y;
        if (50..86).contains(&local_y) {
            if (18..58).contains(&local_x) {
                self.navigate("hexa://home", HOME);
                self.browser_editing = false;
                return true;
            }
            if (66..rect.width as i16 - 20).contains(&local_x) {
                let url = self.document.url().as_bytes();
                self.browser_len = url.len().min(self.browser_line.len());
                self.browser_line[..self.browser_len].copy_from_slice(&url[..self.browser_len]);
                self.browser_editing = true;
                return true;
            }
        }

        let mut content_y = rect.y as i32 + 110;
        let mut selected: Option<(usize, BrowserText, bool)> = None;
        for styled in self.document.styled_nodes() {
            if styled.node.kind == NodeKind::Title || !styled.style.is_rendered() {
                continue;
            }
            let layout = browser_layout(rect, styled, content_y);
            content_y = layout.next_y;
            if (layout.x..layout.x + layout.width).contains(&(x as i32))
                && (layout.y..layout.y + layout.height).contains(&(y as i32))
            {
                selected = Some((styled.index, styled.node.target, styled.clickable));
                break;
            }
        }
        let Some((index, target, scripted)) = selected else {
            return false;
        };
        if scripted && self.document.dispatch_click_at_node(index) {
            let report = self.document.script_report();
            slog!(
                "HEXA_BROWSER_EVENT type=click node={} executed={}\r\n",
                index,
                report.statements_executed
            );
            self.log_browser_engine();
            return true;
        }
        if !target.as_str().is_empty() {
            let mut address = [0_u8; 512];
            let Some(length) =
                resolve_browser_link(self.document.url(), target.as_str(), &mut address)
            else {
                self.navigate("hexa://error", NETWORK_ERROR);
                slog!("HEXA_BROWSER_HTTP_ERROR error=BadUrl\r\n");
                return true;
            };
            let address = core::str::from_utf8(&address[..length]).unwrap_or("");
            self.navigate_address(address);
            self.frame_pacer.reset_phase(crate::hardware::timestamp());
            return true;
        }
        false
    }

    fn select_settings_category(&mut self, category: SettingsCategory) {
        self.settings_category = category;
        self.settings_row = 0;
    }

    fn shift_settings_category(&mut self, direction: i8) {
        self.select_settings_category(self.settings_category.shifted(direction));
    }

    fn settings_click(&mut self, x: i16, y: i16, rect: Rect) -> bool {
        let local_x = x - rect.x;
        let local_y = y - rect.y;
        let sidebar_width = settings_sidebar_width(rect);
        let category_top = settings_category_top(rect);
        let category_step = settings_category_step(rect);
        let row_top = settings_row_top(rect);
        let row_step = settings_row_step(rect);
        let row_height = settings_row_height(rect);
        if (12..sidebar_width).contains(&local_x) && local_y >= category_top {
            let index = ((local_y - category_top) / category_step) as usize;
            if let Some(category) = SettingsCategory::ALL.get(index).copied() {
                if local_y < category_top + index as i16 * category_step + category_step - 4 {
                    self.select_settings_category(category);
                    return true;
                }
            }
        }

        if local_x >= sidebar_width + 24 && local_x < rect.width as i16 - 20 && local_y >= row_top {
            let row = ((local_y - row_top) / row_step) as usize;
            if row < self.settings_category.row_count()
                && local_y < row_top + row as i16 * row_step + row_height
            {
                self.settings_row = row;
                self.activate_setting(1);
                return true;
            }
        }
        false
    }

    fn handle_settings_key(&mut self, key: u8) -> bool {
        self.full_redraw_requested = false;
        match key {
            KEY_LEFT | b'[' => self.shift_settings_category(-1),
            KEY_RIGHT | b']' => self.shift_settings_category(1),
            KEY_UP | b'k' => {
                self.settings_row = self.settings_row.saturating_sub(1);
            }
            KEY_DOWN | b'j' => {
                self.settings_row = (self.settings_row + 1)
                    .min(self.settings_category.row_count().saturating_sub(1));
            }
            b'1'..=b'8' => {
                let index = (key - b'1') as usize;
                self.select_settings_category(SettingsCategory::ALL[index]);
            }
            b'\n' | b' ' | b'+' | b'=' => self.activate_setting(1),
            b'-' => self.activate_setting(-1),
            _ => return false,
        }
        true
    }

    fn change_network_policy(&mut self) {
        let Some(handle_id) = self.settings_radio_handle else {
            self.settings_notice = "Read-only: DIESE did not grant Configure access.";
            slog!("HEXA_SETTING_DENIED key=network error=CapabilityDenied\r\n");
            return;
        };
        let enabled = !radio::snapshot().network_enabled;
        match radio::set_network_enabled(&self.broker, handle_id, SETTINGS_FIN, STABLE_FIN, enabled)
        {
            Ok(_) => {
                self.settings_notice = if enabled {
                    "Network packet access is enabled."
                } else {
                    "Network packet access is disabled."
                };
                slog!(
                    "HEXA_SETTING_CHANGED key=network value={}\r\n",
                    if enabled { "on" } else { "off" }
                );
            }
            Err(error) => {
                self.settings_notice = error.message();
                slog!("HEXA_SETTING_DENIED key=network error={:?}\r\n", error);
            }
        }
    }

    fn change_radio_policy(&mut self, kind: radio::RadioKind) {
        let Some(handle_id) = self.settings_radio_handle else {
            self.settings_notice = "Read-only: DIESE did not grant Configure access.";
            slog!("HEXA_SETTING_DENIED key=radio error=CapabilityDenied\r\n");
            return;
        };
        let snapshot = radio::snapshot();
        let (enabled, key) = match kind {
            radio::RadioKind::Wifi => (!snapshot.wifi_requested, "wifi"),
            radio::RadioKind::Bluetooth => (!snapshot.bluetooth_requested, "bluetooth"),
        };
        match radio::set_radio_enabled(
            &self.broker,
            handle_id,
            SETTINGS_FIN,
            STABLE_FIN,
            kind,
            enabled,
        ) {
            Ok(_) => {
                self.settings_notice = if enabled {
                    "Radio power is enabled."
                } else {
                    "Radio power is disabled."
                };
                slog!(
                    "HEXA_SETTING_CHANGED key={} value={}\r\n",
                    key,
                    if enabled { "on" } else { "off" }
                );
            }
            Err(error) => {
                self.settings_notice = error.message();
                slog!("HEXA_SETTING_DENIED key={} error={:?}\r\n", key, error);
            }
        }
    }

    fn activate_setting(&mut self, direction: i8) {
        match (self.settings_category, self.settings_row) {
            (SettingsCategory::System, 0) => {
                self.full_redraw_requested = true;
                self.preferences.taskbar_visible = !self.preferences.taskbar_visible;
                let visible = self.preferences.taskbar_visible;
                let _ = self
                    .server
                    .set_visible(DISPLAY_FIN, self.panel_surface, visible);
                let _ = self.server.commit(DISPLAY_FIN, self.panel_surface);
                if self.fullscreen {
                    self.set_app_geometry(
                        self.active,
                        Rect::new(
                            10,
                            10,
                            framebuffer::width() as u16 - 20,
                            self.work_area_bottom() as u16 - 20,
                        ),
                    );
                }
                slog!(
                    "HEXA_SETTING_CHANGED key=taskbar value={}\r\n",
                    if visible { "on" } else { "off" }
                );
                self.settings_notice = "Taskbar visibility updated.";
            }
            (SettingsCategory::System, 1) | (SettingsCategory::Network, 1) => {
                self.full_redraw_requested = true;
                self.preferences.status_visible = !self.preferences.status_visible;
                slog!(
                    "HEXA_SETTING_CHANGED key=status-indicator value={}\r\n",
                    if self.preferences.status_visible {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Status area visibility updated.";
            }
            (SettingsCategory::Appearance, 0) => {
                self.full_redraw_requested = true;
                self.preferences.theme = self.preferences.theme.shifted(direction);
                slog!(
                    "HEXA_SETTING_CHANGED key=theme value={}\r\n",
                    self.preferences.theme.label()
                );
                self.settings_notice = "Desktop theme updated.";
            }
            (SettingsCategory::Appearance, 1) => {
                self.full_redraw_requested = true;
                self.preferences.wallpaper = self.preferences.wallpaper.shifted(direction);
                slog!(
                    "HEXA_SETTING_CHANGED key=wallpaper value={}\r\n",
                    self.preferences.wallpaper.label()
                );
                self.settings_notice = "Wallpaper updated.";
            }
            (SettingsCategory::Appearance, 2) => {
                self.full_redraw_requested = true;
                self.preferences.accent = self.preferences.accent.shifted(direction);
                self.cursor
                    .set_style(self.preferences.cursor, self.preferences.accent.color());
                slog!(
                    "HEXA_SETTING_CHANGED key=accent value={}\r\n",
                    self.preferences.accent.label()
                );
                self.settings_notice = "Accent color updated.";
            }
            (SettingsCategory::Appearance, 3) => {
                self.full_redraw_requested = true;
                self.preferences.backdrop = self.preferences.backdrop.shifted(direction);
                slog!(
                    "HEXA_SETTING_CHANGED key=background-tone value={}\r\n",
                    self.preferences.backdrop.label()
                );
                self.settings_notice = "Wallpaper tone updated.";
            }
            (SettingsCategory::Appearance, 4) => {
                self.full_redraw_requested = true;
                self.preferences.pure_black_apps = !self.preferences.pure_black_apps;
                slog!(
                    "HEXA_SETTING_CHANGED key=pure-black-apps value={}\r\n",
                    if self.preferences.pure_black_apps {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Application background updated.";
            }
            (SettingsCategory::Appearance, 5) => {
                self.full_redraw_requested = true;
                self.preferences.rounded_controls = !self.preferences.rounded_controls;
                slog!(
                    "HEXA_SETTING_CHANGED key=rounded-controls value={}\r\n",
                    if self.preferences.rounded_controls {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Control shape updated.";
            }
            (SettingsCategory::Network, 0) => {
                self.full_redraw_requested = true;
                self.change_network_policy();
            }
            (SettingsCategory::Network, 2) => {
                self.full_redraw_requested = true;
                self.change_radio_policy(radio::RadioKind::Wifi);
            }
            (SettingsCategory::Bluetooth, 0) => {
                self.full_redraw_requested = true;
                self.change_radio_policy(radio::RadioKind::Bluetooth)
            }
            (SettingsCategory::Display, 0) => {
                let selected = shift_display_mode(framebuffer::requested_mode(), direction);
                let _ = framebuffer::request_mode(selected);
                slog!(
                    "HEXA_SETTING_CHANGED key=resolution value={}\r\n",
                    selected.label()
                );
                self.settings_notice = "Resolution applies when the desktop is reopened.";
            }
            (SettingsCategory::Display, 1) => {
                self.preferences.refresh_rate =
                    shift_refresh_rate(self.preferences.refresh_rate, direction);
                self.reconfigure_presentation();
                slog!(
                    "HEXA_SETTING_CHANGED key=refresh-rate value={}\r\n",
                    refresh_rate_label(self.preferences.refresh_rate)
                );
                self.settings_notice = "Compositor presentation rate updated.";
            }
            (SettingsCategory::Display, 2) => {
                self.preferences.vsync = !self.preferences.vsync;
                self.reconfigure_presentation();
                slog!(
                    "HEXA_SETTING_CHANGED key=vsync value={}\r\n",
                    if self.preferences.vsync { "on" } else { "off" }
                );
                self.settings_notice = "Page-flip synchronization updated.";
            }
            (SettingsCategory::Display, 4) => {
                self.full_redraw_requested = true;
                self.preferences.window_borders = !self.preferences.window_borders;
                slog!(
                    "HEXA_SETTING_CHANGED key=window-borders value={}\r\n",
                    if self.preferences.window_borders {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Window border rendering updated.";
            }
            (SettingsCategory::Display, 5) => {
                self.full_redraw_requested = true;
                self.preferences.high_contrast = !self.preferences.high_contrast;
                slog!(
                    "HEXA_SETTING_CHANGED key=high-contrast value={}\r\n",
                    if self.preferences.high_contrast {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Contrast rendering updated.";
            }
            (SettingsCategory::Input, 0) => {
                self.preferences.pointer_speed = if direction < 0 {
                    match self.preferences.pointer_speed {
                        1 => 3,
                        speed => speed - 1,
                    }
                } else {
                    match self.preferences.pointer_speed {
                        1 | 2 => self.preferences.pointer_speed + 1,
                        _ => 1,
                    }
                };
                slog!(
                    "HEXA_SETTING_CHANGED key=pointer-speed value={}\r\n",
                    self.preferences.pointer_speed_label()
                );
                self.settings_notice = "Pointer speed updated.";
            }
            (SettingsCategory::Input, 1) => {
                self.preferences.cursor = self.preferences.cursor.shifted(direction);
                self.cursor
                    .set_style(self.preferences.cursor, self.preferences.accent.color());
                slog!(
                    "HEXA_SETTING_CHANGED key=cursor-theme value={}\r\n",
                    self.preferences.cursor.label()
                );
                self.settings_notice = "Cursor theme updated.";
            }
            (SettingsCategory::Privacy, 0) => {
                self.browser_line.fill(0);
                self.browser_len = 0;
                self.browser_editing = false;
                self.navigate("hexa://home", HOME);
                slog!("HEXA_SETTING_CHANGED key=browser-data value=cleared\r\n");
                self.settings_notice = "Browser session data cleared.";
            }
            _ => {}
        }
        self.save_preferences();
    }

    fn handle_browser_key(&mut self, key: u8) -> bool {
        if !self.browser_editing {
            return false;
        }
        match key {
            b'\n' => {
                let mut value = [0_u8; 512];
                value[..self.browser_len].copy_from_slice(&self.browser_line[..self.browser_len]);
                let address = core::str::from_utf8(&value[..self.browser_len]).unwrap_or("");
                self.navigate_address(address);
                self.frame_pacer.reset_phase(crate::hardware::timestamp());
                self.browser_editing = false;
            }
            0x08 => self.browser_len = self.browser_len.saturating_sub(1),
            byte if (byte.is_ascii_graphic() || byte == b' ')
                && self.browser_len < self.browser_line.len() =>
            {
                self.browser_line[self.browser_len] = byte;
                self.browser_len += 1;
            }
            _ => return false,
        }
        true
    }

    fn handle_notes_key(&mut self, key: u8) -> bool {
        match key {
            0x08 => self.notes_len = self.notes_len.saturating_sub(1),
            b'\n' if self.notes_len < self.notes.len() => {
                self.notes[self.notes_len] = b'\n';
                self.notes_len += 1;
            }
            b'\t' if self.notes_len + 4 <= self.notes.len() => {
                self.notes[self.notes_len..self.notes_len + 4].copy_from_slice(b"    ");
                self.notes_len += 4;
            }
            byte if (byte.is_ascii_graphic() || byte == b' ')
                && self.notes_len < self.notes.len() =>
            {
                self.notes[self.notes_len] = byte;
                self.notes_len += 1;
            }
            _ => return false,
        }
        true
    }

    fn terminal_clear(&mut self) {
        self.terminal_output_len.fill(0);
        self.terminal_output_next = 0;
        self.terminal_output_count = 0;
    }

    fn terminal_push(&mut self, value: &str) {
        self.terminal_push_bytes(value.as_bytes());
    }

    fn terminal_push_bytes(&mut self, value: &[u8]) {
        if value.is_empty() {
            self.terminal_store_line(&[]);
            return;
        }
        for source_line in value.split(|byte| *byte == b'\n') {
            if source_line.is_empty() {
                self.terminal_store_line(&[]);
                continue;
            }
            for chunk in source_line.chunks(TERMINAL_OUTPUT_CAPACITY) {
                self.terminal_store_line(chunk);
            }
        }
    }

    fn terminal_store_line(&mut self, value: &[u8]) {
        let index = self.terminal_output_next;
        self.terminal_output[index].fill(0);
        let count = value.len().min(TERMINAL_OUTPUT_CAPACITY);
        self.terminal_output[index][..count].copy_from_slice(&value[..count]);
        self.terminal_output_len[index] = count as u8;
        self.terminal_output_next = (index + 1) % TERMINAL_SCROLLBACK;
        self.terminal_output_count = (self.terminal_output_count + 1).min(TERMINAL_SCROLLBACK);
    }

    fn terminal_push_parts(&mut self, parts: &[&str]) {
        let mut line = [0_u8; TERMINAL_OUTPUT_CAPACITY];
        let mut length = 0;
        for part in parts {
            let bytes = part.as_bytes();
            let count = bytes.len().min(line.len().saturating_sub(length));
            line[length..length + count].copy_from_slice(&bytes[..count]);
            length += count;
        }
        self.terminal_push_bytes(&line[..length]);
    }

    fn terminal_push_number(&mut self, prefix: &str, value: u64, suffix: &str) {
        let mut line = [0_u8; TERMINAL_OUTPUT_CAPACITY];
        let mut length = append_bytes(&mut line, 0, prefix.as_bytes());
        length = append_decimal(&mut line, length, value);
        length = append_bytes(&mut line, length, suffix.as_bytes());
        self.terminal_push_bytes(&line[..length]);
    }

    fn terminal_print_help(&mut self) {
        self.terminal_push("ExpOS terminal commands:");
        self.terminal_push("  help clear status version hostname pwd whoami id uname uptime");
        self.terminal_push("  users display resolution network netstat storage theme history");
        self.terminal_push("  apps ls ps echo <text>");
        self.terminal_push("  open <app> close exit shell");
        self.terminal_push("Use Up/Down for command history.");
    }

    fn terminal_print_status(&mut self) {
        self.terminal_push("ExpOS desktop is ready.");
        let mut user = [0_u8; 24];
        let user_length = self.session.name().len().min(user.len());
        user[..user_length].copy_from_slice(&self.session.name().as_bytes()[..user_length]);
        let user = core::str::from_utf8(&user[..user_length]).unwrap_or("unknown");
        let authority = self.session.authority_name();
        self.terminal_push_parts(&["user: ", user, " (", authority, ")"]);
        let mode = framebuffer::current_mode();
        self.terminal_push_parts(&["display: ", mode.label()]);
        let connectivity = radio::snapshot();
        self.terminal_push_parts(&[
            "network: ",
            if connectivity.network_enabled {
                if connectivity.ethernet.connected() {
                    "connected"
                } else {
                    "enabled, link down"
                }
            } else {
                "disabled"
            },
        ]);
        if let Some(generation) = state::loaded_generation() {
            self.terminal_push_number("saved state generation: ", generation, "");
        } else {
            self.terminal_push("saved state: defaults (first commit pending)");
        }
    }

    fn terminal_print_users(&mut self) {
        self.terminal_push("Local accounts:");
        crate::session::visit_accounts(|name, authority| {
            let authority = match authority {
                Authority::Operator => "Operator",
                Authority::Power => "Power",
                Authority::Guest => "Guest",
            };
            self.terminal_push_parts(&["  ", name, "  ", authority]);
        });
    }

    fn terminal_print_display(&mut self) {
        let current =
            framebuffer::DisplayMode::from_dimensions(framebuffer::width(), framebuffer::height())
                .unwrap_or_else(framebuffer::current_mode);
        let requested = framebuffer::requested_mode();
        let (width, height) = current.dimensions();
        self.terminal_push_parts(&["active preset: ", current.label()]);
        self.terminal_push_number("width: ", width as u64, " px");
        self.terminal_push_number("height: ", height as u64, " px");
        self.terminal_push_number("stride: ", framebuffer::stride_bytes() as u64, " bytes");
        self.terminal_push_parts(&["next desktop preset: ", requested.label()]);
        self.terminal_push_number(
            "presentation target: ",
            self.preferences.refresh_rate.hz() as u64,
            " Hz",
        );
        self.terminal_push_parts(&["vsync: ", if self.preferences.vsync { "on" } else { "off" }]);
        let pacing = self.frame_pacer.stats();
        let scanout = framebuffer::presentation_stats();
        self.terminal_push_number("frames presented: ", scanout.frames, "");
        self.terminal_push_number("pacing misses: ", pacing.missed_frames, "");
        self.terminal_push_number("vblank timeouts: ", scanout.vblank_timeouts, "");
    }

    fn terminal_print_network(&mut self) {
        let connectivity = radio::snapshot();
        self.terminal_push_parts(&[
            "packet policy: ",
            if connectivity.network_enabled {
                "on"
            } else {
                "off"
            },
        ]);
        self.terminal_push_parts(&["ethernet: ", connectivity.ethernet.status_text()]);
        self.terminal_push_parts(&["wifi: ", connectivity.wifi.status_text()]);
        self.terminal_push_parts(&["bluetooth: ", connectivity.bluetooth.status_text()]);
        self.terminal_push_parts(&[
            "rtl8139: ",
            if network::available() {
                if network::link_up() {
                    "link up"
                } else {
                    "link down"
                }
            } else {
                "not detected"
            },
        ]);
    }

    fn terminal_print_history(&mut self) {
        if self.terminal_history_count == 0 {
            self.terminal_push("No command history yet.");
            return;
        }
        for oldest_index in (0..self.terminal_history_count).rev() {
            let Some(command) = self.terminal_history_entry(oldest_index) else {
                continue;
            };
            let mut copy = [0_u8; TERMINAL_CAPACITY];
            let count = command.len().min(copy.len());
            copy[..count].copy_from_slice(&command.as_bytes()[..count]);
            self.terminal_push_bytes(&copy[..count]);
        }
    }

    fn terminal_print_running_apps(&mut self) {
        self.terminal_push("Running applications:");
        let open = self.app_open;
        let minimized = self.app_minimized;
        let mut count = 0;
        for app in AppKind::ALL {
            if open[app.index()] {
                self.terminal_push_parts(&[
                    "  ",
                    app.label(),
                    if minimized[app.index()] {
                        " (minimized)"
                    } else {
                        ""
                    },
                ]);
                count += 1;
            }
        }
        if count == 0 {
            self.terminal_push("  none");
        }
    }

    fn terminal_print_theme(&mut self) {
        self.terminal_push_parts(&["theme: ", self.preferences.theme.label()]);
        self.terminal_push_parts(&["wallpaper: ", self.preferences.wallpaper.label()]);
        self.terminal_push_parts(&["cursor: ", self.preferences.cursor.label()]);
        self.terminal_push_parts(&["accent: ", self.preferences.accent.label()]);
    }

    fn terminal_print_storage(&mut self) {
        if let Some(generation) = state::loaded_generation() {
            self.terminal_push_number("persistent journal generation: ", generation, "");
        } else if state::persistent_available() {
            self.terminal_push("persistent state disk: blank");
        } else {
            self.terminal_push("persistent state disk: unavailable");
        }
        self.terminal_push("stores: accounts and desktop preferences");
    }

    fn execute_terminal_command(&mut self, command: &[u8]) {
        let (name, arguments) = split_command(command);
        if let Ok(name) = core::str::from_utf8(name) {
            slog!("HEXA_TERMINAL_COMMAND name={}\r\n", name);
        }
        if name.is_empty() || name.eq_ignore_ascii_case(b"help") {
            self.terminal_print_help();
        } else if name.eq_ignore_ascii_case(b"clear") {
            self.terminal_clear();
        } else if name.eq_ignore_ascii_case(b"status") {
            self.terminal_print_status();
        } else if name.eq_ignore_ascii_case(b"version") {
            self.terminal_push_parts(&["ExpOS ", env!("CARGO_PKG_VERSION")]);
        } else if name.eq_ignore_ascii_case(b"hostname") {
            self.terminal_push("expos");
        } else if name.eq_ignore_ascii_case(b"pwd") {
            self.terminal_push("Stable:/");
        } else if name.eq_ignore_ascii_case(b"whoami") {
            let name = self.session.name();
            let mut copy = [0_u8; TERMINAL_CAPACITY];
            let count = name.len().min(copy.len());
            copy[..count].copy_from_slice(&name.as_bytes()[..count]);
            self.terminal_push_bytes(&copy[..count]);
        } else if name.eq_ignore_ascii_case(b"id") {
            let mut user = [0_u8; 24];
            let user_length = self.session.name().len().min(user.len());
            user[..user_length].copy_from_slice(&self.session.name().as_bytes()[..user_length]);
            let user = core::str::from_utf8(&user[..user_length]).unwrap_or("unknown");
            let authority = self.session.authority_name();
            self.terminal_push_parts(&["user=", user, " authority=", authority]);
        } else if name.eq_ignore_ascii_case(b"uname") {
            self.terminal_push("ExpOS hexa-kernel x86_64");
        } else if name.eq_ignore_ascii_case(b"uptime") {
            self.terminal_push_number("monotonic ticks: ", crate::hardware::timestamp(), "");
        } else if name.eq_ignore_ascii_case(b"users") {
            self.terminal_print_users();
        } else if name.eq_ignore_ascii_case(b"display") || name.eq_ignore_ascii_case(b"resolution")
        {
            self.terminal_print_display();
        } else if name.eq_ignore_ascii_case(b"network") || name.eq_ignore_ascii_case(b"netstat") {
            self.terminal_print_network();
        } else if name.eq_ignore_ascii_case(b"storage") {
            self.terminal_print_storage();
        } else if name.eq_ignore_ascii_case(b"theme") {
            self.terminal_print_theme();
        } else if name.eq_ignore_ascii_case(b"history") {
            self.terminal_print_history();
        } else if name.eq_ignore_ascii_case(b"apps") || name.eq_ignore_ascii_case(b"ls") {
            self.terminal_push("browser terminal forms packages settings system games notes");
        } else if name.eq_ignore_ascii_case(b"ps") {
            self.terminal_print_running_apps();
        } else if name.eq_ignore_ascii_case(b"echo") {
            self.terminal_push_bytes(arguments);
        } else if name.eq_ignore_ascii_case(b"open") {
            if let Some(app) = parse_app(arguments) {
                self.terminal_push_parts(&["Opening ", app.label(), "."]);
                self.switch_to(app);
            } else {
                self.terminal_push(
                    "usage: open <browser|forms|packages|settings|system|games|notes>",
                );
            }
        } else if name.eq_ignore_ascii_case(b"close") {
            self.close_active();
        } else if name.eq_ignore_ascii_case(b"exit") || name.eq_ignore_ascii_case(b"shell") {
            self.should_exit = true;
        } else {
            self.terminal_push("Unknown command. Type help.");
        }
    }

    fn handle_terminal_key(&mut self, key: u8) {
        match key {
            b'\n' => {
                let command = self.terminal_line;
                let command_len = self.terminal_len;
                self.record_terminal_history(&command[..command_len]);
                self.terminal_len = 0;
                self.terminal_history_cursor = None;
                let trimmed = trim_ascii(&command[..command_len]);
                if !trimmed.is_empty() {
                    let mut echoed = [0_u8; TERMINAL_OUTPUT_CAPACITY];
                    echoed[0] = b'$';
                    echoed[1] = b' ';
                    let count = trimmed.len().min(echoed.len() - 2);
                    echoed[2..2 + count].copy_from_slice(&trimmed[..count]);
                    self.terminal_push_bytes(&echoed[..2 + count]);
                }
                self.execute_terminal_command(trimmed);
            }
            0x08 => {
                self.terminal_len = self.terminal_len.saturating_sub(1);
                self.terminal_history_cursor = None;
            }
            byte if (byte.is_ascii_graphic() || byte == b' ')
                && self.terminal_len < self.terminal_line.len() =>
            {
                self.terminal_line[self.terminal_len] = byte;
                self.terminal_len += 1;
                self.terminal_history_cursor = None;
            }
            _ => {}
        }
    }

    fn record_terminal_history(&mut self, command: &[u8]) {
        let command = trim_ascii(command);
        if command.is_empty() {
            return;
        }
        let count = command.len().min(self.terminal_line.len());
        self.terminal_history[self.terminal_history_next][..count]
            .copy_from_slice(&command[..count]);
        self.terminal_history_len[self.terminal_history_next] = count as u8;
        self.terminal_history_next = (self.terminal_history_next + 1) % TERMINAL_HISTORY;
        self.terminal_history_count = (self.terminal_history_count + 1).min(TERMINAL_HISTORY);
    }

    fn recall_terminal_history(&mut self, older: bool) {
        if self.terminal_history_count == 0 {
            return;
        }
        let cursor = if older {
            self.terminal_history_cursor
                .map(|cursor| (cursor + 1).min(self.terminal_history_count - 1))
                .unwrap_or(0)
        } else {
            let Some(cursor) = self.terminal_history_cursor else {
                return;
            };
            if cursor == 0 {
                self.terminal_len = 0;
                self.terminal_history_cursor = None;
                return;
            }
            cursor - 1
        };
        let index = (self.terminal_history_next + TERMINAL_HISTORY - 1 - cursor) % TERMINAL_HISTORY;
        let count = self.terminal_history_len[index] as usize;
        self.terminal_line[..count].copy_from_slice(&self.terminal_history[index][..count]);
        self.terminal_len = count;
        self.terminal_history_cursor = Some(cursor);
    }

    fn terminal_history_entry(&self, reverse_index: usize) -> Option<&str> {
        if reverse_index >= self.terminal_history_count {
            return None;
        }
        let index =
            (self.terminal_history_next + TERMINAL_HISTORY - 1 - reverse_index) % TERMINAL_HISTORY;
        let count = self.terminal_history_len[index] as usize;
        core::str::from_utf8(&self.terminal_history[index][..count]).ok()
    }

    fn terminal_output_entry(&self, reverse_index: usize) -> Option<&str> {
        if reverse_index >= self.terminal_output_count {
            return None;
        }
        let index = (self.terminal_output_next + TERMINAL_SCROLLBACK - 1 - reverse_index)
            % TERMINAL_SCROLLBACK;
        let count = self.terminal_output_len[index] as usize;
        core::str::from_utf8(&self.terminal_output[index][..count]).ok()
    }
}

pub fn run(input: &mut Input, start_browser: bool, session: crate::session::Session) {
    run_with_network(input, start_browser, session, true);
}

pub fn run_with_network(
    input: &mut Input,
    start_browser: bool,
    session: crate::session::Session,
    allow_network: bool,
) {
    run_session(
        input,
        if start_browser {
            Some(AppKind::Browser)
        } else {
            None
        },
        session,
        allow_network,
    );
}

pub fn run_games(input: &mut Input, session: crate::session::Session) {
    run_session(input, Some(AppKind::Games), session, false);
}

fn run_session(
    input: &mut Input,
    start_app: Option<AppKind>,
    session: crate::session::Session,
    allow_network: bool,
) {
    let mouse_ready = input.enable_mouse();
    if !framebuffer::enter() {
        crate::println!("HexaDisplay unavailable: no Bochs/QEMU VBE framebuffer.");
        slog!("HEXA_DISPLAY_UNAVAILABLE\r\n");
        return;
    }

    let mut desktop = DesktopState::new(start_app, session, allow_network);
    // Desktop construction can include capability setup and document parsing;
    // begin presentation timing only when the first frame is ready to draw.
    desktop
        .frame_pacer
        .reset_phase(crate::hardware::timestamp());
    render(&mut desktop);
    slog!("HEXA_DISPLAY_READY surfaces=11 commit=11\r\n");
    if start_app.is_none() {
        slog!("HEXA_DESKTOP_EMPTY open_apps=0 pinned_apps=0\r\n");
    }
    slog!("HEXA_MOUSE_READY enabled={}\r\n", mouse_ready);
    slog!(
        "HEXA_DESKTOP_PREFS theme={} wallpaper={} cursor={} accent={}\r\n",
        desktop.preferences.theme.label(),
        desktop.preferences.wallpaper.label(),
        desktop.preferences.cursor.label(),
        desktop.preferences.accent.label()
    );
    let clock = crate::hardware::clock_info();
    slog!(
        "HEXA_PRESENTATION_READY rate={} vsync={} pageflip={} clock_hz={} source={}\r\n",
        refresh_rate_label(desktop.preferences.refresh_rate),
        desktop.preferences.vsync,
        framebuffer::presentation_stats().page_flip_available,
        clock.tsc_hz,
        clock.source.label()
    );

    while !desktop.should_exit {
        let Some(event) = input.poll_event() else {
            let now = crate::hardware::timestamp();
            if desktop.active == AppKind::Games
                && desktop.app_is_visible(AppKind::Games)
                && desktop.games.tick(now)
            {
                render_active_window(&mut desktop);
            } else {
                let _ = desktop.frame_pacer.decide(now, false);
            }
            core::hint::spin_loop();
            continue;
        };
        let InputEvent::Key(key) = event else {
            if let InputEvent::Pointer(pointer) = event {
                match desktop.handle_pointer(pointer) {
                    PointerRender::None => {}
                    PointerRender::Cursor(damage) => {
                        present_frame_damage(&mut desktop, &[damage]);
                    }
                    PointerRender::Full => render(&mut desktop),
                }
            }
            continue;
        };
        desktop.route_key(key);

        if key == 0x1B {
            if desktop.browser_editing {
                desktop.browser_editing = false;
                render_active_window(&mut desktop);
                continue;
            }
            if desktop.launcher_open {
                desktop.close_launcher();
                if desktop.has_active_window() {
                    let _ = desktop.server.focus(desktop.active_surface());
                }
                render(&mut desktop);
                continue;
            }
            break;
        }
        if key == b'\x60' || key == b'~' || key == KEY_SUPER_LAUNCHER {
            desktop.toggle_launcher();
            render(&mut desktop);
            continue;
        }
        if desktop.launcher_open {
            if let Some(app) = launcher_shortcut(key) {
                desktop.switch_to(app);
            }
            render(&mut desktop);
            continue;
        }
        if desktop.active == AppKind::Games
            && desktop.app_is_visible(AppKind::Games)
            && desktop.games.handle_key(key)
        {
            render(&mut desktop);
            continue;
        }
        if desktop.active == AppKind::Browser
            && desktop.app_is_visible(AppKind::Browser)
            && desktop.handle_browser_key(key)
        {
            render_active_window(&mut desktop);
            continue;
        }
        if desktop.active == AppKind::Notes
            && desktop.app_is_visible(AppKind::Notes)
            && desktop.handle_notes_key(key)
        {
            render_active_window(&mut desktop);
            continue;
        }
        if desktop.active == AppKind::Settings
            && desktop.app_is_visible(AppKind::Settings)
            && desktop.handle_settings_key(key)
        {
            if desktop.full_redraw_requested {
                render(&mut desktop);
            } else {
                render_active_window(&mut desktop);
            }
            continue;
        }
        match key {
            KEY_SUPER_TERMINAL => desktop.switch_to(AppKind::Terminal),
            KEY_SUPER_BROWSER => desktop.switch_to(AppKind::Browser),
            KEY_SUPER_CLOSE => desktop.close_active(),
            KEY_SUPER_CYCLE | KEY_SUPER_LEFT | KEY_SUPER_RIGHT => desktop.cycle_app(),
            KEY_SUPER_FULLSCREEN => desktop.toggle_fullscreen(),
            KEY_SUPER_UP if !desktop.fullscreen && desktop.app_is_visible(desktop.active) => {
                desktop.toggle_fullscreen()
            }
            KEY_SUPER_DOWN if desktop.fullscreen => desktop.toggle_fullscreen(),
            KEY_SUPER_DOWN if desktop.app_is_visible(desktop.active) => desktop.minimize_active(),
            b'\t' => desktop.cycle_app(),
            KEY_UP if desktop.active == AppKind::Terminal => {
                desktop.recall_terminal_history(true);
                render_active_window(&mut desktop);
                continue;
            }
            KEY_DOWN if desktop.active == AppKind::Terminal => {
                desktop.recall_terminal_history(false);
                render_active_window(&mut desktop);
                continue;
            }
            KEY_LEFT => desktop.move_active(-12, 0),
            KEY_RIGHT => desktop.move_active(12, 0),
            KEY_UP => desktop.move_active(0, -12),
            KEY_DOWN => desktop.move_active(0, 12),
            _ if desktop.active == AppKind::Terminal
                && desktop.app_is_visible(AppKind::Terminal) =>
            {
                desktop.handle_terminal_key(key);
                if desktop.app_is_visible(AppKind::Terminal) {
                    render_active_window(&mut desktop);
                } else {
                    render(&mut desktop);
                }
                continue;
            }
            b'q' | b'Q' => break,
            _ => {
                if let Some(app) = app_shortcut(key) {
                    desktop.switch_to(app);
                } else if desktop.active == AppKind::Browser {
                    match key.to_ascii_lowercase() {
                        b'/' | b'l' => {
                            desktop.browser_len = 0;
                            desktop.browser_editing = true;
                        }
                        b'h' => desktop.navigate("hexa://home", HOME),
                        b'1' | b'a' => desktop.navigate("hexa://about", ABOUT),
                        b'2' => desktop.navigate("hexa://packages", BROWSER_PACKAGES),
                        b'3' => desktop.navigate("hexa://system", BROWSER_SYSTEM),
                        b'n' => desktop.navigate("hexa://blocked", NETWORK_BLOCKED),
                        _ => {}
                    }
                }
            }
        }
        render(&mut desktop);
    }

    let pacing = desktop.frame_pacer.stats();
    let presentation = framebuffer::presentation_stats();
    slog!(
        "HEXA_PRESENTATION_STATS frames={} missed={} idle={} vblank_timeouts={}\r\n",
        presentation.frames,
        pacing.missed_frames,
        pacing.idle_frames,
        presentation.vblank_timeouts
    );
    framebuffer::exit();
    crate::clear_console();
    crate::println!("HexaDisplay session closed; command environment restored.");
    slog!("HEXA_DISPLAY_CLOSED\r\n");
}

fn app_shortcut(key: u8) -> Option<AppKind> {
    match key.to_ascii_lowercase() {
        b'b' => Some(AppKind::Browser),
        b't' => Some(AppKind::Terminal),
        b'f' => Some(AppKind::Forms),
        b'p' => Some(AppKind::Packages),
        b's' => Some(AppKind::Settings),
        b'i' => Some(AppKind::System),
        b'g' => Some(AppKind::Games),
        b'n' => Some(AppKind::Notes),
        _ => None,
    }
}

fn launcher_shortcut(key: u8) -> Option<AppKind> {
    match key.to_ascii_lowercase() {
        b'b' | b'1' => Some(AppKind::Browser),
        b't' | b'2' => Some(AppKind::Terminal),
        b'f' | b'3' => Some(AppKind::Forms),
        b'p' | b'4' => Some(AppKind::Packages),
        b's' | b'5' => Some(AppKind::Settings),
        b'i' | b'6' => Some(AppKind::System),
        b'g' | b'7' => Some(AppKind::Games),
        b'n' | b'8' => Some(AppKind::Notes),
        _ => None,
    }
}

fn split_command(command: &[u8]) -> (&[u8], &[u8]) {
    let command = trim_ascii(command);
    let split = command
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(command.len());
    let name = &command[..split];
    let arguments = if split < command.len() {
        trim_ascii(&command[split + 1..])
    } else {
        &[]
    };
    (name, arguments)
}

fn parse_app(value: &[u8]) -> Option<AppKind> {
    let value = trim_ascii(value);
    if value.eq_ignore_ascii_case(b"browser") || value.eq_ignore_ascii_case(b"b") {
        Some(AppKind::Browser)
    } else if value.eq_ignore_ascii_case(b"terminal") || value.eq_ignore_ascii_case(b"t") {
        Some(AppKind::Terminal)
    } else if value.eq_ignore_ascii_case(b"forms") || value.eq_ignore_ascii_case(b"f") {
        Some(AppKind::Forms)
    } else if value.eq_ignore_ascii_case(b"packages") || value.eq_ignore_ascii_case(b"p") {
        Some(AppKind::Packages)
    } else if value.eq_ignore_ascii_case(b"settings") || value.eq_ignore_ascii_case(b"s") {
        Some(AppKind::Settings)
    } else if value.eq_ignore_ascii_case(b"system") || value.eq_ignore_ascii_case(b"i") {
        Some(AppKind::System)
    } else if value.eq_ignore_ascii_case(b"games") || value.eq_ignore_ascii_case(b"g") {
        Some(AppKind::Games)
    } else if value.eq_ignore_ascii_case(b"notes") || value.eq_ignore_ascii_case(b"n") {
        Some(AppKind::Notes)
    } else {
        None
    }
}

fn append_bytes(output: &mut [u8], offset: usize, value: &[u8]) -> usize {
    let count = value.len().min(output.len().saturating_sub(offset));
    output[offset..offset + count].copy_from_slice(&value[..count]);
    offset + count
}

fn append_decimal(output: &mut [u8], offset: usize, mut value: u64) -> usize {
    let mut digits = [0_u8; 20];
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    append_bytes(output, offset, &digits[start..])
}

fn trim_ascii(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

fn buffer(id: u32, owner: Fin, width: u16, height: u16) -> BufferHandle {
    BufferHandle {
        id,
        owner,
        width,
        height,
        format: BufferFormat::Xrgb8888,
    }
}

fn default_rect(app: AppKind) -> Rect {
    let offset = (app.index() % 4) as i16;
    let (app_width, app_height) = app_dimensions();
    let centered_x = ((framebuffer::width() as i32 - app_width as i32) / 2).max(4) as i16;
    let centered_y = ((taskbar_y() as i32 - app_height as i32) / 2).max(4) as i16;
    let max_x = (framebuffer::width() as i16 - app_width as i16).max(4);
    let max_y = (taskbar_y() - app_height as i16).max(4);
    Rect::new(
        (centered_x + offset * 18).min(max_x),
        (centered_y + offset * 8).min(max_y),
        app_width,
        app_height,
    )
}

fn draw_wallpaper(preferences: DesktopPreferences) {
    let width = framebuffer::width() as i32;
    let height = framebuffer::height() as i32;
    let base = preferences.backdrop.color();
    match preferences.wallpaper {
        WallpaperChoice::Solid => framebuffer::clear(base),
        WallpaperChoice::Gradient => {
            let (top, bottom) = match preferences.backdrop {
                BackdropChoice::Graphite => (0x0019_1D20, 0x0007_090B),
                BackdropChoice::Midnight => (0x0009_1828, 0x0002_060B),
                BackdropChoice::Black => (0x0008_0A0D, 0x0000_0000),
            };
            framebuffer::vertical_gradient(0, 0, width, height, top, bottom);
        }
        WallpaperChoice::Horizon => {
            let accent = preferences.accent.color();
            framebuffer::vertical_gradient(0, 0, width, height, 0x0005_0A12, base);
            let horizon = height * 3 / 5;
            framebuffer::alpha_rect(0, horizon - 2, width, 5, accent, 120);
            framebuffer::alpha_rect(0, horizon + 3, width, height - horizon, accent, 18);
            let mut line_y = horizon + 36;
            while line_y < height {
                framebuffer::alpha_rect(0, line_y, width, 1, accent, 38);
                line_y += 34;
            }
        }
        WallpaperChoice::Grid => {
            framebuffer::clear(base);
            let grid = preferences.theme.card();
            let mut column = 0;
            while column < width {
                framebuffer::alpha_rect(column, 0, 1, height, grid, 100);
                column += 64;
            }
            let mut row = 0;
            while row < height {
                framebuffer::alpha_rect(0, row, width, 1, grid, 100);
                row += 64;
            }
        }
        WallpaperChoice::Dusk => {
            framebuffer::vertical_gradient(0, 0, width, height, 0x0021_1832, 0x0007_0A11);
            framebuffer::alpha_rect(
                0,
                height * 2 / 3,
                width,
                height / 3,
                preferences.accent.color(),
                28,
            );
        }
        WallpaperChoice::Aurora => {
            framebuffer::vertical_gradient(0, 0, width, height, 0x0004_101B, 0x0002_060B);
            let accent = preferences.accent.color();
            let band_height = (height / 8).max(24);
            for band in 0..5 {
                let y = height / 7 + band * band_height;
                let inset = band * width / 18;
                framebuffer::rounded_rect(
                    inset - width / 5,
                    y,
                    width - inset / 2,
                    band_height + 18,
                    band_height / 2,
                    if band % 2 == 0 { accent } else { color::CYAN },
                );
                framebuffer::alpha_rect(0, y + band_height / 3, width, band_height, base, 205);
            }
        }
        WallpaperChoice::Mesh => {
            framebuffer::vertical_gradient(0, 0, width, height, base, 0x0002_0508);
            let accent = preferences.accent.color();
            let spacing = if width <= 640 { 56 } else { 88 };
            let mut offset = -height;
            while offset < width {
                framebuffer::line(offset, 0, offset + height, height, accent);
                offset += spacing;
            }
            let mut offset = 0;
            while offset < width + height {
                framebuffer::line(offset, 0, offset - height, height, preferences.theme.card());
                offset += spacing;
            }
            framebuffer::alpha_rect(0, 0, width, height, base, 155);
        }
    }
}

fn render(desktop: &mut DesktopState) {
    desktop.cursor.invalidate();
    draw_wallpaper(desktop.preferences);

    for app in AppKind::ALL {
        if app != desktop.active && desktop.app_is_visible(app) {
            draw_app(desktop, app, false, true);
        }
    }
    if desktop.app_is_visible(desktop.active) {
        draw_app(desktop, desktop.active, true, true);
    }
    if desktop.preferences.taskbar_visible {
        draw_dock(desktop);
    }
    if desktop.launcher_open {
        draw_launcher(desktop);
    }
    desktop.cursor.draw();
    desktop.drain_protocol_events();
    present_frame(desktop);
}

fn render_active_window(desktop: &mut DesktopState) {
    desktop.cursor.restore();
    let mut damage = [desktop.cursor.damage_region(); 2];
    let mut damage_count = 1;
    if desktop.app_is_visible(desktop.active) {
        let rect = desktop
            .server
            .surface(desktop.active_surface())
            .map(|surface| surface.current.rect)
            .unwrap_or_else(|| {
                let (width, height) = app_dimensions();
                Rect::new(8, 8, width, height)
            });
        damage[damage_count] = framebuffer::DamageRegion::new(
            rect.x as i32,
            rect.y as i32,
            rect.width as i32 + 10,
            rect.height as i32 + 12,
        );
        damage_count += 1;
        draw_app(desktop, desktop.active, true, false);
    }
    desktop.cursor.draw();
    desktop.drain_protocol_events();
    present_frame_damage(desktop, &damage[..damage_count]);
}

fn present_frame(desktop: &mut DesktopState) {
    pace_frame(desktop);
    framebuffer::present(desktop.preferences.vsync);
}

fn present_frame_damage(desktop: &mut DesktopState, damage: &[framebuffer::DamageRegion]) {
    pace_frame(desktop);
    framebuffer::present_damage(desktop.preferences.vsync, damage);
}

fn pace_frame(desktop: &mut DesktopState) {
    loop {
        let now = crate::hardware::timestamp();
        match desktop.frame_pacer.decide(now, true) {
            FrameDecision::WaitUntil { deadline } => {
                while crate::hardware::timestamp() < deadline.not_before_tick() {
                    core::hint::spin_loop();
                }
            }
            FrameDecision::PresentNow { missed_frames, .. } => {
                let severe_miss = desktop.preferences.refresh_rate.hz() as u64 / 2;
                if missed_frames >= severe_miss {
                    slog!("HEXA_FRAME_MISSED count={}\r\n", missed_frames);
                }
                break;
            }
            FrameDecision::ClockExhausted => {
                desktop.frame_pacer.reset_phase(now);
            }
            FrameDecision::Idle { .. } => {}
        }
    }
}

fn draw_app(desktop: &DesktopState, app: AppKind, focused: bool, draw_shadow: bool) {
    let rect = desktop
        .server
        .surface(desktop.app_surfaces[app.index()])
        .map(|surface| surface.current.rect)
        .unwrap_or_else(|| {
            let (width, height) = app_dimensions();
            Rect::new(8, 8, width, height)
        });
    draw_window(rect, app.label(), focused, desktop.preferences, draw_shadow);
    let responsive_full = matches!(
        app,
        AppKind::Browser | AppKind::Terminal | AppKind::Settings | AppKind::Notes
    ) && rect.width >= 480
        && rect.height >= 360;
    if responsive_full || (rect.width >= 600 && rect.height >= 380) {
        match app {
            AppKind::Browser => draw_browser(rect, desktop),
            AppKind::Terminal => draw_terminal(rect, desktop),
            AppKind::Forms => draw_forms(rect),
            AppKind::Packages => draw_packages(rect),
            AppKind::Settings => draw_settings(rect, desktop),
            AppKind::System => draw_system(rect, desktop),
            AppKind::Games => desktop.games.render(rect),
            AppKind::Notes => draw_notes(rect, desktop),
        }
    }
}

fn draw_window(
    rect: Rect,
    title: &str,
    focused: bool,
    preferences: DesktopPreferences,
    draw_shadow: bool,
) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    if draw_shadow && width > 24 && height > 24 {
        framebuffer::alpha_rect(x + 8, y + 10, width, height, 0x0000_0000, 105);
    }
    if preferences.rounded_controls {
        framebuffer::rounded_rect(x, y, width, height, 10, preferences.window_color());
    } else {
        framebuffer::rect(x, y, width, height, preferences.window_color());
    }
    if preferences.window_borders {
        framebuffer::outline(x, y, width, height, preferences.border_color(focused));
    }
    if focused {
        framebuffer::rect(x + 1, y + 1, width - 2, 2, preferences.accent.color());
    }
    if preferences.rounded_controls {
        framebuffer::rounded_rect(x + 1, y + 3, width - 2, 29, 8, preferences.chrome_color());
        framebuffer::rect(x + 1, y + 18, width - 2, 14, preferences.chrome_color());
    } else {
        framebuffer::rect(x + 1, y + 3, width - 2, 29, preferences.chrome_color());
    }
    framebuffer::text(x + 12, y + 13, title, color::INK, 1);
    framebuffer::line(
        x + width - 126,
        y + 3,
        x + width - 126,
        y + 31,
        color::BORDER,
    );
    framebuffer::line(x + width - 84, y + 3, x + width - 84, y + 31, color::BORDER);
    framebuffer::line(x + width - 42, y + 3, x + width - 42, y + 31, color::BORDER);
    settings_rect(
        preferences,
        x + width - 120,
        y + 7,
        32,
        21,
        5,
        preferences.card_color(),
    );
    settings_rect(
        preferences,
        x + width - 79,
        y + 7,
        32,
        21,
        5,
        preferences.card_color(),
    );
    settings_rect(
        preferences,
        x + width - 38,
        y + 7,
        30,
        21,
        5,
        if focused {
            0x0066_3038
        } else {
            preferences.card_color()
        },
    );
    framebuffer::text(x + width - 109, y + 13, "-", color::MUTED, 1);
    framebuffer::outline(x + width - 69, y + 12, 12, 8, color::MUTED);
    framebuffer::text(x + width - 28, y + 13, "x", color::INK, 1);
}

#[derive(Clone, Copy)]
struct BrowserLayout {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    text_x: i32,
    text_y: i32,
    text_width: i32,
    scale: i32,
    next_y: i32,
}

fn browser_layout(rect: Rect, styled: hexa_core::StyledNode<'_>, content_y: i32) -> BrowserLayout {
    let style = styled.style;
    let margin_left = style.margin.left as i32;
    let margin_right = style.margin.right as i32;
    let margin_top = style.margin.top as i32;
    let margin_bottom = style.margin.bottom as i32;
    let border = style.border.width.min(4) as i32;
    let padding_left = style.padding.left.max(0) as i32;
    let padding_right = style.padding.right.max(0) as i32;
    let padding_top = style.padding.top.max(0) as i32;
    let padding_bottom = style.padding.bottom.max(0) as i32;
    let scale = match style.font_size {
        0..=18 => 1,
        19..=31 => 2,
        _ => 3,
    };
    let button = styled.tag.eq_ignore_ascii_case("button");
    let prefix = if button {
        0
    } else if matches!(styled.node.kind, NodeKind::Link | NodeKind::ListItem) {
        18
    } else {
        0
    };
    let x = rect.x as i32 + 34 + margin_left;
    let available_width = (rect.width as i32 - 68 - margin_left - margin_right).max(48);
    let width = if button {
        available_width.min(280)
    } else {
        available_width
    };
    let text_width = (width - border * 2 - padding_left - padding_right - prefix).max(24);
    let text_height = browser_wrapped_height(styled.node.text.as_str(), text_width, scale);
    let minimum_height = if button { 30 } else { 0 };
    let height = (border * 2 + padding_top + text_height + padding_bottom).max(minimum_height);
    let y = content_y + margin_top;
    let mut text_x = x + border + padding_left + prefix;
    let text_y = y
        + border
        + padding_top
        + (height - border * 2 - padding_top - padding_bottom - text_height) / 2;
    let one_line_width = styled.node.text.as_str().len() as i32 * framebuffer::text_advance(scale);
    if one_line_width <= text_width {
        text_x += match style.text_align {
            TextAlign::Center => (text_width - one_line_width) / 2,
            TextAlign::Right => text_width - one_line_width,
            TextAlign::Left | TextAlign::Justify => 0,
        };
    }
    BrowserLayout {
        x,
        y,
        width,
        height,
        text_x,
        text_y,
        text_width,
        scale,
        next_y: y + height + margin_bottom + 8,
    }
}

fn browser_wrapped_height(value: &str, width: i32, scale: i32) -> i32 {
    let advance = framebuffer::text_advance(scale);
    let line_height = if scale <= 1 { 10 } else { 18 };
    let mut used = 0;
    let mut lines = 1;
    for word in value.split_ascii_whitespace() {
        let word_width = word.len() as i32 * advance;
        let required = if used == 0 {
            word_width
        } else {
            advance + word_width
        };
        if used != 0 && used + required > width {
            lines += 1;
            used = word_width;
        } else {
            used += required;
        }
    }
    lines * line_height
}

const fn browser_color(value: hexa_core::CssColor) -> u32 {
    ((value.red as u32) << 16) | ((value.green as u32) << 8) | value.blue as u32
}

fn draw_browser(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let bottom = y + rect.height as i32;
    framebuffer::rect(x + 18, y + 50, 40, 36, desktop.preferences.panel_color());
    framebuffer::outline(
        x + 18,
        y + 50,
        40,
        36,
        desktop.preferences.border_color(false),
    );
    framebuffer::text(x + 34, y + 64, "<", color::INK, 1);
    framebuffer::rect(
        x + 66,
        y + 50,
        width - 86,
        36,
        desktop.preferences.panel_color(),
    );
    framebuffer::outline(
        x + 66,
        y + 50,
        width - 86,
        36,
        desktop.preferences.border_color(false),
    );
    framebuffer::rect(
        x + 79,
        y + 65,
        5,
        5,
        if desktop.network_active() {
            color::GREEN
        } else {
            color::MUTED
        },
    );
    let address = if desktop.browser_editing {
        core::str::from_utf8(&desktop.browser_line[..desktop.browser_len]).unwrap_or("")
    } else {
        desktop.document.url()
    };
    let address_capacity = ((width - 116) / framebuffer::text_advance(1)).max(1) as usize;
    let (address_text, address_color) = if desktop.browser_editing && address.is_empty() {
        ("Search DuckDuckGo or enter an address", color::MUTED)
    } else if desktop.browser_editing && address.len() > address_capacity {
        (&address[address.len() - address_capacity..], color::INK)
    } else if address.len() > address_capacity {
        (&address[..address_capacity], color::INK)
    } else {
        (address, color::INK)
    };
    framebuffer::text(x + 96, y + 64, address_text, address_color, 1);
    if desktop.browser_editing {
        let caret_x =
            (x + 96 + address.len().min(address_capacity) as i32 * framebuffer::text_advance(1))
                .min(x + width - 24);
        framebuffer::rect(caret_x, y + 61, 2, 14, color::GREEN);
    }
    let mut content_y = y + 110;
    for styled in desktop.document.styled_nodes() {
        if content_y > bottom - 24 {
            break;
        }
        if styled.node.kind == NodeKind::Title || !styled.style.is_rendered() {
            continue;
        }
        let layout = browser_layout(rect, styled, content_y);
        if layout.y > bottom - 24 {
            break;
        }
        let button = styled.tag.eq_ignore_ascii_case("button");
        if styled.style.background.alpha != 0 || button {
            let background = if styled.style.background.alpha == 0 {
                desktop.preferences.accent.color()
            } else {
                browser_color(styled.style.background)
            };
            if button {
                framebuffer::rounded_rect(
                    layout.x,
                    layout.y,
                    layout.width,
                    layout.height,
                    5,
                    background,
                );
            } else if styled.style.background.alpha == u8::MAX {
                framebuffer::rect(layout.x, layout.y, layout.width, layout.height, background);
            } else {
                framebuffer::alpha_rect(
                    layout.x,
                    layout.y,
                    layout.width,
                    layout.height,
                    background,
                    styled.style.background.alpha,
                );
            }
        }
        let border_width = styled.style.border.width.min(4) as i32;
        for inset in 0..border_width {
            framebuffer::outline(
                layout.x + inset,
                layout.y + inset,
                layout.width - inset * 2,
                layout.height - inset * 2,
                browser_color(styled.style.border.color),
            );
        }
        match styled.node.kind {
            NodeKind::Title => {}
            NodeKind::ListItem => {
                framebuffer::rect(layout.x + 5, layout.text_y + 3, 5, 5, color::GREEN);
            }
            NodeKind::Link if !button => {
                framebuffer::text(layout.x + 3, layout.text_y, ">", color::GREEN, 1);
            }
            NodeKind::Heading | NodeKind::Paragraph | NodeKind::Link => {}
        }
        let ink = browser_color(styled.style.color);
        let _ = wrapped_text(
            layout.text_x,
            layout.text_y,
            layout.text_width,
            styled.node.text.as_str(),
            ink,
            layout.scale,
        );
        if styled.style.font_weight >= 600 {
            let _ = wrapped_text(
                layout.text_x + 1,
                layout.text_y,
                layout.text_width,
                styled.node.text.as_str(),
                ink,
                layout.scale,
            );
        }
        content_y = layout.next_y;
    }
}

fn draw_terminal(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(
        x + 1,
        y + 32,
        width - 2,
        height - 33,
        desktop.preferences.theme.terminal(),
    );
    let prompt_y = y + height - 34;
    let output_top = y + 48;
    let line_height = 14;
    let visible_lines = ((prompt_y - output_top - 8) / line_height).max(0) as usize;
    let lines = visible_lines.min(desktop.terminal_output_count);
    for visual_index in 0..lines {
        let reverse_index = lines - visual_index - 1;
        if let Some(line) = desktop.terminal_output_entry(reverse_index) {
            let visible_length = line
                .len()
                .min(((width - 40) / framebuffer::text_advance(1)).max(1) as usize);
            framebuffer::text(
                x + 20,
                output_top + visual_index as i32 * line_height,
                &line[..visible_length],
                if line.starts_with("$ ") {
                    desktop.preferences.accent.color()
                } else {
                    color::INK
                },
                1,
            );
        }
    }
    framebuffer::line(
        x + 14,
        prompt_y - 10,
        x + width - 14,
        prompt_y - 10,
        desktop.preferences.border_color(false),
    );
    let prompt_scale = if width < 800 { 1 } else { 2 };
    let advance = framebuffer::text_advance(prompt_scale);
    let accent = desktop.preferences.accent.color();
    framebuffer::text(
        x + 20,
        prompt_y,
        desktop.session.name(),
        accent,
        prompt_scale,
    );
    framebuffer::text(
        x + 20 + desktop.session.name().len() as i32 * advance,
        prompt_y,
        "@expos $",
        accent,
        prompt_scale,
    );
    if let Ok(line) = core::str::from_utf8(&desktop.terminal_line[..desktop.terminal_len]) {
        let input_x = x + 20 + (desktop.session.name().len() as i32 + 8) * advance;
        let visible_capacity = ((x + width - 20 - input_x) / advance).max(1) as usize;
        let visible_start = line.len().saturating_sub(visible_capacity);
        let visible = &line[visible_start..];
        framebuffer::text(input_x, prompt_y, visible, color::WHITE, prompt_scale);
        framebuffer::rect(
            input_x + visible.len() as i32 * advance,
            prompt_y,
            if prompt_scale == 1 { 5 } else { 8 },
            if prompt_scale == 1 { 9 } else { 16 },
            color::MUTED,
        );
    }
}

fn draw_notes(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(x + 12, y + 44, width - 24, 42, 0x000A_0D13);
    framebuffer::text(x + 28, y + 59, "Untitled note", color::INK, 1);
    framebuffer::rect(x + 12, y + 88, width - 24, height - 122, 0x0000_0000);
    framebuffer::outline(x + 12, y + 88, width - 24, height - 122, color::BORDER);

    if desktop.notes_len == 0 {
        framebuffer::text(x + 32, y + 112, "Start typing...", color::MUTED, 1);
        framebuffer::rect(x + 32, y + 110, 2, 17, color::PURPLE);
    } else {
        draw_note_text(
            x + 32,
            y + 112,
            width - 64,
            height - 166,
            &desktop.notes[..desktop.notes_len],
        );
    }
}

fn draw_note_text(left: i32, top: i32, width: i32, height: i32, bytes: &[u8]) {
    let mut x = left;
    let mut y = top;
    let right = left + width;
    let bottom = top + height;
    for byte in bytes.iter().copied() {
        if byte == b'\n' || x + 12 > right {
            x = left;
            y += 20;
            if byte == b'\n' {
                continue;
            }
        }
        if y + 16 > bottom {
            break;
        }
        framebuffer::glyph(x, y, byte, color::INK, 2);
        x += 12;
    }
    if y + 17 <= bottom {
        framebuffer::rect(x, y - 1, 2, 17, color::PURPLE);
    }
}

fn draw_forms(rect: Rect) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    framebuffer::text(x + 28, y + 58, "Forms", color::INK, 2);
    let rows = [
        ("ROOT", "DIMENSION", "ACTIVE", color::GREEN),
        ("AYO", "PACKAGE", "BOUND", color::CYAN),
        ("HEXADISPLAY", "SERVICE", "ACTIVE", color::GREEN),
        ("BROWSER", "INTERFACE", "FOCUSED", color::PURPLE),
        ("GO ABI V1", "INTERFACE", "READY", color::CYAN),
        ("HEXAFS", "STORAGE", "JOURNALED", color::GREEN),
    ];
    for (index, (name, kind, status, status_color)) in rows.iter().enumerate() {
        let row_y = y + 96 + index as i32 * 43;
        framebuffer::rect(x + 28, row_y, rect.width as i32 - 56, 34, 0x0015_1922);
        framebuffer::rect(x + 28, row_y, 5, 34, *status_color);
        framebuffer::text(x + 46, row_y + 12, name, color::INK, 1);
        framebuffer::text(x + 260, row_y + 12, kind, color::MUTED, 1);
        framebuffer::text(x + 494, row_y + 12, status, *status_color, 1);
    }
}

fn draw_packages(rect: Rect) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let content_width = rect.width as i32 - 56;
    framebuffer::text(x + 28, y + 58, "Packages", color::INK, 2);
    package_card(
        x + 28,
        y + 96,
        content_width,
        "Core tools",
        "Installed",
        "Diagnostics and repair",
    );
    package_card(
        x + 28,
        y + 154,
        content_width,
        "Display + Renderkit",
        "Installed",
        "Display and graphics",
    );
    package_card(
        x + 28,
        y + 212,
        content_width,
        "Games",
        "Available",
        "Native games",
    );
    package_card(
        x + 28,
        y + 270,
        content_width,
        "Desktop + Session",
        "Available",
        "Desktop and accounts",
    );
}

fn package_card(x: i32, y: i32, width: i32, name: &str, status: &str, detail: &str) {
    framebuffer::rect(x, y, width, 46, 0x0015_1922);
    framebuffer::outline(x, y, width, 46, color::BORDER);
    framebuffer::text(x + 14, y + 11, name, color::INK, 1);
    framebuffer::text(x + 178, y + 11, detail, color::MUTED, 1);
    framebuffer::text(
        x + width - 108,
        y + 28,
        status,
        if status == "Installed" {
            color::GREEN
        } else {
            color::CYAN
        },
        1,
    );
}

#[derive(Clone, Copy)]
enum SettingControl {
    Toggle { on: bool, available: bool },
    Choice,
    Status { ready: bool },
    Action { available: bool },
    Plain,
}

fn draw_settings(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    let sidebar_width = settings_sidebar_width(rect) as i32;
    let compact = settings_compact(rect);
    let category_top = settings_category_top(rect) as i32;
    let category_step = settings_category_step(rect) as i32;
    let accent = desktop.preferences.accent.color();
    let connectivity = radio::snapshot();
    let can_configure_radios = desktop.settings_radio_handle.is_some();

    framebuffer::rect(
        x + 1,
        y + 32,
        sidebar_width,
        height - 33,
        desktop.preferences.chrome_color(),
    );
    framebuffer::line(
        x + sidebar_width,
        y + 32,
        x + sidebar_width,
        y + height - 1,
        desktop.preferences.border_color(false),
    );
    framebuffer::text(
        x + 20,
        y + if compact { 44 } else { 54 },
        "Settings",
        color::INK,
        if compact { 1 } else { 2 },
    );

    for category in SettingsCategory::ALL {
        let row_y = y + category_top + category.index() as i32 * category_step;
        let selected = category == desktop.settings_category;
        if selected {
            settings_rect(
                desktop.preferences,
                x + 12,
                row_y,
                sidebar_width - 24,
                category_step - 4,
                5,
                0x001B_2225,
            );
            framebuffer::rect(x + 12, row_y + 6, 3, 26, accent);
        }
        framebuffer::text(
            x + 26,
            row_y + 14,
            category.label(),
            if selected { color::WHITE } else { color::MUTED },
            1,
        );
    }

    let content_x = x + sidebar_width + 28;
    let content_width = width - sidebar_width - 50;
    framebuffer::text(
        content_x,
        y + if compact { 45 } else { 55 },
        desktop.settings_category.label(),
        color::INK,
        if compact { 1 } else { 2 },
    );
    if !compact {
        framebuffer::text(
            content_x,
            y + 82,
            desktop.settings_category.description(),
            color::MUTED,
            1,
        );
    }

    match desktop.settings_category {
        SettingsCategory::System => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Taskbar",
                "Show running apps and the launcher button",
                if desktop.preferences.taskbar_visible {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.taskbar_visible,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Status area",
                "Show connection state on the taskbar",
                if desktop.preferences.status_visible {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.status_visible,
                    available: true,
                },
            );
        }
        SettingsCategory::Appearance => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Theme",
                "Window chrome and panel palette",
                desktop.preferences.theme.label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Wallpaper",
                "Procedural desktop artwork",
                desktop.preferences.wallpaper.label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Accent color",
                "Used for focus, selections, and switches",
                desktop.preferences.accent.label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Wallpaper tone",
                "Base color behind the wallpaper",
                desktop.preferences.backdrop.label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
                "Pure black apps",
                "Use black instead of charcoal for windows",
                if desktop.preferences.pure_black_apps {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.pure_black_apps,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                5,
                "Rounded controls",
                "Round buttons, selections, and switches",
                if desktop.preferences.rounded_controls {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.rounded_controls,
                    available: true,
                },
            );
        }
        SettingsCategory::Network => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Network access",
                "Master policy for real packet input and output",
                if connectivity.network_enabled {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: connectivity.network_enabled,
                    available: can_configure_radios,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Network indicator",
                "Show connection state in the taskbar",
                if desktop.preferences.status_visible {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.status_visible,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Wi-Fi power",
                connectivity.wifi.status_text(),
                if connectivity.wifi_connected() {
                    "Connected"
                } else if connectivity.wifi_requested {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: connectivity.wifi_requested,
                    available: can_configure_radios,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Wired network",
                "Native RTL8139 Ethernet driver",
                connectivity.ethernet.status_text(),
                SettingControl::Status {
                    ready: connectivity.ethernet.connected(),
                },
            );
        }
        SettingsCategory::Bluetooth => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Bluetooth power",
                connectivity.bluetooth.status_text(),
                if connectivity.bluetooth_connected() {
                    "Connected"
                } else if connectivity.bluetooth_requested {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: connectivity.bluetooth_requested,
                    available: can_configure_radios,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "USB controller",
                "Device enumeration needs the queued USB host stack",
                if connectivity.usb_controller_detected {
                    "Detected"
                } else {
                    "Not detected"
                },
                SettingControl::Status {
                    ready: connectivity.usb_controller_detected,
                },
            );
            framebuffer::text(content_x, y + 260, "Paired devices", color::INK, 1);
            framebuffer::text(
                content_x,
                y + 288,
                "Unavailable until a supported Bluetooth data path is active.",
                color::MUTED,
                1,
            );
        }
        SettingsCategory::Display => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Resolution",
                "480p, 720p, or 1080p; applies next desktop session",
                framebuffer::requested_mode().label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Presentation rate",
                "Frame pacing target; physical output remains host-controlled",
                refresh_rate_label(desktop.preferences.refresh_rate),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "VSync",
                "Synchronize double-buffer page flips to vertical retrace",
                if desktop.preferences.vsync {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.vsync,
                    available: framebuffer::presentation_stats().page_flip_available,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Active output",
                "Double-buffered XRGB8888 HexaDisplay composition",
                framebuffer::current_mode().label(),
                SettingControl::Status {
                    ready: framebuffer::presentation_stats().page_flip_available,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
                "Window borders",
                "Draw an outline around application windows",
                if desktop.preferences.window_borders {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.window_borders,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                5,
                "High contrast",
                "Strengthen edges and active window focus",
                if desktop.preferences.high_contrast {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.high_contrast,
                    available: true,
                },
            );
        }
        SettingsCategory::Input => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Pointer speed",
                "Scale PS/2 mouse movement",
                desktop.preferences.pointer_speed_label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Cursor theme",
                "Light, dark, accent, or crosshair pointer",
                desktop.preferences.cursor.label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Mouse",
                "PS/2 pointer with buttons and window dragging",
                "Ready",
                SettingControl::Status { ready: true },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Keyboard",
                "PS/2 and serial input with desktop shortcuts",
                "Ready",
                SettingControl::Status { ready: true },
            );
        }
        SettingsCategory::Privacy => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Clear browser data",
                "Reset the address field and current document",
                "Clear",
                SettingControl::Action { available: true },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Telemetry",
                "ExpOS does not send usage or settings data",
                "Off",
                SettingControl::Toggle {
                    on: false,
                    available: false,
                },
            );
        }
        SettingsCategory::About => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Operating system",
                "Capability-native experimental system",
                "ExpOS",
                SettingControl::Plain,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Display server",
                "Form-owned surfaces and explicit input routing",
                "HexaDisplay",
                SettingControl::Plain,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Session user",
                "Current authenticated account",
                desktop.session.name(),
                SettingControl::Plain,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Local accounts",
                "Accounts stored by the session manager",
                "Available",
                SettingControl::Plain,
            );
            framebuffer::text(
                content_x + content_width - 42,
                y + settings_row_top(rect) as i32 + 3 * settings_row_step(rect) as i32 + 31,
                "#",
                color::MUTED,
                1,
            );
            draw_number(
                content_x + content_width - 28,
                y + settings_row_top(rect) as i32 + 3 * settings_row_step(rect) as i32 + 31,
                crate::session::account_count() as u64,
                color::CYAN,
            );
        }
    }

    if height >= 560 {
        framebuffer::rect(content_x, y + height - 29, 6, 6, accent);
        framebuffer::text(
            content_x + 16,
            y + height - 30,
            desktop.settings_notice,
            color::MUTED,
            1,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn settings_row(
    desktop: &DesktopState,
    x: i32,
    window_y: i32,
    width: i32,
    index: usize,
    label: &str,
    detail: &str,
    value: &str,
    control: SettingControl,
) {
    let compact = width < 600;
    let row_top = if compact { 88 } else { 112 };
    let row_step = if compact { 47 } else { 62 };
    let row_height = if compact { 42 } else { 54 };
    let y = window_y + row_top + index as i32 * row_step;
    let selected = desktop.settings_row == index;
    let accent = desktop.preferences.accent.color();
    settings_rect(
        desktop.preferences,
        x,
        y,
        width,
        row_height,
        6,
        if selected {
            desktop.preferences.panel_color()
        } else {
            desktop.preferences.card_color()
        },
    );
    if selected {
        framebuffer::outline(x, y, width, row_height, accent);
    }
    framebuffer::text(
        x + 16,
        y + if compact { 17 } else { 12 },
        label,
        color::INK,
        1,
    );
    if !compact {
        framebuffer::text(x + 16, y + 33, detail, color::MUTED, 1);
    }

    match control {
        SettingControl::Toggle { on, available } => {
            let control_x = x + width - 62;
            settings_rect(
                desktop.preferences,
                control_x,
                y + if compact { 11 } else { 17 },
                42,
                20,
                10,
                if !available {
                    color::BORDER
                } else if on {
                    accent
                } else {
                    0x0036_3D42
                },
            );
            let knob_x = if on { control_x + 23 } else { control_x + 3 };
            settings_rect(
                desktop.preferences,
                knob_x,
                y + if compact { 14 } else { 20 },
                14,
                14,
                7,
                if available {
                    color::WHITE
                } else {
                    color::MUTED
                },
            );
            framebuffer::text(
                control_x - 88,
                y + if compact { 17 } else { 23 },
                value,
                if available { color::INK } else { color::MUTED },
                1,
            );
        }
        SettingControl::Choice => {
            let control_width = 154;
            let control_x = x + width - control_width - 20;
            settings_rect(
                desktop.preferences,
                control_x,
                y + if compact { 6 } else { 12 },
                control_width,
                30,
                5,
                0x0018_1D21,
            );
            let control_y = y + if compact { 6 } else { 12 };
            framebuffer::outline(control_x, control_y, control_width, 30, color::BORDER);
            framebuffer::text(control_x + 10, control_y + 10, "<", color::MUTED, 1);
            framebuffer::text(control_x + 30, control_y + 10, value, color::INK, 1);
            framebuffer::text(
                control_x + control_width - 18,
                control_y + 10,
                ">",
                color::MUTED,
                1,
            );
        }
        SettingControl::Status { ready } => {
            let value_x = x + width - 20 - value.len() as i32 * framebuffer::text_advance(1);
            let marker_x = value_x - 16;
            framebuffer::rect(
                marker_x,
                y + if compact { 18 } else { 24 },
                7,
                7,
                if ready { accent } else { color::MUTED },
            );
            framebuffer::text(
                value_x,
                y + if compact { 17 } else { 23 },
                value,
                color::MUTED,
                1,
            );
        }
        SettingControl::Action { available } => {
            let button_x = x + width - 104;
            settings_rect(
                desktop.preferences,
                button_x,
                y + if compact { 6 } else { 12 },
                84,
                30,
                5,
                if available { accent } else { color::BORDER },
            );
            framebuffer::text(
                button_x + 20,
                y + if compact { 16 } else { 22 },
                value,
                color::WHITE,
                1,
            );
        }
        SettingControl::Plain => {
            let value_x = x + width - 20 - value.len() as i32 * framebuffer::text_advance(1);
            framebuffer::text(
                value_x,
                y + if compact { 17 } else { 23 },
                value,
                color::MUTED,
                1,
            );
        }
    }
}

fn settings_rect(
    preferences: DesktopPreferences,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    radius: i32,
    value: u32,
) {
    if preferences.rounded_controls {
        framebuffer::rounded_rect(x, y, width, height, radius, value);
    } else {
        framebuffer::rect(x, y, width, height, value);
    }
}

fn draw_system(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let metric_width = (rect.width as i32 - 68) / 2;
    let right_x = x + 40 + metric_width;
    framebuffer::text(x + 28, y + 58, "System", color::INK, 2);
    metric(
        x + 28,
        y + 98,
        metric_width,
        "Display",
        "Protocol v1",
        color::GREEN,
    );
    metric(
        right_x,
        y + 98,
        metric_width,
        "Framebuffer",
        framebuffer::current_mode().label(),
        color::CYAN,
    );
    metric(
        x + 28,
        y + 174,
        metric_width,
        "Surfaces",
        "11 Form-owned",
        color::PURPLE,
    );
    metric(
        right_x,
        y + 174,
        metric_width,
        "Presentation",
        refresh_rate_label(desktop.preferences.refresh_rate),
        color::GREEN,
    );
    metric(
        x + 28,
        y + 250,
        metric_width,
        "Input",
        "PS/2 mouse + keys",
        color::CYAN,
    );
    metric(
        right_x,
        y + 250,
        metric_width,
        "Network",
        "Native IPv4 stack",
        color::GREEN,
    );
    framebuffer::text(x + 34, y + 343, "Commits", color::MUTED, 1);
    draw_number(
        x + 180,
        y + 343,
        desktop.server.commit_sequence(),
        color::GREEN,
    );
    framebuffer::text(x + 352, y + 343, "Active", color::MUTED, 1);
    framebuffer::text(x + 430, y + 343, desktop.active.label(), color::CYAN, 1);
    let stats = framebuffer::presentation_stats();
    framebuffer::text(x + 540, y + 343, "Frames", color::MUTED, 1);
    draw_number(x + 602, y + 343, stats.frames, color::GREEN);
}

fn metric(x: i32, y: i32, width: i32, label: &str, value: &str, accent: u32) {
    framebuffer::rect(x, y, width, 60, 0x0015_1922);
    framebuffer::rect(x, y, 5, 60, accent);
    framebuffer::text(x + 18, y + 13, label, color::MUTED, 1);
    framebuffer::text(x + 18, y + 34, value, accent, 1);
}

fn draw_dock(desktop: &DesktopState) {
    let y = taskbar_y() as i32;
    let accent = desktop.preferences.accent.color();
    framebuffer::alpha_rect(
        0,
        y,
        framebuffer::width() as i32,
        TASKBAR_HEIGHT as i32,
        desktop.preferences.panel_color(),
        244,
    );
    framebuffer::rect(
        0,
        y,
        framebuffer::width() as i32,
        1,
        desktop.preferences.border_color(false),
    );
    let start_color = if desktop.launcher_open {
        0x0032_2654
    } else {
        0x0017_1B25
    };
    settings_rect(
        desktop.preferences,
        START_X as i32,
        y + 7,
        32,
        34,
        7,
        start_color,
    );
    settings_rect(desktop.preferences, 16, y + 15, 16, 16, 4, accent);
    framebuffer::text(20, y + 19, "E", color::WHITE, 1);
    let mut running_index = 0_i32;
    for app in AppKind::ALL {
        if !desktop.app_open[app.index()] {
            continue;
        }
        let x = TASK_ICON_X as i32 + running_index * TASK_ICON_STEP as i32;
        if app == desktop.active && desktop.app_is_visible(app) {
            settings_rect(desktop.preferences, x, y + 7, 32, 34, 7, 0x0024_292F);
        }
        settings_rect(desktop.preferences, x + 8, y + 15, 18, 18, 4, app.accent());
        framebuffer::text(x + 13, y + 19, app.shortcut(), color::WHITE, 1);
        framebuffer::rect(
            x + 8,
            y + 40,
            18,
            2,
            if desktop.app_minimized[app.index()] {
                color::MUTED
            } else {
                accent
            },
        );
        running_index += 1;
    }
    if desktop.preferences.status_visible {
        let connectivity = radio::snapshot();
        framebuffer::rect(
            framebuffer::width() as i32 - 22,
            y + 21,
            7,
            7,
            if connectivity.network_enabled && connectivity.ethernet.connected() {
                accent
            } else {
                color::MUTED
            },
        );
    }
}

fn draw_launcher(desktop: &DesktopState) {
    let x = LAUNCHER_X as i32;
    let y = launcher_y() as i32;
    let width = LAUNCHER_WIDTH as i32;
    let height = LAUNCHER_HEIGHT as i32;
    framebuffer::rounded_rect(x, y, width, height, 9, desktop.preferences.panel_color());
    framebuffer::outline(x, y, width, height, desktop.preferences.border_color(false));
    framebuffer::text(x + 16, y + 17, "Applications", color::INK, 1);
    framebuffer::rect(x + 12, y + 39, width - 24, 1, color::BORDER);
    for (index, app) in AppKind::ALL.iter().copied().enumerate() {
        let row_y = y + 48 + index as i32 * 32;
        framebuffer::rounded_rect(
            x + 8,
            row_y,
            width - 16,
            28,
            5,
            if app == desktop.active && desktop.app_open[app.index()] {
                0x0024_292F
            } else {
                desktop.preferences.panel_color()
            },
        );
        framebuffer::rounded_rect(x + 16, row_y + 6, 16, 16, 4, app.accent());
        framebuffer::text(x + 20, row_y + 10, app.shortcut(), color::WHITE, 1);
        framebuffer::text(x + 44, row_y + 10, app.label(), color::INK, 1);
        if desktop.app_open[app.index()] {
            framebuffer::rect(x + width - 24, row_y + 11, 5, 5, color::GREEN);
        }
    }
}

fn wrapped_text(mut x: i32, mut y: i32, width: i32, value: &str, color: u32, scale: i32) -> i32 {
    let left = x;
    let advance = framebuffer::text_advance(scale);
    for word in value.split_ascii_whitespace() {
        let word_width = word.len() as i32 * advance;
        if x != left && x + word_width > left + width {
            x = left;
            y += if scale <= 1 { 10 } else { 18 };
        }
        framebuffer::text(x, y, word, color, scale);
        x += word_width + advance;
    }
    y + if scale <= 1 { 8 } else { 16 }
}

fn encode_search_url(query: &str, output: &mut [u8]) -> Option<usize> {
    let prefix = SEARCH_PREFIX.as_bytes();
    if prefix.len() > output.len() {
        return None;
    }
    output[..prefix.len()].copy_from_slice(prefix);
    let mut length = prefix.len();
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in query.bytes() {
        let encoded = if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            [byte, 0, 0]
        } else if byte == b' ' {
            [b'+', 0, 0]
        } else {
            [b'%', HEX[(byte >> 4) as usize], HEX[(byte & 0x0F) as usize]]
        };
        let count = if encoded[0] == b'%' { 3 } else { 1 };
        if length + count > output.len() {
            return None;
        }
        output[length..length + count].copy_from_slice(&encoded[..count]);
        length += count;
    }
    Some(length)
}

const fn is_http_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn resolve_browser_link(base: &str, target: &str, output: &mut [u8]) -> Option<usize> {
    let target = target.trim();
    if target.is_empty() || !target.is_ascii() {
        return None;
    }
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("hexa://")
    {
        return copy_browser_url(output, target.as_bytes());
    }

    if target.starts_with("//") {
        let scheme = if base.starts_with("https://") {
            b"https:".as_slice()
        } else if base.starts_with("http://") {
            b"http:".as_slice()
        } else {
            return None;
        };
        return join_browser_url(output, scheme, target.as_bytes());
    }

    if target.starts_with('#') {
        let fragment_start = base.find('#').unwrap_or(base.len());
        return join_browser_url(
            output,
            &base.as_bytes()[..fragment_start],
            target.as_bytes(),
        );
    }

    let scheme_end = base.find("://")?.checked_add(3)?;
    let authority_end = base[scheme_end..]
        .find(['/', '?', '#'])
        .map(|offset| scheme_end + offset)
        .unwrap_or(base.len());
    if target.starts_with('/') {
        return join_browser_url(output, &base.as_bytes()[..authority_end], target.as_bytes());
    }

    let fragment_start = base.find('#').unwrap_or(base.len());
    let clean_end = base[..fragment_start].find('?').unwrap_or(fragment_start);
    if target.starts_with('?') {
        if authority_end == clean_end {
            let length = authority_end.checked_add(1)?.checked_add(target.len())?;
            if length > output.len() {
                return None;
            }
            output[..authority_end].copy_from_slice(&base.as_bytes()[..authority_end]);
            output[authority_end] = b'/';
            output[authority_end + 1..length].copy_from_slice(target.as_bytes());
            return Some(length);
        }
        return join_browser_url(output, &base.as_bytes()[..clean_end], target.as_bytes());
    }
    let path_start = authority_end.min(clean_end);
    if path_start == clean_end {
        let length = base[..authority_end]
            .len()
            .checked_add(1)?
            .checked_add(target.len())?;
        if length > output.len() {
            return None;
        }
        output[..authority_end].copy_from_slice(&base.as_bytes()[..authority_end]);
        output[authority_end] = b'/';
        output[authority_end + 1..length].copy_from_slice(target.as_bytes());
        return Some(length);
    }
    let directory_end = base[path_start..clean_end]
        .rfind('/')
        .map(|offset| path_start + offset + 1)
        .unwrap_or_else(|| authority_end.saturating_add(1).min(clean_end));
    join_browser_url(output, &base.as_bytes()[..directory_end], target.as_bytes())
}

fn copy_browser_url(output: &mut [u8], value: &[u8]) -> Option<usize> {
    if value.len() > output.len() {
        return None;
    }
    output[..value.len()].copy_from_slice(value);
    Some(value.len())
}

fn join_browser_url(output: &mut [u8], left: &[u8], right: &[u8]) -> Option<usize> {
    let length = left.len().checked_add(right.len())?;
    if length > output.len() {
        return None;
    }
    output[..left.len()].copy_from_slice(left);
    output[left.len()..length].copy_from_slice(right);
    Some(length)
}

fn draw_number(x: i32, y: i32, mut value: u64, color: u32) {
    let mut bytes = [b'0'; 20];
    let mut start = bytes.len() - 1;
    while value >= 10 {
        bytes[start] = b'0' + (value % 10) as u8;
        value /= 10;
        start -= 1;
    }
    bytes[start] = b'0' + value as u8;
    let text = core::str::from_utf8(&bytes[start..]).unwrap_or("?");
    framebuffer::text(x, y, text, color, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_url_uses_duckduckgo_html_and_percent_encoding() {
        let mut output = [0_u8; 512];
        let length = encode_search_url("rust os + tls", &mut output).unwrap();
        assert_eq!(
            core::str::from_utf8(&output[..length]).unwrap(),
            "https://duckduckgo.com/html/?q=rust+os+%2B+tls"
        );
    }

    #[test]
    fn browser_links_resolve_absolute_root_query_and_relative_targets() {
        let mut output = [0_u8; 512];
        let cases = [
            (
                "https://html.duckduckgo.com/html/?q=kernel",
                "//duckduckgo.com/l/?uddg=example",
                "https://duckduckgo.com/l/?uddg=example",
            ),
            (
                "https://html.duckduckgo.com/html/?q=kernel",
                "/html/?q=display",
                "https://html.duckduckgo.com/html/?q=display",
            ),
            (
                "https://example.com/docs/page.html",
                "?compact=1",
                "https://example.com/docs/page.html?compact=1",
            ),
            (
                "https://example.com/docs/page.html",
                "next.html",
                "https://example.com/docs/next.html",
            ),
            (
                "https://example.com",
                "index.html",
                "https://example.com/index.html",
            ),
            (
                "https://example.com?q=old",
                "?q=new",
                "https://example.com/?q=new",
            ),
            (
                "https://example.com/page?q=one#old",
                "#section",
                "https://example.com/page?q=one#section",
            ),
        ];
        for (base, target, expected) in cases {
            output.fill(0);
            let length = resolve_browser_link(base, target, &mut output).unwrap();
            assert_eq!(core::str::from_utf8(&output[..length]).unwrap(), expected);
        }
    }

    #[test]
    fn only_navigation_redirect_statuses_are_followed() {
        for status in [301, 302, 303, 307, 308] {
            assert!(is_http_redirect(status));
        }
        for status in [200, 300, 304, 305, 306, 400] {
            assert!(!is_http_redirect(status));
        }
    }
}
