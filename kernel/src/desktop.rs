use crate::{
    audio,
    display_timing::{FrameDecision, FramePacer, RefreshRate, TimingConfig, VSyncPolicy},
    framebuffer,
    input::{
        Input, InputEvent, PointerEvent, KEY_DESKTOP_SHELL_CONFIRM, KEY_DOWN, KEY_LEFT, KEY_RIGHT,
        KEY_SUPER_ALT_DOWN, KEY_SUPER_ALT_LEFT, KEY_SUPER_ALT_RIGHT, KEY_SUPER_ALT_UP,
        KEY_SUPER_CLOSE, KEY_SUPER_CYCLE, KEY_SUPER_DOWN, KEY_SUPER_FULLSCREEN, KEY_SUPER_LAUNCHER,
        KEY_SUPER_LEFT, KEY_SUPER_RIGHT, KEY_SUPER_UP, KEY_UP,
    },
    locale::{Locale, Text as LocalText},
    network, radio, slog, state,
};
use alloc::boxed::Box;
use expos_core::{
    AbiCall, AbiRequest, AbiResponse, AbiStatus, Authority, BrowserError, BrowserText,
    BrowserWebState, BufferFormat, BufferHandle, CapabilityBroker, CfcFin, DisplayServer, Document,
    ExternalResourceKind, Fin, FormKind, NativeCallGate, NodeKind, Operations, Rect,
    ResourceManifest, SurfaceRole, TextAlign, WebApiRequest, FORM_ABI_VERSION,
};
use framebuffer::color;

pub const DISPLAY_FIN: Fin = Fin::from_u128(0x4449_5350_4C41_5900_0000_0000_0000_0001);
pub const BROWSER_FIN: Fin = Fin::from_u128(0x4252_4F57_5345_5200_0000_0000_0000_0001);
pub const TERMINAL_FIN: Fin = Fin::from_u128(0x5445_524D_494E_414C_0000_0000_0000_0001);
pub const FORMS_FIN: Fin = Fin::from_u128(0x464F_524D_5300_0000_0000_0000_0000_0001);
pub const PACKAGES_FIN: Fin = Fin::from_u128(0x5041_434B_4147_4553_0000_0000_0000_0001);
pub const SETTINGS_FIN: Fin = Fin::from_u128(0x5345_5454_494E_4753_0000_0000_0000_0001);
pub const SYSTEM_FIN: Fin = Fin::from_u128(0x5359_5354_454D_0000_0000_0000_0000_0001);
pub const GAMES_FIN: Fin = Fin::from_u128(0x4741_4D45_5300_0000_0000_0000_0000_0001);
pub const NOTES_FIN: Fin = Fin::from_u128(0x4E4F_5445_5300_0000_0000_0000_0000_0001);
pub const APPS_FIN: Fin = Fin::from_u128(0x4150_5053_0000_0000_0000_0000_0000_0001);
pub const AYO_FIN: Fin = Fin::from_u128(0x4159_4F00_0000_0000_0000_0000_0000_0001);

const APP_COUNT: usize = 9;
const DESKTOP_UI_STATE: &str = "DesktopUI.state";
const BROWSER_DATA_STATE: &str = "BrowserData.state";
const DESKTOP_UI_MAGIC_V1: [u8; 4] = *b"DUI1";
const DESKTOP_UI_MAGIC_V2: [u8; 4] = *b"DUI2";
const DESKTOP_UI_MAGIC_V3: [u8; 4] = *b"DUI3";
const TERMINAL_HISTORY: usize = 24;
const TERMINAL_CAPACITY: usize = 96;
const TERMINAL_SCROLLBACK: usize = 40;
const TERMINAL_OUTPUT_CAPACITY: usize = 112;
const NOTES_CAPACITY: usize = crate::expfs_store::FORM_CONTENT_CAPACITY;
const BROWSER_HISTORY_CAPACITY: usize = 12;
const BROWSER_TAB_CAPACITY: usize = 6;
const BROWSER_BOOKMARK_CAPACITY: usize = 8;
const BROWSER_TITLE_CAPACITY: usize = 64;
const BROWSER_FIND_CAPACITY: usize = 64;
const CURSOR_WIDTH: usize = 14;
const CURSOR_HEIGHT: usize = 20;
const LAUNCHER_WIDTH: u16 = 250;
const LAUNCHER_GRID_WIDTH: u16 = 430;
const LAUNCHER_HEIGHT: u16 = 410;
const STABLE_FIN: Fin = Fin::from_u128(0x4449_4D00_0000_0000_0000_0000_0000_0001);
const WEATHER_ALLOWED_HOSTS: [&str; 2] = ["geocoding-api.open-meteo.com", "api.open-meteo.com"];
const WEATHER_REQUEST_COOLDOWN_SECONDS: u64 = 3;

#[derive(Clone, Copy)]
struct BrowserTab {
    history: [[u8; 512]; BROWSER_HISTORY_CAPACITY],
    history_len: [u16; BROWSER_HISTORY_CAPACITY],
    history_count: u8,
    history_cursor: u8,
    title: [u8; BROWSER_TITLE_CAPACITY],
    title_len: u8,
    scroll: i32,
}

impl BrowserTab {
    const EMPTY: Self = Self {
        history: [[0; 512]; BROWSER_HISTORY_CAPACITY],
        history_len: [0; BROWSER_HISTORY_CAPACITY],
        history_count: 0,
        history_cursor: 0,
        title: [0; BROWSER_TITLE_CAPACITY],
        title_len: 0,
        scroll: 0,
    };

    fn home() -> Self {
        let mut tab = Self::EMPTY;
        tab.history[0][..12].copy_from_slice(b"expos://home");
        tab.history_len[0] = 12;
        tab.history_count = 1;
        tab.set_title("New tab");
        tab
    }

    fn url(&self) -> &str {
        let cursor =
            (self.history_cursor as usize).min(self.history_count.saturating_sub(1) as usize);
        let length = self.history_len[cursor] as usize;
        core::str::from_utf8(&self.history[cursor][..length]).unwrap_or("expos://home")
    }

    fn title(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len as usize]).unwrap_or("New tab")
    }

    fn set_title(&mut self, value: &str) {
        let length = value.len().min(self.title.len());
        self.title.fill(0);
        self.title[..length].copy_from_slice(&value.as_bytes()[..length]);
        self.title_len = length as u8;
    }

    fn reset(&mut self) {
        *self = Self::home();
    }
}

#[derive(Clone, Copy)]
struct BrowserBookmark {
    url: [u8; 512],
    url_len: u16,
    title: [u8; BROWSER_TITLE_CAPACITY],
    title_len: u8,
}

impl BrowserBookmark {
    const EMPTY: Self = Self {
        url: [0; 512],
        url_len: 0,
        title: [0; BROWSER_TITLE_CAPACITY],
        title_len: 0,
    };

    fn new(url: &str, title: &str) -> Self {
        let mut bookmark = Self::EMPTY;
        let url_len = url.len().min(bookmark.url.len());
        bookmark.url[..url_len].copy_from_slice(&url.as_bytes()[..url_len]);
        bookmark.url_len = url_len as u16;
        let title_len = title.len().min(bookmark.title.len());
        bookmark.title[..title_len].copy_from_slice(&title.as_bytes()[..title_len]);
        bookmark.title_len = title_len as u8;
        bookmark
    }

    fn url(&self) -> &str {
        core::str::from_utf8(&self.url[..self.url_len as usize]).unwrap_or("expos://home")
    }

    fn title(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len as usize]).unwrap_or("Bookmark")
    }
}

/// Heap-backed, bounded page container for the native Browser.
///
/// The old design embedded the complete DOM/style/script arena directly in
/// `DesktopState`. Loading a page therefore kept the live arena and a staged
/// replacement on the already busy kernel stack. The container moves the live
/// arena out of that stack and swaps it only after a complete parse succeeds;
/// malformed and over-budget documents leave the last valid page intact.
struct BrowserContainer {
    document: Box<Document>,
    generation: u64,
    rejected_loads: u64,
    last_error: Option<BrowserError>,
}

#[derive(Clone, Copy, Debug, Default)]
struct BrowserResourceStats {
    discovered: u16,
    loaded: u16,
    rejected: u16,
    stylesheets: u16,
    scripts: u16,
    images: u16,
    audio: u16,
    fetches: u16,
}

struct BrowserMediaCache {
    image_url: BrowserText,
    image_width: u8,
    image_height: u8,
    image_pixels: [u32; 64 * 64],
    audio_url: BrowserText,
    audio_len: usize,
    audio_bytes: [u8; network::HTTP_BODY_CAPACITY],
}

impl BrowserMediaCache {
    const fn new() -> Self {
        Self {
            image_url: BrowserText::empty(),
            image_width: 0,
            image_height: 0,
            image_pixels: [0; 64 * 64],
            audio_url: BrowserText::empty(),
            audio_len: 0,
            audio_bytes: [0; network::HTTP_BODY_CAPACITY],
        }
    }

    fn clear(&mut self) {
        self.image_url = BrowserText::empty();
        self.image_width = 0;
        self.image_height = 0;
        self.audio_url = BrowserText::empty();
        self.audio_len = 0;
    }
}

impl BrowserContainer {
    fn new(document: Document) -> Self {
        Self {
            document: Box::new(document),
            generation: 1,
            rejected_loads: 0,
            last_error: None,
        }
    }

    fn replace(&mut self, document: Document) {
        *self.document = document;
        self.generation = self.generation.saturating_add(1);
    }

    fn reject(&mut self, error: BrowserError) {
        self.rejected_loads = self.rejected_loads.saturating_add(1);
        self.last_error = Some(error);
    }
}

impl core::ops::Deref for BrowserContainer {
    type Target = Document;

    fn deref(&self) -> &Self::Target {
        &self.document
    }
}

impl core::ops::DerefMut for BrowserContainer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.document
    }
}

fn taskbar_thickness(preferences: DesktopPreferences) -> i16 {
    state::TASKBAR_SIZES_PX[preferences.taskbar_size.min(8) as usize] as i16
}

fn taskbar_widget_width(preferences: DesktopPreferences) -> i32 {
    if preferences.taskbar_placement.vertical() {
        return 0;
    }
    let flags = preferences.taskbar_widgets;
    i32::from(flags & TASKBAR_WIDGET_DATE != 0) * 66
        + i32::from(flags & TASKBAR_WIDGET_ACTIVE_APP != 0) * 86
        + i32::from(flags & TASKBAR_WIDGET_WEATHER != 0) * 68
        + i32::from(flags & TASKBAR_WIDGET_PERFORMANCE != 0) * 58
        + i32::from(flags & TASKBAR_WIDGET_AUDIO != 0) * 54
        + i32::from(flags & TASKBAR_WIDGET_TIMEZONE != 0) * 62
}

fn taskbar_rect(preferences: DesktopPreferences) -> Rect {
    let width = framebuffer::width() as u16;
    let height = framebuffer::height() as u16;
    let thickness = taskbar_thickness(preferences) as u16;
    match preferences.taskbar_placement {
        TaskbarPlacement::Bottom => {
            Rect::new(0, height.saturating_sub(thickness) as i16, width, thickness)
        }
        TaskbarPlacement::Top => Rect::new(0, 0, width, thickness),
        TaskbarPlacement::Left => Rect::new(0, 0, thickness, height),
        TaskbarPlacement::Right => {
            Rect::new(width.saturating_sub(thickness) as i16, 0, thickness, height)
        }
    }
}

fn work_area(preferences: DesktopPreferences) -> Rect {
    let width = framebuffer::width() as u16;
    let height = framebuffer::height() as u16;
    if !preferences.taskbar_visible || preferences.taskbar_autohide {
        return Rect::new(0, 0, width, height);
    }
    let thickness = taskbar_thickness(preferences) as u16;
    match preferences.taskbar_placement {
        TaskbarPlacement::Bottom => Rect::new(0, 0, width, height.saturating_sub(thickness)),
        TaskbarPlacement::Top => {
            Rect::new(0, thickness as i16, width, height.saturating_sub(thickness))
        }
        TaskbarPlacement::Left => {
            Rect::new(thickness as i16, 0, width.saturating_sub(thickness), height)
        }
        TaskbarPlacement::Right => Rect::new(0, 0, width.saturating_sub(thickness), height),
    }
}

fn launcher_rect(preferences: DesktopPreferences) -> Rect {
    let dock = taskbar_rect(preferences);
    let screen_width = framebuffer::width() as i16;
    let screen_height = framebuffer::height() as i16;
    let requested_width = if preferences.menu_grid {
        LAUNCHER_GRID_WIDTH
    } else {
        LAUNCHER_WIDTH
    };
    let width = requested_width.min(framebuffer::width() as u16 - 8);
    let height = LAUNCHER_HEIGHT.min(framebuffer::height() as u16 - 8);
    let (x, y) = match preferences.taskbar_placement {
        TaskbarPlacement::Bottom => (8, (dock.y - height as i16 - 8).max(4)),
        TaskbarPlacement::Top => (
            8,
            (dock.height as i16 + 8).min(screen_height - height as i16 - 4),
        ),
        TaskbarPlacement::Left => (
            (dock.width as i16 + 8).min(screen_width - width as i16 - 4),
            8,
        ),
        TaskbarPlacement::Right => ((dock.x - width as i16 - 8).max(4), 8),
    };
    Rect::new(x, y, width, height)
}

fn contains(rect: Rect, x: i16, y: i16) -> bool {
    let right = rect.x as i32 + rect.width as i32;
    let bottom = rect.y as i32 + rect.height as i32;
    (rect.x as i32..right).contains(&(x as i32)) && (rect.y as i32..bottom).contains(&(y as i32))
}

fn taskbar_start_rect(preferences: DesktopPreferences) -> Rect {
    let dock = taskbar_rect(preferences);
    if preferences.taskbar_placement.vertical() {
        let size = (dock.width.saturating_sub(8)).clamp(20, 32);
        Rect::new(
            dock.x + (dock.width as i16 - size as i16) / 2,
            6,
            size,
            size,
        )
    } else {
        let size = (dock.height.saturating_sub(8)).clamp(20, 32);
        Rect::new(
            8,
            dock.y + (dock.height as i16 - size as i16) / 2,
            size,
            size,
        )
    }
}

fn taskbar_app_rect(preferences: DesktopPreferences, ordinal: usize, running_count: usize) -> Rect {
    let dock = taskbar_rect(preferences);
    let vertical = preferences.taskbar_placement.vertical();
    let desired_extent = if preferences.taskbar_labels && !vertical {
        100
    } else {
        32
    };
    let axis_length = if vertical {
        dock.height as i32
    } else {
        dock.width as i32
    };
    let trailing_reserve = (if !preferences.status_visible {
        12
    } else if vertical {
        if preferences.clock_seconds {
            84
        } else {
            66
        }
    } else if preferences.clock_seconds {
        80 + if preferences.clock_24h { 0 } else { 18 }
    } else {
        62 + if preferences.clock_24h { 0 } else { 18 }
    }) + taskbar_widget_width(preferences);
    let available = (axis_length - 46 - trailing_reserve).max(32);
    let item_extent =
        desired_extent.min((available / running_count.max(1) as i32 - 4).clamp(24, desired_extent));
    let step = item_extent + 4;
    let total = running_count as i32 * step;
    let start = match preferences.taskbar_alignment {
        TaskbarAlignment::Start => 46,
        TaskbarAlignment::Center => ((axis_length - total) / 2).max(46),
        TaskbarAlignment::End => (axis_length - total - trailing_reserve).max(46),
    };
    let offset = start + ordinal as i32 * step;
    if vertical {
        let size = (dock.width.saturating_sub(8)).clamp(20, 32);
        Rect::new(
            dock.x + (dock.width as i16 - size as i16) / 2,
            offset as i16,
            size,
            size,
        )
    } else {
        let size = (dock.height.saturating_sub(8)).clamp(20, 32);
        Rect::new(
            offset as i16,
            dock.y + (dock.height as i16 - size as i16) / 2,
            item_extent as u16,
            size,
        )
    }
}

fn app_dimensions(preferences: DesktopPreferences) -> (u16, u16) {
    let area = work_area(preferences);
    let screen_width = area.width;
    let work_height = area.height.max(80);
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
    (
        width.max(480).min(screen_width.saturating_sub(8)),
        height.max(360).min(work_height.saturating_sub(8)),
    )
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

fn settings_category_capacity(rect: Rect) -> usize {
    if settings_compact(rect) {
        7
    } else {
        11
    }
}

fn settings_category_view_start(rect: Rect, selected: usize) -> usize {
    let capacity = settings_category_capacity(rect).min(SETTINGS_CATEGORY_COUNT);
    selected
        .saturating_sub(capacity - 1)
        .min(SETTINGS_CATEGORY_COUNT - capacity)
}

fn settings_row_capacity(content_width: i32) -> usize {
    if content_width < 600 {
        5
    } else {
        7
    }
}

fn settings_row_view_start(
    category: SettingsCategory,
    selected: usize,
    content_width: i32,
) -> usize {
    let count = category.row_count();
    let capacity = settings_row_capacity(content_width).min(count.max(1));
    selected.saturating_sub(capacity - 1).min(count - capacity)
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

const fn bypass_software_pacing(responsive: bool, damaged_commit: bool) -> bool {
    responsive && damaged_commit
}

const HOME: &str = "<style>h1{color:#a8d9df;max-width:720px}.hero{background:#182427;border:1px solid #466267;border-radius:18px;padding:14px;max-width:720px;line-height:21px}.card{background:#151c20;border:1px solid #35433f;border-radius:12px;padding:11px;max-width:720px;line-height:20px}.chip{color:#bce8ed;background:#243a3e;border:1px solid #426167;border-radius:16px;padding:6px}button{color:#071f23;background:#9cdce4;border-radius:14px;padding:8px;max-width:260px}</style><title>New tab</title><h1>ExpOS Browser</h1><p id='status' class='hero'>A calm, bounded web workspace with verified TLS, six tab sessions, history, bookmarks, find, external page resources and persistent site data.</p><button id='demo'>Try a local interaction</button><p class='card'>Web bridge: bounded fetch, cookies, local/session storage, external CSS and script, BMP images, and PCM WAV audio. Every queue, body, origin and media buffer has a fixed ceiling.</p><p class='card'>Press N for a new tab, Tab to switch, X to close, F to find, B to bookmark, or / to focus the omnibox.</p><a class='chip' href='expos://about'>Capabilities</a><a class='chip' href='expos://packages'>Packages</a><a class='chip' href='expos://system'>System</a><script>document.title='New tab';document.getElementById('demo').onclick=function(){document.getElementById('status').textContent='The deterministic script engine handled this Material-style action.';document.getElementById('status').style.backgroundColor='#20363a';}</script>";
const ABOUT: &str = "<style>h1{color:#a8d9df}.card{background:#151c20;border:1px solid #35433f;border-radius:12px;padding:11px;max-width:720px;line-height:20px}.accent{background:#20363a;border:1px solid #568087;border-radius:16px;padding:12px;max-width:720px}</style><title>About Browser</title><h1>Browser capabilities</h1><p class='accent'>Form-native navigation, verified HTTP/TLS transport, external CSS and deterministic scripts, bounded fetch, origin-partitioned cookies and storage, BMP images, and PCM WAV playback through ExpAudio.</p><p class='card'>This is a contained browser engine, not Blink/V8. It does not promise arbitrary ECMAScript, a standards-complete Web API surface, compressed media or video decoding, extensions, or GPU raster acceleration.</p><a href='expos://home'>New tab</a>";
const BROWSER_PACKAGES: &str = "<title>Packages</title><h1>Packages</h1><li>Core tools</li><li>Display</li><li>Notes</li><li>Games</li><a href='expos://home'>Home</a>";
const BROWSER_SYSTEM: &str = "<title>System</title><h1>System</h1><li>480p / 720p / 1080p display</li><li>60 / 75 / 120 / 144 Hz compositor pacing</li><li>Keyboard and mouse</li><li>RTL8139 network</li><a href='expos://home'>Home</a>";
const NETWORK_BLOCKED: &str = "<title>Offline</title><h1>Offline</h1><p>The address could not be loaded.</p><a href='expos://home'>Home</a>";
const NETWORK_ERROR: &str = "<title>Load failed</title><h1>Could not load page</h1><p>Check the address, connection, and certificate.</p><a href='expos://home'>Home</a>";
const SEARCH_ERROR: &str = "<title>Search failed</title><h1>Search query is too long</h1><p>Use a shorter query in the address bar.</p><a href='expos://home'>Home</a>";
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
    Apps,
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
        Self::Apps,
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
            Self::Apps => 8,
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Browser => "BROWSER",
            Self::Terminal => "TERMINAL",
            Self::Forms => "FORMS",
            Self::Packages => "AYO PACKAGE MANAGER",
            Self::Settings => "SETTINGS",
            Self::System => "SYSTEM SCOPE",
            Self::Games => "PRISM ARCADE",
            Self::Notes => "NOTES",
            Self::Apps => "AYO APPS",
        }
    }

    const fn localized_label(self, locale: Locale) -> &'static str {
        locale.app(self.index())
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
            Self::Apps => APPS_FIN,
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
            Self::Apps => "A",
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
            Self::Apps => color::CYAN,
        }
    }
}

#[derive(Clone, Copy)]
enum LauncherItem {
    BuiltIn(AppKind),
    Installed(usize),
}

fn launcher_item(desktop: &DesktopState, ordinal: usize) -> Option<LauncherItem> {
    let mut cursor = 0;
    if desktop.preferences.menu_show_builtins {
        for app in AppKind::ALL {
            // Ayo-installed apps have direct entries. The shared Apps surface
            // is an internal host, not a second app-inside-an-app launcher.
            if app == AppKind::Apps {
                continue;
            }
            if !desktop.app_enabled(app) {
                continue;
            }
            if cursor == ordinal {
                return Some(LauncherItem::BuiltIn(app));
            }
            cursor += 1;
        }
    }
    if desktop.preferences.menu_show_installed {
        let installed_ordinal = ordinal.checked_sub(cursor)?;
        return desktop
            .native_apps
            .installed_at(installed_ordinal)
            .map(LauncherItem::Installed);
    }
    None
}

fn launcher_item_count(desktop: &DesktopState) -> usize {
    let builtins = if desktop.preferences.menu_show_builtins {
        AppKind::ALL
            .iter()
            .copied()
            .filter(|app| *app != AppKind::Apps && desktop.app_enabled(*app))
            .count()
    } else {
        0
    };
    builtins
        + if desktop.preferences.menu_show_installed {
            desktop.native_apps.installed_count()
        } else {
            0
        }
}

fn launcher_columns(preferences: DesktopPreferences) -> usize {
    match preferences.menu_layout {
        0 => 1,
        2 => 3,
        3 => 2,
        _ => 2,
    }
}

fn launcher_row_height(preferences: DesktopPreferences) -> i32 {
    let base = state::MENU_ROW_HEIGHTS_PX[preferences.menu_density.min(4) as usize] as i32;
    match preferences.menu_layout {
        2 => (base - 4).max(18),
        3 => base + 12,
        _ => base,
    }
}

fn launcher_visible_rows(preferences: DesktopPreferences) -> usize {
    let height = launcher_rect(preferences).height as i32;
    ((height - 62) / launcher_row_height(preferences)).max(1) as usize
}

fn launcher_capacity(preferences: DesktopPreferences) -> usize {
    launcher_visible_rows(preferences) * launcher_columns(preferences)
}

const SETTINGS_CATEGORY_COUNT: usize = 18;
const CUSTOMIZATION_VALUE_COUNT: usize = state::THEME_PALETTE_CHOICES
    + state::WALLPAPER_VARIANT_CHOICES
    + state::ACCENT_COLOR_CHOICES
    + state::BACKDROP_TONE_CHOICES
    + 2 // pure-black apps
    + 2 // rounded controls
    + state::FONT_FACE_NAMES.len()
    + state::FONT_WEIGHT_NAMES.len()
    + state::WINDOW_CORNER_RADII_PX.len()
    + state::WINDOW_BORDER_WIDTHS_PX.len()
    + state::TITLEBAR_HEIGHTS_PX.len()
    + state::WINDOW_OPACITY_ALPHA.len()
    + state::WINDOW_OFFSCREEN_ALLOWANCES_PX.len()
    + 2 // edge snapping
    + state::WINDOW_SNAP_DISTANCES_PX.len()
    + state::FOCUS_POLICY_NAMES.len()
    + state::TASKBAR_PLACEMENT_NAMES.len()
    + state::TASKBAR_SIZES_PX.len()
    + state::TASKBAR_ALIGNMENT_NAMES.len()
    + 2 // auto-hide
    + 2 // translucent panel
    + 2 // application labels
    + 2 // clock seconds
    + state::MENU_ROW_HEIGHTS_PX.len()
    + state::ANIMATION_DURATIONS_MS.len()
    + state::UI_SCALE_PERCENT.len()
    + 2 // list / grid
    + 2 // built-in apps
    + 2 // installed apps
    + 2 // category labels
    + 2 // tooltips
    + 2 // install feedback
    + TIMEZONE_LABELS.len()
    + 2 // 12 / 24 hour clock
    + DATE_FORMAT_LABELS.len()
    + 2 // week start
    + 12 // six taskbar widgets, each on/off
    + 101 // audio volume
    + 2 // audio mute
    + 2 // reduce transparency
    + 2 // focus ring
    + WINDOW_TILING_LABELS.len()
    + CustomizationProfile::ALL.len(); // coordinated whole-desktop profiles

const TASKBAR_WIDGET_DATE: u8 = 1 << 0;
const TASKBAR_WIDGET_ACTIVE_APP: u8 = 1 << 1;
const TASKBAR_WIDGET_WEATHER: u8 = 1 << 2;
const TASKBAR_WIDGET_PERFORMANCE: u8 = 1 << 3;
const TASKBAR_WIDGET_AUDIO: u8 = 1 << 4;
const TASKBAR_WIDGET_TIMEZONE: u8 = 1 << 5;
const TIMEZONE_LABELS: [&str; 16] = [
    "UTC-12",
    "Pacific UTC-8",
    "Eastern UTC-5",
    "UTC",
    "Central Europe UTC+1",
    "Eastern Europe UTC+2",
    "Moscow UTC+3",
    "Gulf UTC+4",
    "India UTC+5:30",
    "Bishkek UTC+6",
    "Bangkok UTC+7",
    "China UTC+8",
    "Japan UTC+9",
    "Sydney UTC+10",
    "Auckland UTC+12",
    "Line Islands UTC+14",
];
const TIMEZONE_OFFSETS_MINUTES: [i16; 16] = [
    -720, -480, -300, 0, 60, 120, 180, 240, 330, 360, 420, 480, 540, 600, 720, 840,
];
const DATE_FORMAT_LABELS: [&str; 3] = ["YYYY-MM-DD", "DD/MM/YYYY", "MM/DD/YYYY"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsCategory {
    System,
    Profiles,
    Appearance,
    Accessibility,
    Network,
    Bluetooth,
    Display,
    Audio,
    Performance,
    Input,
    Windows,
    Taskbar,
    Menu,
    Privacy,
    About,
    Terminal,
    Language,
    Time,
}

impl SettingsCategory {
    const ALL: [Self; SETTINGS_CATEGORY_COUNT] = [
        Self::System,
        Self::Appearance,
        Self::Network,
        Self::Bluetooth,
        Self::Display,
        Self::Audio,
        Self::Performance,
        Self::Input,
        Self::Windows,
        Self::Taskbar,
        Self::Menu,
        Self::Profiles,
        Self::Accessibility,
        Self::Privacy,
        Self::About,
        Self::Terminal,
        Self::Language,
        Self::Time,
    ];

    const fn index(self) -> usize {
        match self {
            Self::System => 0,
            Self::Appearance => 1,
            Self::Network => 2,
            Self::Bluetooth => 3,
            Self::Display => 4,
            Self::Audio => 5,
            Self::Performance => 6,
            Self::Input => 7,
            Self::Windows => 8,
            Self::Taskbar => 9,
            Self::Menu => 10,
            Self::Profiles => 11,
            Self::Accessibility => 12,
            Self::Privacy => 13,
            Self::About => 14,
            Self::Terminal => 15,
            Self::Language => 16,
            Self::Time => 17,
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::System => "Desktop behavior",
            Self::Profiles => "Coordinated whole-desktop styles",
            Self::Appearance => "Colors and window style",
            Self::Accessibility => "Readability, pointer, and motion",
            Self::Network => "Connections and network access",
            Self::Bluetooth => "Nearby wireless devices",
            Self::Display => "ExpDisplay output",
            Self::Audio => "Output, volume, and media diagnostics",
            Self::Performance => "Rendering cost and responsiveness",
            Self::Input => "Pointer and keyboard",
            Self::Windows => "Placement, decoration, and focus",
            Self::Taskbar => "Panel placement and behavior",
            Self::Menu => "Launcher layout, contents, and motion",
            Self::Privacy => "Local data and access",
            Self::About => "ExpOS system information",
            Self::Terminal => "Font, size, color, and spacing",
            Self::Language => "System and Genesis language",
            Self::Time => "Clock, time zone, region, and panel widgets",
        }
    }

    const fn row_count(self) -> usize {
        match self {
            Self::System => 2,
            Self::Profiles => CustomizationProfile::ALL.len(),
            Self::Appearance => 8,
            Self::Accessibility => 11,
            Self::Network => 4,
            Self::Bluetooth => 2,
            Self::Display => 6,
            Self::Audio => 5,
            Self::Performance => 4,
            Self::Input => 5,
            Self::Windows => 9,
            Self::Taskbar => 13,
            Self::Menu => 9,
            Self::Privacy => 5,
            Self::About => 4,
            Self::Terminal => 5,
            Self::Language => 5,
            Self::Time => 7,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CustomizationProfile {
    Balanced,
    Compact,
    Focus,
    Accessible,
    Showcase,
    Touch,
    Night,
    Presentation,
}

impl CustomizationProfile {
    const ALL: [Self; 8] = [
        Self::Balanced,
        Self::Compact,
        Self::Focus,
        Self::Accessible,
        Self::Showcase,
        Self::Touch,
        Self::Night,
        Self::Presentation,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Balanced => "Balanced",
            Self::Compact => "Compact",
            Self::Focus => "Focus",
            Self::Accessible => "Accessible",
            Self::Showcase => "Showcase",
            Self::Touch => "Touch friendly",
            Self::Night => "Night",
            Self::Presentation => "Presentation",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Balanced => "Restore the calm, efficient desktop baseline",
            Self::Compact => "Fit more content with dense chrome and menus",
            Self::Focus => "Remove motion and decoration for distraction-free work",
            Self::Accessible => "Increase contrast, weight, targets, and pointer clarity",
            Self::Showcase => "Enable rich color, depth, translucency, and motion",
            Self::Touch => "Enlarge controls, launcher rows, titlebars, and the taskbar",
            Self::Night => "Warm low-motion surfaces with reduced transparency",
            Self::Presentation => "Large focused windows, strong contrast, and visible status",
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskbarPlacement {
    Bottom,
    Top,
    Left,
    Right,
}

impl TaskbarPlacement {
    const ALL: [Self; 4] = [Self::Bottom, Self::Top, Self::Left, Self::Right];

    const fn label(self) -> &'static str {
        match self {
            Self::Bottom => "Bottom",
            Self::Top => "Top",
            Self::Left => "Left",
            Self::Right => "Right",
        }
    }

    const fn from_persisted(value: u8) -> Self {
        match value {
            1 => Self::Top,
            2 => Self::Left,
            3 => Self::Right,
            _ => Self::Bottom,
        }
    }

    fn shifted(self, direction: i8) -> Self {
        Self::ALL[shift_index(self as u8, Self::ALL.len() as u8, direction) as usize]
    }

    const fn vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskbarAlignment {
    Start,
    Center,
    End,
}

impl TaskbarAlignment {
    const ALL: [Self; 3] = [Self::Start, Self::Center, Self::End];

    const fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Center => "Center",
            Self::End => "End",
        }
    }

    const fn from_persisted(value: u8) -> Self {
        match value {
            1 => Self::Center,
            2 => Self::End,
            _ => Self::Start,
        }
    }

    fn shifted(self, direction: i8) -> Self {
        Self::ALL[shift_index(self as u8, Self::ALL.len() as u8, direction) as usize]
    }
}

const fn shift_index(current: u8, count: u8, direction: i8) -> u8 {
    if direction < 0 {
        (current + count - 1) % count
    } else {
        (current + 1) % count
    }
}

const CORNER_RADIUS_LABELS: [&str; 9] = [
    "Square", "2 px", "4 px", "6 px", "8 px", "10 px", "12 px", "16 px", "20 px",
];
const BORDER_WIDTH_LABELS: [&str; 7] = ["Off", "1 px", "2 px", "3 px", "4 px", "6 px", "8 px"];
const TITLEBAR_LABELS: [&str; 5] = ["Compact", "Small", "Normal", "Large", "Tall"];
const WINDOW_OPACITY_LABELS: [&str; 6] = ["Opaque", "96%", "91%", "85%", "75%", "63%"];
const OFFSCREEN_LABELS: [&str; 9] = [
    "Contained",
    "8 px",
    "16 px",
    "32 px",
    "64 px",
    "96 px",
    "128 px",
    "192 px",
    "256 px",
];
const SNAP_DISTANCE_LABELS: [&str; 9] = [
    "Exact", "4 px", "8 px", "12 px", "16 px", "24 px", "32 px", "48 px", "64 px",
];
const TASKBAR_SIZE_LABELS: [&str; 9] = [
    "28 px", "32 px", "36 px", "40 px", "44 px", "48 px", "52 px", "60 px", "72 px",
];
const MENU_DENSITY_LABELS: [&str; 5] = ["Dense", "Compact", "Balanced", "Comfortable", "Large"];
const ANIMATION_LABELS: [&str; 5] = ["Off", "Fast", "Balanced", "Smooth", "Cinematic"];
const UI_SCALE_LABELS: [&str; 7] = ["100%", "80%", "90%", "110%", "125%", "150%", "200%"];
const MENU_LAYOUT_LABELS: [&str; 4] = ["List", "Grid", "Compact grid", "Dashboard"];
const WINDOW_TILING_LABELS: [&str; 3] = ["Halves", "Thirds", "Quarters"];
const AUDIO_VOLUME_LABELS: [&str; 21] = [
    "0%", "5%", "10%", "15%", "20%", "25%", "30%", "35%", "40%", "45%", "50%", "55%", "60%", "65%",
    "70%", "75%", "80%", "85%", "90%", "95%", "100%",
];
const TERMINAL_SCALE_LABELS: [&str; 3] = ["Small", "Medium", "Large"];
const TERMINAL_COLOR_LABELS: [&str; 8] = [
    "Default", "White", "Green", "Cyan", "Amber", "Rose", "Blue", "Black",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AccentChoice(u8);

impl AccentChoice {
    const fn color(self) -> u32 {
        match self.0 {
            0 => color::GREEN,
            1 => color::CYAN,
            2 => color::PURPLE,
            3 => 0x00B6_8B50,
            value => spectrum_color(value),
        }
    }

    const fn label(self) -> &'static str {
        match self.0 {
            0 => "Forest 1/256",
            1 => "Ocean 2/256",
            2 => "Violet 3/256",
            3 => "Amber 4/256",
            4..=42 => "Spectrum red",
            43..=85 => "Spectrum amber",
            86..=128 => "Spectrum green",
            129..=171 => "Spectrum cyan",
            172..=214 => "Spectrum blue",
            _ => "Spectrum violet",
        }
    }

    fn shifted(self, direction: i8) -> Self {
        let step = direction.unsigned_abs().max(1);
        Self(if direction < 0 {
            self.0.wrapping_sub(step)
        } else {
            self.0.wrapping_add(step)
        })
    }

    const fn from_persisted(value: u8) -> Self {
        Self(value)
    }

    const fn persisted(self) -> u8 {
        self.0
    }
}

const fn spectrum_color(value: u8) -> u32 {
    let wheel = value as u16 * 6;
    let raw_sector = wheel / 256;
    let sector = if raw_sector > 5 { 5 } else { raw_sector };
    let offset = wheel % 256;
    let rising = 64 + (offset * 191 / 255) as u8;
    let falling = 255 - (offset * 191 / 255) as u8;
    let (red, green, blue) = match sector {
        0 => (255, rising, 64),
        1 => (falling, 255, 64),
        2 => (64, 255, rising),
        3 => (64, falling, 255),
        4 => (rising, 64, 255),
        _ => (255, 64, falling),
    };
    ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}

/// Mix two XRGB colors with an allocation-free, integer-only blend. Procedural
/// themes use this to keep every palette dark enough for the existing white
/// text and bounded bitmap renderer while still providing a broad hue range.
const fn blend_color(background: u32, foreground: u32, foreground_alpha: u8) -> u32 {
    let inverse = 255_u32 - foreground_alpha as u32;
    let alpha = foreground_alpha as u32;
    let red = (((background >> 16) & 0xFF) * inverse + ((foreground >> 16) & 0xFF) * alpha) / 255;
    let green = (((background >> 8) & 0xFF) * inverse + ((foreground >> 8) & 0xFF) * alpha) / 255;
    let blue = ((background & 0xFF) * inverse + (foreground & 0xFF) * alpha) / 255;
    (red << 16) | (green << 8) | blue
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackdropChoice {
    Graphite,
    Midnight,
    Black,
    Custom(u8),
}

impl BackdropChoice {
    const fn color(self) -> u32 {
        match self {
            Self::Graphite => color::BACKGROUND,
            Self::Midnight => 0x0009_1019,
            Self::Black => 0x0000_0000,
            Self::Custom(value) => blend_color(0x0002_0406, spectrum_color(value), 52),
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Graphite => "Graphite",
            Self::Midnight => "Midnight",
            Self::Black => "Black",
            Self::Custom(3..=44) => "Custom ember",
            Self::Custom(45..=86) => "Custom amber",
            Self::Custom(87..=128) => "Custom forest",
            Self::Custom(129..=170) => "Custom ocean",
            Self::Custom(171..=212) => "Custom blue",
            Self::Custom(_) => "Custom violet",
        }
    }

    fn shifted(self, direction: i8) -> Self {
        let step = direction.unsigned_abs().max(1);
        let next = if direction < 0 {
            self.persisted().wrapping_sub(step)
        } else {
            self.persisted().wrapping_add(step)
        };
        Self::from_persisted(next)
    }

    const fn from_persisted(value: u8) -> Self {
        match value {
            1 => Self::Midnight,
            2 => Self::Black,
            3..=u8::MAX => Self::Custom(value),
            _ => Self::Graphite,
        }
    }

    const fn persisted(self) -> u8 {
        match self {
            Self::Graphite => 0,
            Self::Midnight => 1,
            Self::Black => 2,
            Self::Custom(value) => value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThemeChoice {
    Obsidian,
    Graphite,
    Nord,
    Forest,
    Aurora,
    Rose,
    Custom(u8),
}

impl ThemeChoice {
    const fn label(self) -> &'static str {
        match self {
            Self::Obsidian => "Obsidian",
            Self::Graphite => "Graphite",
            Self::Nord => "Nord",
            Self::Forest => "Forest",
            Self::Aurora => "Aurora",
            Self::Rose => "Rose",
            Self::Custom(6..=47) => "Custom ember",
            Self::Custom(48..=89) => "Custom amber",
            Self::Custom(90..=131) => "Custom forest",
            Self::Custom(132..=173) => "Custom ocean",
            Self::Custom(174..=215) => "Custom blue",
            Self::Custom(_) => "Custom violet",
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
            Self::Custom(value) => blend_color(0x000C_1014, spectrum_color(value), 68),
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
            Self::Custom(value) => blend_color(0x0005_080B, spectrum_color(value), 48),
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
            Self::Custom(value) => blend_color(0x0007_0A0D, spectrum_color(value), 38),
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
            Self::Custom(value) => blend_color(0x0012_171C, spectrum_color(value), 78),
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
            Self::Custom(value) => blend_color(0x0000_0102, spectrum_color(value), 30),
        }
    }

    const fn from_persisted(value: u8) -> Self {
        match value {
            1 => Self::Graphite,
            2 => Self::Nord,
            3 => Self::Forest,
            4 => Self::Aurora,
            5 => Self::Rose,
            6..=u8::MAX => Self::Custom(value),
            _ => Self::Obsidian,
        }
    }

    const fn persisted(self) -> u8 {
        match self {
            Self::Obsidian => 0,
            Self::Graphite => 1,
            Self::Nord => 2,
            Self::Forest => 3,
            Self::Aurora => 4,
            Self::Rose => 5,
            Self::Custom(value) => value,
        }
    }

    fn shifted(self, direction: i8) -> Self {
        let step = direction.unsigned_abs().max(1);
        let next = if direction < 0 {
            self.persisted().wrapping_sub(step)
        } else {
            self.persisted().wrapping_add(step)
        };
        Self::from_persisted(next)
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
    wallpaper_variant: u8,
    cursor: CursorChoice,
    accent: AccentChoice,
    backdrop: BackdropChoice,
    pure_black_apps: bool,
    rounded_controls: bool,
    taskbar_visible: bool,
    status_visible: bool,
    window_borders: bool,
    high_contrast: bool,
    window_shadows: bool,
    wallpaper_effects: bool,
    responsive_presentation: bool,
    pointer_speed: u8,
    scroll_speed: u8,
    refresh_rate: state::RefreshRate,
    vsync: bool,
    font_face: framebuffer::FontFace,
    font_weight: framebuffer::FontWeight,
    window_corner_radius: u8,
    window_border_width: u8,
    titlebar_density: u8,
    taskbar_placement: TaskbarPlacement,
    taskbar_size: u8,
    taskbar_alignment: TaskbarAlignment,
    taskbar_autohide: bool,
    taskbar_translucent: bool,
    taskbar_labels: bool,
    clock_seconds: bool,
    window_offscreen_allowance: u8,
    window_snap_distance: u8,
    window_snap: bool,
    focus_policy: u8,
    window_opacity: u8,
    cursor_shadow: bool,
    menu_density: u8,
    animation_level: u8,
    ui_scale: u8,
    tooltips: bool,
    notification_animations: bool,
    menu_show_builtins: bool,
    menu_show_installed: bool,
    menu_grid: bool,
    menu_categories: bool,
    menu_layout: u8,
    terminal_font_face: framebuffer::FontFace,
    terminal_font_weight: framebuffer::FontWeight,
    terminal_scale: u8,
    terminal_foreground: u8,
    terminal_background: u8,
    locale: u8,
    timezone: u8,
    clock_24h: bool,
    clock_offset_quarters: i8,
    date_format: u8,
    week_starts_monday: bool,
    taskbar_widgets: u8,
    reduce_transparency: bool,
    focus_ring: bool,
    window_tiling: u8,
    audio_volume: u8,
    audio_muted: bool,
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

    const fn wallpaper_color(self) -> u32 {
        if self.wallpaper_variant < 7 {
            self.accent.color()
        } else {
            spectrum_color(self.wallpaper_variant)
        }
    }

    const fn border_color(self, focused: bool) -> u32 {
        if focused && self.focus_ring {
            self.accent.color()
        } else if self.high_contrast {
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

    const fn presentation_policy_label(self) -> &'static str {
        if self.responsive_presentation {
            "Responsive"
        } else {
            "Efficient"
        }
    }

    fn window_corner_radius(self) -> i32 {
        state::WINDOW_CORNER_RADII_PX[self.window_corner_radius.min(8) as usize] as i32
    }

    fn window_border_width(self) -> i32 {
        state::WINDOW_BORDER_WIDTHS_PX[self.window_border_width.min(6) as usize] as i32
    }

    fn titlebar_height(self) -> i16 {
        state::TITLEBAR_HEIGHTS_PX[self.titlebar_density.min(4) as usize] as i16
    }

    fn window_alpha(self) -> u8 {
        if self.reduce_transparency {
            u8::MAX
        } else {
            state::WINDOW_OPACITY_ALPHA[self.window_opacity.min(5) as usize]
        }
    }

    fn offscreen_pixels(self) -> i32 {
        state::WINDOW_OFFSCREEN_ALLOWANCES_PX[self.window_offscreen_allowance.min(8) as usize]
            as i32
    }

    fn snap_pixels(self) -> i32 {
        state::WINDOW_SNAP_DISTANCES_PX[self.window_snap_distance.min(8) as usize] as i32
    }

    fn focus_policy_label(self) -> &'static str {
        state::FOCUS_POLICY_NAMES[self.focus_policy.min(2) as usize]
    }

    fn from_persistent(value: state::PersistentPreferences) -> Self {
        let flags = value.flags;
        Self {
            theme: ThemeChoice::from_persisted(value.theme),
            wallpaper: WallpaperChoice::from_persisted(value.wallpaper % 7),
            wallpaper_variant: value
                .wallpaper
                .min((state::WALLPAPER_VARIANT_CHOICES - 1) as u8),
            cursor: CursorChoice::from_persisted(value.cursor_theme),
            accent: AccentChoice::from_persisted(value.accent),
            backdrop: BackdropChoice::from_persisted(value.backdrop),
            pure_black_apps: flags & state::PREF_PURE_BLACK_APPS != 0,
            rounded_controls: flags & state::PREF_ROUNDED_CONTROLS != 0,
            taskbar_visible: flags & state::PREF_TASKBAR_VISIBLE != 0,
            status_visible: flags & state::PREF_STATUS_VISIBLE != 0,
            window_borders: flags & state::PREF_WINDOW_BORDERS != 0
                && value.window_border_width != 0,
            high_contrast: flags & state::PREF_HIGH_CONTRAST != 0,
            window_shadows: flags & state::PREF_WINDOW_SHADOWS != 0,
            wallpaper_effects: flags & state::PREF_WALLPAPER_EFFECTS != 0,
            responsive_presentation: flags & state::PREF_RESPONSIVE_PRESENTATION != 0,
            pointer_speed: value.pointer_speed.clamp(1, 3),
            scroll_speed: value.scroll_speed.min(6),
            refresh_rate: value.refresh_rate,
            vsync: value.vsync,
            font_face: framebuffer::FontFace::from_persisted(value.font_face)
                .unwrap_or(framebuffer::FontFace::System),
            font_weight: framebuffer::FontWeight::from_persisted(value.font_weight)
                .unwrap_or(framebuffer::FontWeight::Regular),
            window_corner_radius: value.window_corner_radius,
            window_border_width: value.window_border_width,
            titlebar_density: value.titlebar_density,
            taskbar_placement: TaskbarPlacement::from_persisted(value.taskbar_placement),
            taskbar_size: value.taskbar_size,
            taskbar_alignment: TaskbarAlignment::from_persisted(value.taskbar_alignment),
            taskbar_autohide: value.taskbar_autohide,
            taskbar_translucent: value.taskbar_translucent,
            taskbar_labels: value.taskbar_labels,
            clock_seconds: value.clock_seconds,
            window_offscreen_allowance: value.window_offscreen_allowance,
            window_snap_distance: value.window_snap_distance,
            window_snap: value.window_snap,
            focus_policy: value.focus_policy,
            window_opacity: value.window_opacity,
            cursor_shadow: value.cursor_shadow,
            menu_density: value.menu_density,
            animation_level: value.animation_level,
            ui_scale: value.ui_scale,
            tooltips: value.tooltips,
            notification_animations: value.notification_animations,
            menu_show_builtins: flags & state::PREF_MENU_HIDE_BUILTINS == 0,
            menu_show_installed: flags & state::PREF_MENU_HIDE_INSTALLED == 0,
            menu_grid: flags & state::PREF_MENU_GRID != 0,
            menu_categories: flags & state::PREF_MENU_CATEGORIES != 0,
            menu_layout: u8::from(flags & state::PREF_MENU_GRID != 0),
            terminal_font_face: framebuffer::FontFace::System,
            terminal_font_weight: framebuffer::FontWeight::Regular,
            terminal_scale: 0,
            terminal_foreground: 0,
            terminal_background: 0,
            locale: crate::locale::active().persisted(),
            timezone: 3,
            clock_24h: true,
            clock_offset_quarters: 0,
            date_format: 0,
            week_starts_monday: true,
            taskbar_widgets: 0,
            reduce_transparency: false,
            focus_ring: false,
            window_tiling: 0,
            audio_volume: 80,
            audio_muted: false,
        }
    }

    fn load_ui_extension(mut self, cfc: CfcFin) -> Self {
        let Some((bytes, length)) = crate::expfs_store::load_data_form(cfc, DESKTOP_UI_STATE)
        else {
            return self;
        };
        if length < 11
            || (bytes[..4] != DESKTOP_UI_MAGIC_V1
                && bytes[..4] != DESKTOP_UI_MAGIC_V2
                && bytes[..4] != DESKTOP_UI_MAGIC_V3)
        {
            return self;
        }
        self.locale = bytes[4].min(4);
        self.menu_layout = bytes[5].min(3);
        self.menu_grid = self.menu_layout != 0;
        self.terminal_font_face = framebuffer::FontFace::from_persisted(bytes[6])
            .unwrap_or(framebuffer::FontFace::System);
        self.terminal_font_weight = framebuffer::FontWeight::from_persisted(bytes[7])
            .unwrap_or(framebuffer::FontWeight::Regular);
        self.terminal_scale = bytes[8].min(2);
        self.terminal_foreground = bytes[9].min(7);
        self.terminal_background = bytes[10].min(7);
        if (bytes[..4] == DESKTOP_UI_MAGIC_V2 || bytes[..4] == DESKTOP_UI_MAGIC_V3) && length >= 17
        {
            self.timezone = bytes[11].min((TIMEZONE_LABELS.len() - 1) as u8);
            self.clock_24h = bytes[12] != 0;
            self.clock_offset_quarters = (bytes[13] as i8).clamp(-48, 48);
            self.date_format = bytes[14].min((DATE_FORMAT_LABELS.len() - 1) as u8);
            self.week_starts_monday = bytes[15] != 0;
            self.taskbar_widgets = bytes[16]
                & (TASKBAR_WIDGET_DATE
                    | TASKBAR_WIDGET_ACTIVE_APP
                    | TASKBAR_WIDGET_WEATHER
                    | TASKBAR_WIDGET_PERFORMANCE
                    | TASKBAR_WIDGET_AUDIO
                    | TASKBAR_WIDGET_TIMEZONE);
        }
        if bytes[..4] == DESKTOP_UI_MAGIC_V3 && length >= 22 {
            self.audio_volume = bytes[17].min(100);
            self.audio_muted = bytes[18] != 0;
            self.reduce_transparency = bytes[19] != 0;
            self.focus_ring = bytes[20] != 0;
            self.window_tiling = bytes[21].min(2);
        }
        self
    }

    fn encode_ui_extension(self) -> [u8; 22] {
        [
            DESKTOP_UI_MAGIC_V3[0],
            DESKTOP_UI_MAGIC_V3[1],
            DESKTOP_UI_MAGIC_V3[2],
            DESKTOP_UI_MAGIC_V3[3],
            self.locale,
            self.menu_layout,
            self.terminal_font_face.persisted(),
            self.terminal_font_weight.persisted(),
            self.terminal_scale,
            self.terminal_foreground,
            self.terminal_background,
            self.timezone,
            u8::from(self.clock_24h),
            self.clock_offset_quarters as u8,
            self.date_format,
            u8::from(self.week_starts_monday),
            self.taskbar_widgets,
            self.audio_volume,
            u8::from(self.audio_muted),
            u8::from(self.reduce_transparency),
            u8::from(self.focus_ring),
            self.window_tiling,
        ]
    }

    fn with_profile(self, profile: CustomizationProfile) -> Self {
        // Profiles intentionally leave display timing and the two system-wide
        // visibility controls alone. They restyle the desktop without
        // unexpectedly changing output cadence or hiding connectivity state.
        let mut next = Self::from_persistent(state::PersistentPreferences::new());
        next.refresh_rate = self.refresh_rate;
        next.vsync = self.vsync;
        next.taskbar_visible = self.taskbar_visible;
        next.status_visible = self.status_visible;
        // Appearance profiles must not silently replace separately chosen
        // launcher, language or terminal preferences.
        next.menu_layout = self.menu_layout;
        next.menu_grid = self.menu_layout != 0;
        next.terminal_font_face = self.terminal_font_face;
        next.terminal_font_weight = self.terminal_font_weight;
        next.terminal_scale = self.terminal_scale;
        next.terminal_foreground = self.terminal_foreground;
        next.terminal_background = self.terminal_background;
        next.locale = self.locale;
        next.timezone = self.timezone;
        next.clock_24h = self.clock_24h;
        next.clock_offset_quarters = self.clock_offset_quarters;
        next.date_format = self.date_format;
        next.week_starts_monday = self.week_starts_monday;
        next.taskbar_widgets = self.taskbar_widgets;
        next.audio_volume = self.audio_volume;
        next.audio_muted = self.audio_muted;
        next.reduce_transparency = self.reduce_transparency;
        next.focus_ring = self.focus_ring;
        next.window_tiling = self.window_tiling;

        match profile {
            CustomizationProfile::Balanced => {}
            CustomizationProfile::Compact => {
                next.theme = ThemeChoice::Graphite;
                next.pure_black_apps = false;
                next.rounded_controls = false;
                next.font_face = framebuffer::FontFace::from_persisted(3)
                    .unwrap_or(framebuffer::FontFace::System);
                next.window_corner_radius = 1;
                next.window_border_width = 1;
                next.window_borders = true;
                next.titlebar_density = 0;
                next.taskbar_size = 0;
                next.menu_density = 0;
                next.ui_scale = 1;
                next.animation_level = 1;
            }
            CustomizationProfile::Focus => {
                next.theme = ThemeChoice::Obsidian;
                next.wallpaper = WallpaperChoice::Solid;
                next.wallpaper_variant = 0;
                next.backdrop = BackdropChoice::Black;
                next.pure_black_apps = true;
                next.rounded_controls = false;
                next.window_borders = false;
                next.window_border_width = 0;
                next.window_shadows = false;
                next.wallpaper_effects = false;
                next.taskbar_autohide = true;
                next.taskbar_translucent = false;
                next.taskbar_labels = false;
                next.animation_level = 0;
                next.tooltips = false;
                next.notification_animations = false;
            }
            CustomizationProfile::Accessible => {
                next.theme = ThemeChoice::Nord;
                next.cursor = CursorChoice::Crosshair;
                next.high_contrast = true;
                next.font_face = framebuffer::FontFace::from_persisted(1)
                    .unwrap_or(framebuffer::FontFace::System);
                next.font_weight = framebuffer::FontWeight::Bold;
                next.window_corner_radius = 4;
                next.window_border_width = 4;
                next.window_borders = true;
                next.titlebar_density = 4;
                next.taskbar_size = 8;
                next.taskbar_labels = true;
                next.cursor_shadow = true;
                next.menu_density = 4;
                next.ui_scale = 6;
                next.animation_level = 0;
                next.tooltips = true;
                next.notification_animations = false;
                next.reduce_transparency = true;
                next.focus_ring = true;
            }
            CustomizationProfile::Showcase => {
                next.theme = ThemeChoice::Aurora;
                next.wallpaper = WallpaperChoice::Aurora;
                next.wallpaper_variant = 145;
                next.cursor = CursorChoice::Accent;
                next.accent = AccentChoice::from_persisted(145);
                next.backdrop = BackdropChoice::Midnight;
                next.pure_black_apps = false;
                next.rounded_controls = true;
                next.window_shadows = true;
                next.wallpaper_effects = true;
                next.responsive_presentation = true;
                next.window_corner_radius = 7;
                next.window_border_width = 2;
                next.window_borders = true;
                next.titlebar_density = 2;
                next.window_opacity = 2;
                next.taskbar_size = 4;
                next.taskbar_alignment = TaskbarAlignment::Center;
                next.taskbar_translucent = true;
                next.taskbar_labels = true;
                next.cursor_shadow = true;
                next.menu_density = 2;
                next.animation_level = 3;
                next.ui_scale = 4;
                next.tooltips = true;
                next.notification_animations = true;
                next.menu_grid = true;
                next.menu_categories = true;
            }
            CustomizationProfile::Touch => {
                next.theme = ThemeChoice::Forest;
                next.cursor = CursorChoice::Accent;
                next.rounded_controls = true;
                next.pointer_speed = 2;
                next.window_corner_radius = 6;
                next.window_border_width = 3;
                next.window_borders = true;
                next.titlebar_density = 4;
                next.taskbar_size = 8;
                next.taskbar_labels = true;
                next.cursor_shadow = true;
                next.menu_density = 4;
                next.animation_level = 2;
                next.ui_scale = 6;
                next.tooltips = true;
                next.menu_grid = true;
                next.menu_categories = true;
            }
            CustomizationProfile::Night => {
                next.theme = ThemeChoice::Rose;
                next.wallpaper = WallpaperChoice::Gradient;
                next.wallpaper_variant = 226;
                next.accent = AccentChoice::from_persisted(226);
                next.backdrop = BackdropChoice::Black;
                next.window_shadows = false;
                next.wallpaper_effects = true;
                next.reduce_transparency = true;
                next.animation_level = 1;
                next.notification_animations = false;
            }
            CustomizationProfile::Presentation => {
                next.theme = ThemeChoice::Nord;
                next.high_contrast = true;
                next.font_weight = framebuffer::FontWeight::Bold;
                next.titlebar_density = 3;
                next.taskbar_size = 5;
                next.taskbar_labels = true;
                next.taskbar_autohide = false;
                next.ui_scale = 4;
                next.focus_ring = true;
                next.reduce_transparency = true;
                next.window_tiling = 0;
                next.animation_level = 1;
                next.tooltips = true;
            }
        }
        next
    }

    fn update_persistent(
        self,
        mut value: state::PersistentPreferences,
    ) -> state::PersistentPreferences {
        value.display_mode = framebuffer::requested_mode().persisted();
        value.theme = self.theme.persisted();
        value.wallpaper = self.wallpaper_variant;
        value.cursor_theme = self.cursor as u8;
        value.accent = self.accent.persisted();
        value.backdrop = self.backdrop.persisted();
        value.pointer_speed = self.pointer_speed;
        value.scroll_speed = self.scroll_speed;
        value.refresh_rate = self.refresh_rate;
        value.vsync = self.vsync;
        value.font_face = self.font_face.persisted();
        value.font_weight = self.font_weight.persisted();
        value.window_corner_radius = self.window_corner_radius;
        value.window_border_width = self.window_border_width;
        value.titlebar_density = self.titlebar_density;
        value.taskbar_placement = self.taskbar_placement as u8;
        value.taskbar_size = self.taskbar_size;
        value.taskbar_alignment = self.taskbar_alignment as u8;
        value.taskbar_autohide = self.taskbar_autohide;
        value.taskbar_translucent = self.taskbar_translucent;
        value.taskbar_labels = self.taskbar_labels;
        value.clock_seconds = self.clock_seconds;
        value.window_offscreen_allowance = self.window_offscreen_allowance;
        value.window_snap_distance = self.window_snap_distance;
        value.window_snap = self.window_snap;
        value.focus_policy = self.focus_policy;
        value.window_opacity = self.window_opacity;
        value.cursor_shadow = self.cursor_shadow;
        value.menu_density = self.menu_density;
        value.animation_level = self.animation_level;
        value.ui_scale = self.ui_scale;
        value.tooltips = self.tooltips;
        value.notification_animations = self.notification_animations;
        let desktop_flags = state::PREF_PURE_BLACK_APPS
            | state::PREF_ROUNDED_CONTROLS
            | state::PREF_TASKBAR_VISIBLE
            | state::PREF_STATUS_VISIBLE
            | state::PREF_WINDOW_BORDERS
            | state::PREF_HIGH_CONTRAST
            | state::PREF_NETWORK_ENABLED
            | state::PREF_WIFI_ENABLED
            | state::PREF_BLUETOOTH_ENABLED
            | state::PREF_WINDOW_SHADOWS
            | state::PREF_WALLPAPER_EFFECTS
            | state::PREF_RESPONSIVE_PRESENTATION
            | state::PREF_MENU_HIDE_BUILTINS
            | state::PREF_MENU_HIDE_INSTALLED
            | state::PREF_MENU_GRID
            | state::PREF_MENU_CATEGORIES;
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
        if self.window_shadows {
            value.flags |= state::PREF_WINDOW_SHADOWS;
        }
        if self.wallpaper_effects {
            value.flags |= state::PREF_WALLPAPER_EFFECTS;
        }
        if self.responsive_presentation {
            value.flags |= state::PREF_RESPONSIVE_PRESENTATION;
        }
        if !self.menu_show_builtins {
            value.flags |= state::PREF_MENU_HIDE_BUILTINS;
        }
        if !self.menu_show_installed {
            value.flags |= state::PREF_MENU_HIDE_INSTALLED;
        }
        if self.menu_grid {
            value.flags |= state::PREF_MENU_GRID;
        }
        if self.menu_categories {
            value.flags |= state::PREF_MENU_CATEGORIES;
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
    ayo_transaction_handle: Option<u32>,
    browser_network_handle: Option<u32>,
    apps_network_handle: Option<u32>,
    settings_radio_handle: Option<u32>,
    app_open: [bool; APP_COUNT],
    app_ever_opened: [bool; APP_COUNT],
    app_minimized: [bool; APP_COUNT],
    panel_surface: u32,
    launcher_surface: u32,
    active: AppKind,
    launcher_open: bool,
    launcher_scroll: usize,
    launcher_selection: usize,
    shell_confirmation: bool,
    fullscreen: bool,
    preferences: DesktopPreferences,
    settings_category: SettingsCategory,
    settings_row: usize,
    settings_notice: &'static str,
    document: BrowserContainer,
    browser_line: [u8; 512],
    browser_len: usize,
    browser_editing: bool,
    browser_scroll: i32,
    browser_tabs: [BrowserTab; BROWSER_TAB_CAPACITY],
    browser_tab_count: usize,
    browser_active_tab: usize,
    browser_history_locked: bool,
    browser_bookmarks: [BrowserBookmark; BROWSER_BOOKMARK_CAPACITY],
    browser_bookmark_count: usize,
    browser_find_line: [u8; BROWSER_FIND_CAPACITY],
    browser_find_len: usize,
    browser_find_editing: bool,
    browser_find_match: Option<usize>,
    browser_web_state: BrowserWebState,
    browser_resources: BrowserResourceStats,
    browser_media: Box<BrowserMediaCache>,
    last_weather_request_ticks: u64,
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
    notes_name: [u8; 32],
    notes_name_len: usize,
    notes_editing_name: bool,
    notes_status: &'static str,
    games: crate::games::GameHub,
    native_apps: crate::apps::NativeApps,
    cfc: CfcFin,
    session: crate::session::Session,
    cursor: PointerCursor,
    dragging: Option<AppKind>,
    should_exit: bool,
    frame_pacer: FramePacer,
    responsive_commits: u64,
    full_frame_commits: u64,
    damaged_frame_commits: u64,
    frame_callbacks: u64,
    pointer_packets_merged: u64,
    deferred_presents: u64,
    full_redraw_requested: bool,
    display_debug_overlay: bool,
}

struct PointerCursor {
    x: i16,
    y: i16,
    under: [u32; CURSOR_WIDTH * CURSOR_HEIGHT],
    style: CursorChoice,
    accent: u32,
    shadow: bool,
    drawn: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PointerRender {
    None,
    Cursor(framebuffer::DamageRegion),
    Full,
}

impl PointerCursor {
    fn new(style: CursorChoice, accent: u32, shadow: bool) -> Self {
        Self {
            x: (framebuffer::width() / 2) as i16,
            y: (framebuffer::height() / 2) as i16,
            under: [0; CURSOR_WIDTH * CURSOR_HEIGHT],
            style,
            accent,
            shadow,
            drawn: false,
        }
    }

    fn set_style(&mut self, style: CursorChoice, accent: u32) {
        self.restore();
        self.style = style;
        self.accent = accent;
    }

    fn set_shadow(&mut self, shadow: bool) {
        self.restore();
        self.shadow = shadow;
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
        if self.shadow {
            for (row, bits) in SHAPE.iter().copied().enumerate().take(CURSOR_HEIGHT - 2) {
                for column in 0..CURSOR_WIDTH - 2 {
                    if bits & (1 << column) != 0 {
                        framebuffer::pixel(
                            self.x as i32 + column as i32 + 2,
                            self.y as i32 + row as i32 + 2,
                            0x0003_0508,
                        );
                    }
                }
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
        if self.shadow {
            framebuffer::line(
                self.x as i32 + 2,
                self.y as i32 + 9,
                self.x as i32 + CURSOR_WIDTH as i32 - 1,
                self.y as i32 + 9,
                0x0003_0508,
            );
            framebuffer::line(
                self.x as i32 + 9,
                self.y as i32 + 2,
                self.x as i32 + 9,
                self.y as i32 + CURSOR_HEIGHT as i32 - 1,
                0x0003_0508,
            );
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
    fn app_enabled(&self, app: AppKind) -> bool {
        app != AppKind::Browser || crate::security::browser_allowed()
    }

    const fn locale(&self) -> Locale {
        Locale::from_persisted(self.preferences.locale)
    }

    fn new(
        start_app: Option<AppKind>,
        session: crate::session::Session,
        allow_network: bool,
        cfc: CfcFin,
    ) -> Self {
        let start_app =
            start_app.filter(|app| *app != AppKind::Browser || crate::security::browser_allowed());
        let active = start_app.unwrap_or(AppKind::Terminal);
        let preferences =
            DesktopPreferences::from_persistent(state::preferences()).load_ui_extension(cfc);
        audio::set_volume(preferences.audio_volume);
        audio::set_muted(preferences.audio_muted);
        framebuffer::set_font_style(framebuffer::FontStyle::new(
            preferences.font_face,
            preferences.font_weight,
        ));
        let frame_pacer = FramePacer::new(timing_config(preferences), crate::hardware::timestamp());
        let mut server = DisplayServer::new();
        let mut broker = CapabilityBroker::new(cfc);
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
        let dock_rect = taskbar_rect(preferences);
        let panel = server
            .create_surface(DISPLAY_FIN, "Panel", SurfaceRole::Panel, dock_rect)
            .expect("desktop panel surface");

        let mut app_surfaces = [0; APP_COUNT];
        let mut app_handles = [0; APP_COUNT];
        for (index, app) in AppKind::ALL.iter().copied().enumerate() {
            let rect = default_rect(app, preferences);
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
        let apps_network_handle = if allow_network && network::available() {
            broker
                .issue_for(
                    APPS_FIN,
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
        let ayo_transaction_handle = broker
            .issue_for(
                PACKAGES_FIN,
                session.authority(),
                AYO_FIN,
                STABLE_FIN,
                Operations::PACKAGE,
                u64::MAX,
            )
            .ok()
            .map(|handle| handle.id);

        // Desktop application authority is complete at activation. ExpSeal
        // closes later ambient/root issuance while preserving each existing
        // Handle's normal narrow delegation and revocation behavior.
        broker
            .seal_context(DISPLAY_FIN, STABLE_FIN)
            .expect("compositor ExpSeal");
        for app in AppKind::ALL.iter().copied() {
            broker
                .seal_context(app.owner(), STABLE_FIN)
                .expect("application ExpSeal");
        }

        let launcher_surface = server
            .create_surface(
                DISPLAY_FIN,
                "Applications",
                SurfaceRole::Popup,
                launcher_rect(preferences),
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
                framebuffer::height() as u16,
            ),
        );
        let _ = server.set_visible(
            DISPLAY_FIN,
            panel,
            preferences.taskbar_visible && !preferences.taskbar_autohide,
        );
        let _ = server.attach(
            DISPLAY_FIN,
            launcher_surface,
            buffer(20, DISPLAY_FIN, LAUNCHER_GRID_WIDTH, LAUNCHER_HEIGHT),
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
        let mut browser_tabs = [BrowserTab::EMPTY; BROWSER_TAB_CAPACITY];
        browser_tabs[0] = BrowserTab::home();
        let mut notes = [0_u8; NOTES_CAPACITY];
        let mut notes_len = 0;
        let mut notes_name = [0_u8; 32];
        let mut notes_name_len = 12;
        notes_name[..notes_name_len].copy_from_slice(b"Untitled.txt");
        let mut notes_status = "Unsaved Data Form";
        if let Some((name, content, length)) = crate::expfs_store::load_text_form(cfc) {
            notes_len = length.min(notes.len());
            notes[..notes_len].copy_from_slice(&content[..notes_len]);
            notes_name_len = name.as_str().len().min(notes_name.len());
            notes_name[..notes_name_len]
                .copy_from_slice(&name.as_str().as_bytes()[..notes_name_len]);
            notes_status = "Loaded from ExpFS";
        }
        let native_apps = crate::expfs_store::load_data_form(cfc, "AyoApps.state")
            .map(|(content, length)| crate::apps::NativeApps::restore_state(&content[..length]))
            .unwrap_or_else(crate::apps::NativeApps::new);
        let browser_web_state = crate::expfs_store::load_data_form(cfc, BROWSER_DATA_STATE)
            .map(|(content, length)| BrowserWebState::restore_local(&content[..length]))
            .unwrap_or_default();
        let mut state = Self {
            server,
            broker,
            app_surfaces,
            app_handles,
            ayo_transaction_handle,
            browser_network_handle,
            apps_network_handle,
            settings_radio_handle,
            app_open: core::array::from_fn(|index| start_app == Some(AppKind::ALL[index])),
            app_ever_opened: core::array::from_fn(|index| start_app == Some(AppKind::ALL[index])),
            app_minimized: [false; APP_COUNT],
            panel_surface: panel,
            launcher_surface,
            active,
            launcher_open: false,
            launcher_scroll: 0,
            launcher_selection: 0,
            shell_confirmation: false,
            fullscreen: false,
            preferences,
            settings_category: SettingsCategory::System,
            settings_row: 0,
            settings_notice: "Changes are saved locally.",
            document: BrowserContainer::new(
                Document::parse("expos://home", HOME).expect("built-in home document"),
            ),
            browser_line: [0; 512],
            browser_len: 0,
            browser_editing: false,
            browser_scroll: 0,
            browser_tabs,
            browser_tab_count: 1,
            browser_active_tab: 0,
            browser_history_locked: false,
            browser_bookmarks: [BrowserBookmark::EMPTY; BROWSER_BOOKMARK_CAPACITY],
            browser_bookmark_count: 0,
            browser_find_line: [0; BROWSER_FIND_CAPACITY],
            browser_find_len: 0,
            browser_find_editing: false,
            browser_find_match: None,
            browser_web_state,
            browser_resources: BrowserResourceStats::default(),
            browser_media: Box::new(BrowserMediaCache::new()),
            last_weather_request_ticks: 0,
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
            notes,
            notes_len,
            notes_name,
            notes_name_len,
            notes_editing_name: false,
            notes_status,
            games: crate::games::GameHub::new(),
            native_apps,
            cfc,
            session,
            cursor: PointerCursor::new(
                preferences.cursor,
                preferences.accent.color(),
                preferences.cursor_shadow,
            ),
            dragging: None,
            should_exit: false,
            frame_pacer,
            responsive_commits: 0,
            full_frame_commits: 0,
            damaged_frame_commits: 0,
            frame_callbacks: 0,
            pointer_packets_merged: 0,
            deferred_presents: 0,
            full_redraw_requested: false,
            display_debug_overlay: false,
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
            slog!("EXPOS_SETTING_PERSIST_FAILED error={:?}\r\n", error);
        }
        let extension = self.preferences.encode_ui_extension();
        if let Err(error) =
            crate::expfs_store::save_data_form(self.cfc, DESKTOP_UI_STATE, &extension)
        {
            self.settings_notice = error.message();
            slog!("EXPOS_UI_PERSIST_FAILED error={:?}\r\n", error);
        }
    }

    fn persist_browser_web_state(&mut self) {
        let mut encoded = [0_u8; crate::expfs_store::FORM_CONTENT_CAPACITY];
        let length = self.browser_web_state.encode_local(&mut encoded);
        if length == 0 {
            return;
        }
        if let Err(error) =
            crate::expfs_store::save_data_form(self.cfc, BROWSER_DATA_STATE, &encoded[..length])
        {
            slog!("EXPOS_BROWSER_STORAGE_PERSIST_FAILED error={:?}\r\n", error);
        }
    }

    fn reconfigure_presentation(&mut self) {
        self.frame_pacer.reconfigure(
            timing_config(self.preferences),
            crate::hardware::timestamp(),
        );
    }

    fn usable_area(&self) -> Rect {
        work_area(self.preferences)
    }

    fn fullscreen_rect(&self) -> Rect {
        let area = self.usable_area();
        Rect::new(
            area.x.saturating_add(8),
            area.y.saturating_add(8),
            area.width.saturating_sub(16).max(1),
            area.height.saturating_sub(16).max(1),
        )
    }

    fn pointer_reveals_taskbar(&self) -> bool {
        let edge = 3;
        contains(taskbar_rect(self.preferences), self.cursor.x, self.cursor.y)
            || match self.preferences.taskbar_placement {
                TaskbarPlacement::Bottom => self.cursor.y >= framebuffer::height() as i16 - edge,
                TaskbarPlacement::Top => self.cursor.y < edge,
                TaskbarPlacement::Left => self.cursor.x < edge,
                TaskbarPlacement::Right => self.cursor.x >= framebuffer::width() as i16 - edge,
            }
    }

    fn taskbar_should_show(&self) -> bool {
        self.preferences.taskbar_visible
            && (!self.preferences.taskbar_autohide
                || self.launcher_open
                || self.pointer_reveals_taskbar())
    }

    fn sync_taskbar_visibility(&mut self) -> bool {
        let visible = self.taskbar_should_show();
        let changed = self
            .server
            .surface(self.panel_surface)
            .is_some_and(|surface| surface.current.visible != visible);
        if changed {
            let _ = self
                .server
                .set_visible(DISPLAY_FIN, self.panel_surface, visible);
            let _ = self.server.commit(DISPLAY_FIN, self.panel_surface);
        }
        changed
    }

    fn sync_desktop_geometry(&mut self) {
        let _ = self.server.set_geometry(
            DISPLAY_FIN,
            self.panel_surface,
            taskbar_rect(self.preferences),
        );
        let _ = self.server.commit(DISPLAY_FIN, self.panel_surface);
        let _ = self.server.set_geometry(
            DISPLAY_FIN,
            self.launcher_surface,
            launcher_rect(self.preferences),
        );
        let _ = self.server.commit(DISPLAY_FIN, self.launcher_surface);
        self.sync_taskbar_visibility();
        if self.fullscreen {
            self.set_app_geometry(self.active, self.fullscreen_rect());
        }
        self.full_redraw_requested = true;
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
        let taskbar_changed = self.sync_taskbar_visibility();
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
            return if self.pointer_press(self.cursor.x, self.cursor.y)
                || taskbar_changed
                || self.full_redraw_requested
            {
                PointerRender::Full
            } else if let Some(damage) = cursor_damage {
                PointerRender::Cursor(damage)
            } else {
                PointerRender::None
            };
        }
        if taskbar_changed || self.full_redraw_requested {
            PointerRender::Full
        } else {
            cursor_damage.map_or(PointerRender::None, PointerRender::Cursor)
        }
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
        let pointer_focus = match self.preferences.focus_policy {
            1 => pointer.buttons == 0 && self.dragging.is_none(),
            2 => self.dragging.is_none(),
            _ => false,
        };
        if pointer_focus && app != self.active {
            self.active = app;
            let _ = self.server.focus(surface_id);
            self.full_redraw_requested = true;
        }
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
        let dock = taskbar_rect(self.preferences);
        if self.taskbar_should_show() && contains(dock, x, y) {
            if contains(taskbar_start_rect(self.preferences), x, y) {
                self.toggle_launcher();
                return true;
            }
            let running_count = self.app_open.iter().filter(|open| **open).count();
            let mut running_index = 0_usize;
            for app in AppKind::ALL {
                if !self.app_open[app.index()] {
                    continue;
                }
                if contains(
                    taskbar_app_rect(self.preferences, running_index, running_count),
                    x,
                    y,
                ) {
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
            let launcher = launcher_rect(self.preferences);
            if contains(launcher, x, y) {
                let columns = launcher_columns(self.preferences);
                let row_height = launcher_row_height(self.preferences) as i16;
                let content_x = x - launcher.x - 8;
                let content_y = y - launcher.y - 48;
                if content_x >= 0 && content_y >= 0 {
                    let cell_width = (launcher.width as i16 - 16) / columns as i16;
                    let column = (content_x / cell_width).min(columns as i16 - 1) as usize;
                    let row = (content_y / row_height) as usize;
                    let ordinal = self.launcher_scroll + row * columns + column;
                    if launcher_item(self, ordinal).is_none() {
                        return false;
                    }
                    self.launcher_selection = ordinal;
                    self.activate_launcher_selection();
                    return true;
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
        if y < rect.y + self.preferences.titlebar_height() {
            if x >= right - 42 {
                self.close_active();
            } else if x >= right - 84 {
                self.toggle_fullscreen();
            } else if x >= right - 126 {
                self.minimize_active();
            } else {
                if self.fullscreen {
                    self.fullscreen = false;
                    self.set_app_geometry(app, default_rect(app, self.preferences));
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
        if app == AppKind::Packages {
            let generation = self.native_apps.persistence_generation();
            if let Some(action) = self.native_apps.handle_manager_click(x, y, rect) {
                if self.native_apps.persistence_generation() != generation {
                    self.persist_native_apps();
                }
                self.complete_manager_action(action);
                return true;
            }
        }
        if app == AppKind::Apps {
            let generation = self.native_apps.persistence_generation();
            if self.native_apps.handle_app_click(x, y, rect) {
                if self.native_apps.persistence_generation() != generation {
                    self.persist_native_apps();
                }
                return true;
            }
        }
        if app == AppKind::Notes && self.notes_click(x, y, rect) {
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
        let (next_x, next_y) = self.constrain_window_position(
            rect,
            rect.x as i32 + dx as i32,
            rect.y as i32 + dy as i32,
        );
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
        if !self.app_enabled(next) {
            self.terminal_push("That component is not installed by Genesis.");
            slog!("EXPOS_APP_DENIED app={} genesis-policy\r\n", next.title());
            return;
        }
        self.normalize_fullscreen();
        let was_closed = !self.app_open[next.index()];
        self.app_open[next.index()] = true;
        self.app_minimized[next.index()] = false;
        self.active = next;
        self.fullscreen = false;
        self.close_launcher();
        if was_closed {
            self.set_app_geometry(next, default_rect(next, self.preferences));
            if self.app_ever_opened[next.index()] {
                slog!("EXPOS_APP_REOPENED {}\r\n", next.title());
            } else {
                self.app_ever_opened[next.index()] = true;
                slog!("EXPOS_APP_OPENED {}\r\n", next.title());
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
        slog!("EXPOS_APP_CLOSED {}\r\n", self.active.title());
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
            self.set_app_geometry(self.active, default_rect(self.active, self.preferences));
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
        let rect = if self.fullscreen {
            self.fullscreen_rect()
        } else {
            default_rect(self.active, self.preferences)
        };
        self.set_app_geometry(self.active, rect);
        let _ = self.server.focus(self.active_surface());
    }

    fn normalize_fullscreen(&mut self) {
        if self.fullscreen {
            let app = self.active;
            self.fullscreen = false;
            self.set_app_geometry(app, default_rect(app, self.preferences));
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
            self.launcher_selection = self
                .launcher_selection
                .min(launcher_item_count(self).saturating_sub(1));
            self.sync_taskbar_visibility();
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
            self.sync_taskbar_visibility();
        }
    }

    fn activate_launcher_selection(&mut self) {
        match launcher_item(self, self.launcher_selection) {
            Some(LauncherItem::BuiltIn(app)) => self.focus_existing(app),
            Some(LauncherItem::Installed(index)) if self.native_apps.activate_installed(index) => {
                self.focus_existing(AppKind::Apps);
            }
            Some(LauncherItem::Installed(_)) => {}
            None => {}
        }
    }

    fn move_launcher_selection(&mut self, delta: isize) {
        let total = launcher_item_count(self);
        if total == 0 {
            self.launcher_selection = 0;
            self.launcher_scroll = 0;
            return;
        }
        self.launcher_selection = self
            .launcher_selection
            .saturating_add_signed(delta)
            .min(total - 1);
        let capacity = launcher_capacity(self.preferences);
        if self.launcher_selection < self.launcher_scroll {
            self.launcher_scroll = self.launcher_selection;
        } else if self.launcher_selection >= self.launcher_scroll + capacity {
            self.launcher_scroll = self.launcher_selection + 1 - capacity;
        }
    }

    fn cycle_launcher_selection(&mut self) {
        let total = launcher_item_count(self);
        if total == 0 {
            return;
        }
        if self.launcher_selection + 1 >= total {
            self.launcher_selection = 0;
            self.launcher_scroll = 0;
        } else {
            self.move_launcher_selection(1);
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
        let (x, y) = self.constrain_window_position(
            rect,
            rect.x as i32 + dx as i32,
            rect.y as i32 + dy as i32,
        );
        let _ = self.server.set_position(owner, id, x, y);
        let _ = self.server.commit(owner, id);
    }

    fn tile_active(&mut self, right_half: bool) {
        if !self.has_active_window() {
            return;
        }
        self.fullscreen = false;
        let area = self.usable_area();
        let rect = match self.preferences.window_tiling {
            1 => {
                let third = area.width / 3;
                if right_half {
                    Rect::new(
                        area.x.saturating_add_unsigned(area.width - third),
                        area.y,
                        third,
                        area.height,
                    )
                } else {
                    Rect::new(area.x, area.y, third, area.height)
                }
            }
            2 => {
                let width = area.width / 2;
                let height = area.height / 2;
                let current = self
                    .server
                    .surface(self.active_surface())
                    .map(|surface| surface.current.rect)
                    .unwrap_or(area);
                let bottom = current.y as i32 + current.height as i32 / 2
                    >= area.y as i32 + area.height as i32 / 2;
                Rect::new(
                    if right_half {
                        area.x.saturating_add_unsigned(area.width - width)
                    } else {
                        area.x
                    },
                    if bottom {
                        area.y.saturating_add_unsigned(area.height - height)
                    } else {
                        area.y
                    },
                    width,
                    height,
                )
            }
            _ => {
                let left_width = area.width / 2;
                let right_width = area.width - left_width;
                if right_half {
                    Rect::new(
                        area.x.saturating_add_unsigned(left_width),
                        area.y,
                        right_width,
                        area.height,
                    )
                } else {
                    Rect::new(area.x, area.y, left_width, area.height)
                }
            }
        };
        self.set_app_geometry(self.active, rect);
    }

    fn constrain_window_position(&self, rect: Rect, mut x: i32, mut y: i32) -> (i16, i16) {
        let area = self.usable_area();
        let left = area.x as i32;
        let top = area.y as i32;
        let right = left + area.width as i32;
        let bottom = top + area.height as i32;
        let allowance = self.preferences.offscreen_pixels();
        // The rightmost 126 px are window controls. Preserve a wider visible
        // titlebar segment so a far-left window always remains mouse-draggable.
        let horizontal_overflow = allowance.min((rect.width as i32 - 160).max(0));
        let vertical_overflow =
            allowance.min((rect.height as i32 - self.preferences.titlebar_height() as i32).max(0));

        if self.preferences.window_snap {
            let snap = self.preferences.snap_pixels();
            let right_edge = right - rect.width as i32;
            let bottom_edge = bottom - rect.height as i32;
            let old_x = rect.x as i32;
            let old_y = rect.y as i32;
            let toward_left = (old_x > left && x < old_x) || (old_x < left && x > old_x);
            let toward_right =
                (old_x < right_edge && x > old_x) || (old_x > right_edge && x < old_x);
            let toward_top = (old_y > top && y < old_y) || (old_y < top && y > old_y);
            let toward_bottom =
                (old_y < bottom_edge && y > old_y) || (old_y > bottom_edge && y < old_y);
            if (x - left).abs() <= snap && toward_left {
                x = left;
            } else if (x - right_edge).abs() <= snap && toward_right {
                x = right_edge;
            }
            if (y - top).abs() <= snap && toward_top {
                y = top;
            } else if (y - bottom_edge).abs() <= snap && toward_bottom {
                y = bottom_edge;
            }
        }

        let min_x = left - horizontal_overflow;
        let max_x = (right - rect.width as i32 + horizontal_overflow).max(min_x);
        // Keep a useful grab area visible even with the largest off-screen
        // allowance. Eight visible pixels made a window look corrupted and
        // left its controls unreachable on compact displays.
        let titlebar = self.preferences.titlebar_height() as i32;
        let visible_titlebar = titlebar.min(32);
        let min_y = top - vertical_overflow.min((titlebar - visible_titlebar).max(0));
        let max_y = (bottom - rect.height as i32 + vertical_overflow).max(min_y);
        (x.clamp(min_x, max_x) as i16, y.clamp(min_y, max_y) as i16)
    }

    fn reconstrain_windows(&mut self) {
        for app in AppKind::ALL {
            if self.fullscreen && app == self.active {
                continue;
            }
            let id = self.app_surfaces[app.index()];
            let Some(rect) = self.server.surface(id).map(|surface| surface.current.rect) else {
                continue;
            };
            let (x, y) = self.constrain_window_position(rect, rect.x as i32, rect.y as i32);
            let _ = self.server.set_position(app.owner(), id, x, y);
            let _ = self.server.commit(app.owner(), id);
        }
    }

    fn reset_window_layout(&mut self) {
        self.fullscreen = false;
        for app in AppKind::ALL {
            self.set_app_geometry(app, default_rect(app, self.preferences));
        }
        self.sync_visibility();
        self.full_redraw_requested = true;
        slog!("EXPOS_WINDOW_LAYOUT_RESET count={}\r\n", APP_COUNT);
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
        match Document::parse(url, source) {
            Ok(mut document) => {
                let manifest = ResourceManifest::scan(source).unwrap_or(ResourceManifest::empty());
                if url.starts_with("http://") || url.starts_with("https://") {
                    self.hydrate_browser_document(&mut document, url, manifest);
                } else {
                    self.process_document_web_apis(&mut document, url);
                }
                self.set_document(document);
            }
            Err(error) => self.reject_browser_candidate(error),
        }
    }

    fn browser_fetch(
        &mut self,
        url: &str,
        accept: &str,
    ) -> Result<Box<network::HttpResponse>, network::NetworkError> {
        let handle_id = self
            .browser_network_handle
            .ok_or(network::NetworkError::CapabilityDenied)?;
        let mut origin_buffer = [0_u8; 512];
        let origin = browser_origin(url, &mut origin_buffer).unwrap_or("");
        let mut cookie_buffer = [0_u8; 384];
        let cookie_length = self
            .browser_web_state
            .cookie_header(origin, &mut cookie_buffer);
        let cookie = (cookie_length != 0)
            .then(|| core::str::from_utf8(&cookie_buffer[..cookie_length]).unwrap_or(""));
        let response = Box::new(network::browser_get(
            &self.broker,
            handle_id,
            BROWSER_FIN,
            STABLE_FIN,
            url,
            accept,
            cookie,
        )?);
        if let Some(value) = response.set_cookie() {
            self.store_response_cookie(origin, value);
        }
        Ok(response)
    }

    fn store_response_cookie(&mut self, origin: &str, header: &str) {
        let pair = header.split(';').next().unwrap_or("").trim();
        let Some((name, value)) = pair.split_once('=') else {
            return;
        };
        let secure = header
            .split(';')
            .skip(1)
            .any(|attribute| attribute.trim().eq_ignore_ascii_case("secure"));
        if self
            .browser_web_state
            .set_cookie(origin, name.trim(), value.trim(), secure)
            .is_ok()
        {
            slog!(
                "EXPOS_BROWSER_COOKIE origin_bytes={} secure={}\r\n",
                origin.len(),
                secure
            );
        }
    }

    fn hydrate_browser_document(
        &mut self,
        document: &mut Document,
        base_url: &str,
        manifest: ResourceManifest,
    ) {
        self.browser_media.clear();
        self.browser_resources = BrowserResourceStats {
            discovered: manifest.len() as u16,
            rejected: manifest.rejected() as u16,
            ..BrowserResourceStats::default()
        };
        for resource in manifest.entries() {
            let mut absolute = [0_u8; 512];
            let Some(length) = resolve_browser_link(base_url, resource.url.as_str(), &mut absolute)
            else {
                self.browser_resources.rejected = self.browser_resources.rejected.saturating_add(1);
                continue;
            };
            let url = core::str::from_utf8(&absolute[..length]).unwrap_or("");
            if base_url.starts_with("https://") && url.starts_with("http://") {
                self.browser_resources.rejected = self.browser_resources.rejected.saturating_add(1);
                continue;
            }
            let accept = match resource.kind {
                ExternalResourceKind::Stylesheet => "text/css",
                ExternalResourceKind::Script => "text/javascript,application/javascript",
                ExternalResourceKind::Image => "image/bmp,image/x-portable-pixmap,image/*;q=0.1",
                ExternalResourceKind::Audio => "audio/wav,audio/x-wav,audio/*;q=0.1",
                ExternalResourceKind::Video => "video/*",
            };
            let Ok(response) = self.browser_fetch(url, accept) else {
                self.browser_resources.rejected = self.browser_resources.rejected.saturating_add(1);
                continue;
            };
            if !(200..300).contains(&response.status) || response.truncated {
                self.browser_resources.rejected = self.browser_resources.rejected.saturating_add(1);
                continue;
            }
            let content_type = response.content_type().unwrap_or("");
            let type_matches = content_type.is_empty()
                || match resource.kind {
                    ExternalResourceKind::Stylesheet => content_type.starts_with("text/css"),
                    ExternalResourceKind::Script => content_type.contains("javascript"),
                    ExternalResourceKind::Image => content_type.starts_with("image/"),
                    ExternalResourceKind::Audio => content_type.starts_with("audio/"),
                    ExternalResourceKind::Video => content_type.starts_with("video/"),
                };
            let loaded = type_matches
                && match resource.kind {
                    ExternalResourceKind::Stylesheet => core::str::from_utf8(response.body())
                        .ok()
                        .is_some_and(|css| document.apply_external_stylesheet(css).is_ok()),
                    ExternalResourceKind::Script => core::str::from_utf8(response.body())
                        .ok()
                        .is_some_and(|script| document.execute_script(script).is_ok()),
                    ExternalResourceKind::Image => decode_browser_image(
                        response.body(),
                        resource.url.as_str(),
                        &mut self.browser_media,
                    ),
                    ExternalResourceKind::Audio => {
                        if response.body().starts_with(b"RIFF")
                            && response.body_len <= self.browser_media.audio_bytes.len()
                        {
                            self.browser_media.audio_bytes[..response.body_len]
                                .copy_from_slice(response.body());
                            self.browser_media.audio_len = response.body_len;
                            self.browser_media.audio_url = BrowserText::new(resource.url.as_str())
                                .unwrap_or(BrowserText::empty());
                            true
                        } else {
                            false
                        }
                    }
                    ExternalResourceKind::Video => false,
                };
            if loaded {
                self.browser_resources.loaded = self.browser_resources.loaded.saturating_add(1);
                match resource.kind {
                    ExternalResourceKind::Stylesheet => {
                        self.browser_resources.stylesheets =
                            self.browser_resources.stylesheets.saturating_add(1)
                    }
                    ExternalResourceKind::Script => {
                        self.browser_resources.scripts =
                            self.browser_resources.scripts.saturating_add(1)
                    }
                    ExternalResourceKind::Image => {
                        self.browser_resources.images =
                            self.browser_resources.images.saturating_add(1)
                    }
                    ExternalResourceKind::Audio => {
                        self.browser_resources.audio =
                            self.browser_resources.audio.saturating_add(1)
                    }
                    ExternalResourceKind::Video => {}
                }
            } else {
                self.browser_resources.rejected = self.browser_resources.rejected.saturating_add(1);
            }
        }
        self.process_document_web_apis(document, base_url);
        slog!(
            "EXPOS_BROWSER_RESOURCES discovered={} loaded={} rejected={} css={} js={} images={} audio={} fetches={}\r\n",
            self.browser_resources.discovered,
            self.browser_resources.loaded,
            self.browser_resources.rejected,
            self.browser_resources.stylesheets,
            self.browser_resources.scripts,
            self.browser_resources.images,
            self.browser_resources.audio,
            self.browser_resources.fetches
        );
    }

    fn process_document_web_apis(&mut self, document: &mut Document, base_url: &str) {
        let requests = document.drain_web_api_requests();
        let mut origin_buffer = [0_u8; 512];
        let origin = browser_origin(base_url, &mut origin_buffer).unwrap_or("expos://local");
        for request in requests.iter().flatten().copied() {
            match request {
                WebApiRequest::StorageSet { area, key, value } => {
                    let stored = self.browser_web_state.set_storage(
                        area,
                        origin,
                        key.as_str(),
                        value.as_str(),
                    );
                    if stored.is_ok() && area == expos_core::StorageArea::Local {
                        self.persist_browser_web_state();
                    }
                    if stored.is_ok() {
                        slog!(
                            "EXPOS_BROWSER_STORAGE area={} action=set key_bytes={} value_bytes={}\r\n",
                            if area == expos_core::StorageArea::Local {
                                "local"
                            } else {
                                "session"
                            },
                            key.as_str().len(),
                            value.as_str().len()
                        );
                    }
                }
                WebApiRequest::StorageGet {
                    area,
                    key,
                    target_node,
                } => {
                    let value = self
                        .browser_web_state
                        .storage(area, origin, key.as_str())
                        .unwrap_or("");
                    let _ = document.set_text_at_node(target_node as usize, value);
                    slog!(
                        "EXPOS_BROWSER_STORAGE area={} action=get key_bytes={} hit={}\r\n",
                        if area == expos_core::StorageArea::Local {
                            "local"
                        } else {
                            "session"
                        },
                        key.as_str().len(),
                        !value.is_empty()
                    );
                }
                WebApiRequest::CookieSet { value } => {
                    self.store_response_cookie(origin, value.as_str());
                }
                WebApiRequest::CookieGet { target_node } => {
                    let mut cookies = [0_u8; expos_core::BROWSER_TEXT_CAPACITY];
                    let length = self.browser_web_state.cookie_header(origin, &mut cookies);
                    if let Ok(value) = core::str::from_utf8(&cookies[..length]) {
                        let _ = document.set_text_at_node(target_node as usize, value);
                    }
                }
                WebApiRequest::Fetch { url, target } => {
                    let mut absolute = [0_u8; 512];
                    let Some(length) = resolve_browser_link(base_url, url.as_str(), &mut absolute)
                    else {
                        continue;
                    };
                    let address = core::str::from_utf8(&absolute[..length]).unwrap_or("");
                    if base_url.starts_with("https://") && address.starts_with("http://") {
                        continue;
                    }
                    let Ok(response) = self.browser_fetch(address, "text/plain,application/json")
                    else {
                        continue;
                    };
                    self.browser_resources.fetches =
                        self.browser_resources.fetches.saturating_add(1);
                    if !target.as_str().is_empty() && (200..300).contains(&response.status) {
                        let mut text = [0_u8; expos_core::BROWSER_TEXT_CAPACITY];
                        let length = flatten_browser_text(response.body(), &mut text);
                        if let Ok(value) = core::str::from_utf8(&text[..length]) {
                            let _ = document.set_text_by_id(target.as_str(), value);
                        }
                    }
                }
            }
        }
    }

    fn reject_browser_candidate(&mut self, error: BrowserError) {
        self.document.reject(error);
        slog!(
            "EXPOS_BROWSER_CONTAINER_REJECTED generation={} rejected={} error={:?}\r\n",
            self.document.generation,
            self.document.rejected_loads,
            error
        );
    }

    fn set_document(&mut self, document: Document) {
        let active = self.browser_active_tab;
        if self.browser_history_locked {
            self.browser_history_locked = false;
        } else {
            let url = document.url().as_bytes();
            let tab = &mut self.browser_tabs[active];
            let cursor = tab.history_cursor as usize;
            let count = tab.history_count as usize;
            if cursor + 1 < count {
                tab.history_count = (cursor + 1) as u8;
            }
            if tab.history_count as usize == BROWSER_HISTORY_CAPACITY {
                for index in 1..BROWSER_HISTORY_CAPACITY {
                    tab.history[index - 1] = tab.history[index];
                    tab.history_len[index - 1] = tab.history_len[index];
                }
                tab.history_count -= 1;
                tab.history_cursor = tab.history_cursor.saturating_sub(1);
            }
            let index = tab.history_count as usize;
            let length = url.len().min(tab.history[index].len());
            tab.history[index][..length].copy_from_slice(&url[..length]);
            tab.history_len[index] = length as u16;
            tab.history_count += 1;
            tab.history_cursor = tab.history_count - 1;
        }
        let title = if document.title().is_empty() {
            document.url()
        } else {
            document.title()
        };
        self.browser_tabs[active].set_title(title);
        self.browser_tabs[active].scroll = 0;
        self.document.replace(document);
        self.browser_scroll = 0;
        self.browser_find_match = None;
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
                let (width, height) = app_dimensions(self.preferences);
                Rect::new(0, 0, width, height)
            });
        let _ = self.server.damage(BROWSER_FIN, surface, damage_rect);
        let _ = self.server.commit(BROWSER_FIN, surface);
    }

    fn browser_history_move(&mut self, direction: i8) {
        let tab = &mut self.browser_tabs[self.browser_active_tab];
        let cursor = tab.history_cursor as usize;
        let count = tab.history_count as usize;
        let next = if direction < 0 {
            cursor.saturating_sub(1)
        } else {
            (cursor + 1).min(count.saturating_sub(1))
        };
        if next == cursor {
            return;
        }
        tab.history_cursor = next as u8;
        let length = tab.history_len[next] as usize;
        let mut address = [0_u8; 512];
        address[..length].copy_from_slice(&tab.history[next][..length]);
        self.browser_history_locked = true;
        self.navigate_address(core::str::from_utf8(&address[..length]).unwrap_or("expos://home"));
    }

    fn browser_new_tab(&mut self) {
        if self.browser_tab_count == BROWSER_TAB_CAPACITY {
            return;
        }
        self.browser_tabs[self.browser_active_tab].scroll = self.browser_scroll;
        let index = self.browser_tab_count;
        self.browser_tabs[index] = BrowserTab::home();
        self.browser_tab_count += 1;
        self.browser_active_tab = index;
        self.browser_history_locked = true;
        self.browser_editing = false;
        self.browser_find_editing = false;
        self.browser_find_len = 0;
        self.navigate_address("expos://home");
        slog!(
            "EXPOS_BROWSER_TAB action=new active={} count={}\r\n",
            self.browser_active_tab + 1,
            self.browser_tab_count
        );
    }

    fn browser_switch_tab(&mut self, index: usize) {
        if index >= self.browser_tab_count || index == self.browser_active_tab {
            return;
        }
        self.browser_tabs[self.browser_active_tab].scroll = self.browser_scroll;
        self.browser_active_tab = index;
        let mut address = [0_u8; 512];
        let url = self.browser_tabs[index].url().as_bytes();
        let length = url.len().min(address.len());
        address[..length].copy_from_slice(&url[..length]);
        let scroll = self.browser_tabs[index].scroll;
        self.browser_history_locked = true;
        self.browser_editing = false;
        self.browser_find_editing = false;
        self.browser_find_len = 0;
        self.navigate_address(core::str::from_utf8(&address[..length]).unwrap_or("expos://home"));
        self.browser_scroll = scroll;
        self.browser_tabs[index].scroll = scroll;
        slog!(
            "EXPOS_BROWSER_TAB action=switch active={} count={}\r\n",
            self.browser_active_tab + 1,
            self.browser_tab_count
        );
    }

    fn browser_cycle_tab(&mut self) {
        let next = (self.browser_active_tab + 1) % self.browser_tab_count.max(1);
        self.browser_switch_tab(next);
    }

    fn browser_close_tab(&mut self, index: usize) {
        if index >= self.browser_tab_count {
            return;
        }
        if self.browser_tab_count == 1 {
            self.browser_tabs[0].reset();
            self.browser_history_locked = true;
            self.navigate_address("expos://home");
            return;
        }
        for tab in index + 1..self.browser_tab_count {
            self.browser_tabs[tab - 1] = self.browser_tabs[tab];
        }
        self.browser_tab_count -= 1;
        self.browser_tabs[self.browser_tab_count] = BrowserTab::EMPTY;
        if self.browser_active_tab > index {
            self.browser_active_tab -= 1;
        } else if self.browser_active_tab >= self.browser_tab_count {
            self.browser_active_tab = self.browser_tab_count - 1;
        }
        let active = self.browser_active_tab;
        let mut address = [0_u8; 512];
        let url = self.browser_tabs[active].url().as_bytes();
        let length = url.len().min(address.len());
        address[..length].copy_from_slice(&url[..length]);
        let scroll = self.browser_tabs[active].scroll;
        self.browser_history_locked = true;
        self.navigate_address(core::str::from_utf8(&address[..length]).unwrap_or("expos://home"));
        self.browser_scroll = scroll;
        self.browser_tabs[active].scroll = scroll;
        slog!(
            "EXPOS_BROWSER_TAB action=close active={} count={}\r\n",
            self.browser_active_tab + 1,
            self.browser_tab_count
        );
    }

    fn browser_bookmark_index(&self, url: &str) -> Option<usize> {
        self.browser_bookmarks[..self.browser_bookmark_count]
            .iter()
            .position(|bookmark| bookmark.url() == url)
    }

    fn browser_toggle_bookmark(&mut self) {
        let url = self.document.url();
        if let Some(index) = self.browser_bookmark_index(url) {
            for bookmark in index + 1..self.browser_bookmark_count {
                self.browser_bookmarks[bookmark - 1] = self.browser_bookmarks[bookmark];
            }
            self.browser_bookmark_count -= 1;
            self.browser_bookmarks[self.browser_bookmark_count] = BrowserBookmark::EMPTY;
            slog!(
                "EXPOS_BROWSER_BOOKMARK action=remove count={}\r\n",
                self.browser_bookmark_count
            );
            return;
        }
        if self.browser_bookmark_count == BROWSER_BOOKMARK_CAPACITY {
            for index in 1..BROWSER_BOOKMARK_CAPACITY {
                self.browser_bookmarks[index - 1] = self.browser_bookmarks[index];
            }
            self.browser_bookmark_count -= 1;
        }
        let title = if self.document.title().is_empty() {
            self.document.url()
        } else {
            self.document.title()
        };
        self.browser_bookmarks[self.browser_bookmark_count] = BrowserBookmark::new(url, title);
        self.browser_bookmark_count += 1;
        slog!(
            "EXPOS_BROWSER_BOOKMARK action=add count={}\r\n",
            self.browser_bookmark_count
        );
    }

    fn browser_find_next(&mut self) {
        if self.browser_find_len == 0 {
            self.browser_find_match = None;
            return;
        }
        let query =
            core::str::from_utf8(&self.browser_find_line[..self.browser_find_len]).unwrap_or("");
        let start = self.browser_find_match.map_or(0, |index| index + 1);
        let mut found = self
            .document
            .styled_nodes()
            .find(|styled| {
                styled.index >= start
                    && styled.style.is_rendered()
                    && browser_text_contains(styled.node.text.as_str(), query)
            })
            .map(|styled| styled.index);
        if found.is_none() && start != 0 {
            found = self
                .document
                .styled_nodes()
                .find(|styled| {
                    styled.index < start
                        && styled.style.is_rendered()
                        && browser_text_contains(styled.node.text.as_str(), query)
                })
                .map(|styled| styled.index);
        }
        self.browser_find_match = found;
        let Some(index) = found else {
            return;
        };
        let surface = self.app_surfaces[AppKind::Browser.index()];
        let rect = self
            .server
            .surface(surface)
            .map(|surface| surface.current.rect)
            .unwrap_or_else(|| {
                let (width, height) = app_dimensions(self.preferences);
                Rect::new(0, 0, width, height)
            });
        let mut content_y = 0;
        for styled in self.document.styled_nodes() {
            if styled.node.kind == NodeKind::Title || !styled.style.is_rendered() {
                continue;
            }
            let layout =
                browser_layout(Rect::new(0, 0, rect.width, rect.height), styled, content_y);
            if styled.index == index {
                self.browser_scroll = layout.y.clamp(0, 8_192);
                self.browser_tabs[self.browser_active_tab].scroll = self.browser_scroll;
                break;
            }
            content_y = layout.next_y;
        }
    }

    fn browser_reload(&mut self) {
        let mut address = [0_u8; 512];
        let bytes = self.document.url().as_bytes();
        let length = bytes.len().min(address.len());
        address[..length].copy_from_slice(&bytes[..length]);
        self.browser_history_locked = true;
        self.navigate_address(core::str::from_utf8(&address[..length]).unwrap_or("expos://home"));
    }

    fn log_browser_engine(&self) {
        let report = self.document.script_report();
        slog!(
            "EXPOS_BROWSER_ENGINE nodes={} css_rules={} scripts={} executed={} rejected={} handlers={} container_generation={} container_rejected={}\r\n",
            self.document.len(),
            self.document.style_rule_count(),
            report.scripts_seen,
            report.scripts_executed,
            report.scripts_rejected,
            report.handlers_registered,
            self.document.generation,
            self.document.rejected_loads
        );
    }

    fn navigate_address(&mut self, address: &str) {
        let address = address.trim();
        if address.is_empty() {
            self.navigate("expos://home", HOME);
            return;
        }
        if address.eq_ignore_ascii_case("expos://home") || address == "home" {
            self.navigate("expos://home", HOME);
            return;
        }
        if address.eq_ignore_ascii_case("expos://about") || address == "about" {
            self.navigate("expos://about", ABOUT);
            return;
        }
        if address.eq_ignore_ascii_case("expos://packages") || address == "packages" {
            self.navigate("expos://packages", BROWSER_PACKAGES);
            return;
        }
        if address.eq_ignore_ascii_case("expos://system") || address == "system" {
            self.navigate("expos://system", BROWSER_SYSTEM);
            return;
        }
        if !address.starts_with("http://") && !address.starts_with("https://") {
            let query = address.strip_prefix('?').unwrap_or(address).trim();
            let mut url = [0_u8; 512];
            let Some(length) = encode_search_url(query, &mut url) else {
                self.navigate("expos://search-error", SEARCH_ERROR);
                return;
            };
            let url = core::str::from_utf8(&url[..length]).unwrap_or("");
            slog!(
                "EXPOS_BROWSER_SEARCH query_bytes={} url_bytes={} provider=duckduckgo-html\r\n",
                query.len(),
                length
            );
            self.navigate_address(url);
            return;
        }
        let mut unwrapped = [0_u8; 512];
        if let Some(length) = unwrap_duckduckgo_target(address, &mut unwrapped) {
            let target = core::str::from_utf8(&unwrapped[..length]).unwrap_or("");
            slog!(
                "EXPOS_BROWSER_UNWRAP provider=duckduckgo target_bytes={}\r\n",
                length
            );
            self.navigate_address(target);
            return;
        }
        if !self.network_active() {
            self.navigate("expos://offline", NETWORK_BLOCKED);
            slog!("EXPOS_BROWSER_HTTP_ERROR error=CapabilityDenied\r\n");
            return;
        }
        if self.browser_network_handle.is_none() {
            return;
        }
        let mut current = [0_u8; 512];
        let Some(current_len) = copy_browser_url(&mut current, address.as_bytes()) else {
            self.navigate("expos://error", NETWORK_ERROR);
            slog!("EXPOS_BROWSER_HTTP_ERROR error=BadUrl\r\n");
            return;
        };
        let mut current_len = current_len;
        let mut wikipedia_original = [0_u8; 512];
        let mut wikipedia_original_len = 0;
        let mut summary_url = [0_u8; 512];
        if let Some(summary_len) = wikipedia_summary_url(address, &mut summary_url) {
            wikipedia_original_len = address.len().min(wikipedia_original.len());
            wikipedia_original[..wikipedia_original_len]
                .copy_from_slice(&address.as_bytes()[..wikipedia_original_len]);
            current[..summary_len].copy_from_slice(&summary_url[..summary_len]);
            current_len = summary_len;
            slog!("EXPOS_WIKIPEDIA_READER request_bytes={}\r\n", summary_len);
        }
        for redirect_count in 0..=MAX_BROWSER_REDIRECTS {
            let current_url = core::str::from_utf8(&current[..current_len]).unwrap_or("");
            match self.browser_fetch(
                current_url,
                "text/html,text/plain,application/xhtml+xml,*/*;q=0.1",
            ) {
                Ok(response) => {
                    if is_http_redirect(response.status) {
                        let Some(location) = response.location() else {
                            self.navigate("expos://error", NETWORK_ERROR);
                            slog!("EXPOS_BROWSER_HTTP_ERROR error=RedirectWithoutLocation\r\n");
                            return;
                        };
                        if redirect_count == MAX_BROWSER_REDIRECTS {
                            self.navigate("expos://error", NETWORK_ERROR);
                            slog!("EXPOS_BROWSER_HTTP_ERROR error=TooManyRedirects\r\n");
                            return;
                        }
                        let mut next = [0_u8; 512];
                        let Some(next_len) = resolve_browser_link(current_url, location, &mut next)
                        else {
                            self.navigate("expos://error", NETWORK_ERROR);
                            slog!("EXPOS_BROWSER_HTTP_ERROR error=BadRedirect\r\n");
                            return;
                        };
                        let next_url = core::str::from_utf8(&next[..next_len]).unwrap_or("");
                        if current_url.starts_with("https://") && next_url.starts_with("http://") {
                            self.navigate("expos://error", NETWORK_ERROR);
                            slog!("EXPOS_BROWSER_HTTP_ERROR error=InsecureRedirect\r\n");
                            return;
                        }
                        slog!(
                            "EXPOS_BROWSER_REDIRECT status={} hop={} target_bytes={}\r\n",
                            response.status,
                            redirect_count + 1,
                            next_len
                        );
                        current[..next_len].copy_from_slice(&next[..next_len]);
                        current_len = next_len;
                        continue;
                    }
                    let mut sanitized = Box::new([0_u8; network::HTTP_BODY_CAPACITY]);
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
                    let manifest = if wikipedia_original_len == 0 && projected.is_none() {
                        ResourceManifest::scan(source).ok()
                    } else {
                        None
                    };
                    let document = if wikipedia_original_len != 0 {
                        let original =
                            core::str::from_utf8(&wikipedia_original[..wikipedia_original_len])
                                .unwrap_or("https://en.wikipedia.org/");
                        let mut wiki_source = Box::new([0_u8; 12 * 1024]);
                        wikipedia_document(response.body(), &mut *wiki_source)
                            .and_then(|length| core::str::from_utf8(&wiki_source[..length]).ok())
                            .ok_or(expos_core::BrowserError::InvalidDocument)
                            .and_then(|source| Document::parse(original, source))
                    } else if let Some(results) = projected {
                        Ok(results.document)
                    } else {
                        Document::parse(current_url, source)
                    };
                    match document {
                        Ok(mut document) => {
                            if let Some(manifest) = manifest {
                                self.hydrate_browser_document(&mut document, current_url, manifest);
                            } else {
                                self.process_document_web_apis(&mut document, current_url);
                            }
                            self.set_document(document);
                            if search_results != 0 {
                                slog!("EXPOS_SEARCH_RESULTS count={}\r\n", search_results);
                            }
                            if wikipedia_original_len != 0 {
                                slog!("EXPOS_WIKIPEDIA_READY bytes={}\r\n", response.body_len);
                            }
                            slog!(
                                "EXPOS_BROWSER_HTTP_OK status={} bytes={} peer={}.{}.{}.{}\r\n",
                                response.status,
                                response.body_len,
                                response.peer[0],
                                response.peer[1],
                                response.peer[2],
                                response.peer[3]
                            );
                        }
                        Err(error) => {
                            self.reject_browser_candidate(error);
                            slog!(
                                "EXPOS_BROWSER_HTTP_CONTAINED error={:?} retained_generation={}\r\n",
                                error,
                                self.document.generation
                            );
                        }
                    }
                }
                Err(error) => {
                    self.navigate("expos://error", NETWORK_ERROR);
                    slog!("EXPOS_BROWSER_HTTP_ERROR error={:?}\r\n", error);
                }
            }
            return;
        }
    }

    fn browser_click(&mut self, x: i16, y: i16, rect: Rect) -> bool {
        let local_x = x - rect.x;
        let local_y = y - rect.y;
        if (42..72).contains(&local_y) {
            let tab_width = browser_tab_width(rect, self.browser_tab_count) as i16;
            for index in 0..self.browser_tab_count {
                let left = 18 + index as i16 * tab_width;
                if (left..left + tab_width - 2).contains(&local_x) {
                    if local_x >= left + tab_width - 22 && self.browser_tab_count > 1 {
                        self.browser_close_tab(index);
                    } else {
                        self.browser_switch_tab(index);
                    }
                    return true;
                }
            }
            let new_left = 18 + self.browser_tab_count as i16 * tab_width;
            if (new_left..new_left + 30).contains(&local_x) {
                self.browser_new_tab();
                return true;
            }
        }
        if (78..114).contains(&local_y) {
            if (18..54).contains(&local_x) {
                self.browser_history_move(-1);
                self.browser_editing = false;
                return true;
            }
            if (56..92).contains(&local_x) {
                self.browser_history_move(1);
                self.browser_editing = false;
                return true;
            }
            if (94..130).contains(&local_x) {
                self.browser_reload();
                self.browser_editing = false;
                return true;
            }
            if (132..168).contains(&local_x) {
                self.navigate("expos://home", HOME);
                self.browser_editing = false;
                return true;
            }
            if (176..rect.width as i16 - 100).contains(&local_x) {
                let url = self.document.url().as_bytes();
                self.browser_len = url.len().min(self.browser_line.len());
                self.browser_line[..self.browser_len].copy_from_slice(&url[..self.browser_len]);
                self.browser_editing = true;
                self.browser_find_editing = false;
                return true;
            }
            if (rect.width as i16 - 94..rect.width as i16 - 58).contains(&local_x) {
                self.browser_find_editing = true;
                self.browser_editing = false;
                return true;
            }
            if (rect.width as i16 - 52..rect.width as i16 - 18).contains(&local_x) {
                self.browser_toggle_bookmark();
                return true;
            }
        }

        if self.browser_bookmark_count != 0 {
            let top = browser_bookmark_top();
            if (top..top + 28).contains(&(local_y as i32)) {
                let item_width = browser_bookmark_width(rect, self.browser_bookmark_count);
                for index in 0..self.browser_bookmark_count {
                    let left = 18 + index as i32 * item_width;
                    if (left..left + item_width - 4).contains(&(local_x as i32)) {
                        let mut address = [0_u8; 512];
                        let url = self.browser_bookmarks[index].url().as_bytes();
                        let length = url.len().min(address.len());
                        address[..length].copy_from_slice(&url[..length]);
                        self.navigate_address(
                            core::str::from_utf8(&address[..length]).unwrap_or("expos://home"),
                        );
                        return true;
                    }
                }
            }
        }

        if self.browser_find_len != 0 || self.browser_find_editing {
            let top = browser_find_top(self);
            if (top..top + 32).contains(&(local_y as i32)) {
                if local_x as i32 >= rect.width as i32 - 54 {
                    self.browser_find_line.fill(0);
                    self.browser_find_len = 0;
                    self.browser_find_match = None;
                    self.browser_find_editing = false;
                } else {
                    self.browser_find_editing = true;
                    self.browser_editing = false;
                }
                return true;
            }
        }

        let mut content_y = rect.y as i32 + browser_content_top(self) - self.browser_scroll;
        let mut selected: Option<(usize, BrowserText, bool, bool, NodeKind)> = None;
        for styled in self.document.styled_nodes() {
            if styled.node.kind == NodeKind::Title || !styled.style.is_rendered() {
                continue;
            }
            let layout = browser_layout(rect, styled, content_y);
            content_y = layout.next_y;
            if (layout.x..layout.x + layout.width).contains(&(x as i32))
                && (layout.y..layout.y + layout.height).contains(&(y as i32))
            {
                selected = Some((
                    styled.index,
                    styled.node.target,
                    styled.clickable,
                    styled.node.kind == NodeKind::Link,
                    styled.node.kind,
                ));
                break;
            }
        }
        let Some((index, target, scripted, navigable, kind)) = selected else {
            return false;
        };
        if scripted && self.document.dispatch_click_at_node(index) {
            if self.document.web_api_requests().next().is_some() {
                let current = self.document.url().as_bytes();
                let mut url = [0_u8; 512];
                let length = current.len().min(url.len());
                url[..length].copy_from_slice(&current[..length]);
                // Handler requests are already held by the document and are
                // processed through the same origin/capability bridge.
                let base = core::str::from_utf8(&url[..length]).unwrap_or("expos://home");
                let mut staged = core::mem::replace(
                    &mut self.document,
                    BrowserContainer::new(
                        Document::parse("expos://pending", "<title>Pending</title>")
                            .expect("built-in pending document"),
                    ),
                );
                self.process_document_web_apis(&mut staged.document, base);
                self.document = staged;
            }
            let report = self.document.script_report();
            slog!(
                "EXPOS_BROWSER_EVENT type=click node={} executed={}\r\n",
                index,
                report.statements_executed
            );
            self.log_browser_engine();
            return true;
        }
        if kind == NodeKind::Audio
            && !target.as_str().is_empty()
            && target.as_str() == self.browser_media.audio_url.as_str()
            && self.browser_media.audio_len != 0
        {
            let status = audio::status();
            if status.playing {
                audio::stop();
            } else {
                let _ = audio::play_wave(
                    &self.browser_media.audio_bytes[..self.browser_media.audio_len],
                );
            }
            return true;
        }
        if navigable && !target.as_str().is_empty() {
            let mut address = [0_u8; 512];
            let Some(length) =
                resolve_browser_link(self.document.url(), target.as_str(), &mut address)
            else {
                self.navigate("expos://error", NETWORK_ERROR);
                slog!("EXPOS_BROWSER_HTTP_ERROR error=BadUrl\r\n");
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
        let category_start = settings_category_view_start(rect, self.settings_category.index());
        if (12..sidebar_width).contains(&local_x) && local_y >= category_top {
            let slot = ((local_y - category_top) / category_step) as usize;
            let index = category_start + slot;
            if let Some(category) = SettingsCategory::ALL.get(index).copied() {
                if slot < settings_category_capacity(rect)
                    && local_y < category_top + slot as i16 * category_step + category_step - 4
                {
                    self.select_settings_category(category);
                    return true;
                }
            }
        }

        if local_x >= sidebar_width + 24 && local_x < rect.width as i16 - 20 && local_y >= row_top {
            let content_width = rect.width as i32 - sidebar_width as i32 - 50;
            let row_start =
                settings_row_view_start(self.settings_category, self.settings_row, content_width);
            let slot = ((local_y - row_top) / row_step) as usize;
            let row = row_start + slot;
            if row < self.settings_category.row_count()
                && slot < settings_row_capacity(content_width)
                && local_y < row_top + slot as i16 * row_step + row_height
            {
                self.settings_row = row;
                // The left chevron occupies the first 28 px of every Choice
                // control. Toggles and actions ignore the direction value.
                let choice_left = rect.width as i16 - 196;
                let direction = if (choice_left..choice_left + 28).contains(&local_x) {
                    -1
                } else {
                    1
                };
                self.activate_setting(direction);
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
            b'1'..=b'9' => {
                let index = (key - b'1') as usize;
                if let Some(category) = SettingsCategory::ALL.get(index).copied() {
                    self.select_settings_category(category);
                }
            }
            b'\n' | b' ' | b'+' | b'=' => self.activate_setting(1),
            b'-' => self.activate_setting(-1),
            b',' if self.settings_category == SettingsCategory::Appearance
                && self.settings_row < 4 =>
            {
                self.activate_setting(-16);
            }
            b'.' if self.settings_category == SettingsCategory::Appearance
                && self.settings_row < 4 =>
            {
                self.activate_setting(16);
            }
            _ => return false,
        }
        true
    }

    fn change_network_policy(&mut self) {
        let Some(handle_id) = self.settings_radio_handle else {
            self.settings_notice = "Read-only: DIESE did not grant Configure access.";
            slog!("EXPOS_SETTING_DENIED key=network error=CapabilityDenied\r\n");
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
                    "EXPOS_SETTING_CHANGED key=network value={}\r\n",
                    if enabled { "on" } else { "off" }
                );
            }
            Err(error) => {
                self.settings_notice = error.message();
                slog!("EXPOS_SETTING_DENIED key=network error={:?}\r\n", error);
            }
        }
    }

    fn change_radio_policy(&mut self, kind: radio::RadioKind) {
        let Some(handle_id) = self.settings_radio_handle else {
            self.settings_notice = "Read-only: DIESE did not grant Configure access.";
            slog!("EXPOS_SETTING_DENIED key=radio error=CapabilityDenied\r\n");
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
                    "EXPOS_SETTING_CHANGED key={} value={}\r\n",
                    key,
                    if enabled { "on" } else { "off" }
                );
            }
            Err(error) => {
                self.settings_notice = error.message();
                slog!("EXPOS_SETTING_DENIED key={} error={:?}\r\n", key, error);
            }
        }
    }

    fn apply_customization_profile(&mut self, profile: CustomizationProfile) {
        self.preferences = self.preferences.with_profile(profile);
        framebuffer::set_font_style(framebuffer::FontStyle::new(
            self.preferences.font_face,
            self.preferences.font_weight,
        ));
        self.cursor
            .set_style(self.preferences.cursor, self.preferences.accent.color());
        self.cursor.set_shadow(self.preferences.cursor_shadow);
        self.launcher_scroll = 0;
        self.sync_desktop_geometry();
        self.reconstrain_windows();
        self.reconfigure_presentation();
        self.full_redraw_requested = true;
        self.settings_notice = match profile {
            CustomizationProfile::Balanced => "Balanced desktop profile applied.",
            CustomizationProfile::Compact => "Compact desktop profile applied.",
            CustomizationProfile::Focus => "Distraction-free desktop profile applied.",
            CustomizationProfile::Accessible => "Accessible desktop profile applied.",
            CustomizationProfile::Showcase => "Showcase desktop profile applied.",
            CustomizationProfile::Touch => "Touch-friendly desktop profile applied.",
            CustomizationProfile::Night => "Night desktop profile applied.",
            CustomizationProfile::Presentation => "Presentation desktop profile applied.",
        };
        slog!(
            "EXPOS_SETTING_CHANGED key=profile value={}\r\n",
            profile.label()
        );
    }

    fn activate_setting(&mut self, direction: i8) {
        match (self.settings_category, self.settings_row) {
            (SettingsCategory::System, 0) => {
                self.full_redraw_requested = true;
                self.preferences.taskbar_visible = !self.preferences.taskbar_visible;
                let visible = self.preferences.taskbar_visible;
                self.sync_desktop_geometry();
                self.reconstrain_windows();
                slog!(
                    "EXPOS_SETTING_CHANGED key=taskbar value={}\r\n",
                    if visible { "on" } else { "off" }
                );
                self.settings_notice = "Taskbar visibility updated.";
            }
            (SettingsCategory::System, 1) | (SettingsCategory::Network, 1) => {
                self.full_redraw_requested = true;
                self.preferences.status_visible = !self.preferences.status_visible;
                slog!(
                    "EXPOS_SETTING_CHANGED key=status-indicator value={}\r\n",
                    if self.preferences.status_visible {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Status area visibility updated.";
            }
            (SettingsCategory::Profiles, row) => {
                if let Some(profile) = CustomizationProfile::ALL.get(row).copied() {
                    self.apply_customization_profile(profile);
                }
            }
            (SettingsCategory::Appearance, 0) => {
                self.full_redraw_requested = true;
                self.preferences.theme = self.preferences.theme.shifted(direction);
                slog!(
                    "EXPOS_SETTING_CHANGED key=theme value={}\r\n",
                    self.preferences.theme.label()
                );
                self.settings_notice = "Desktop theme updated.";
            }
            (SettingsCategory::Appearance, 1) => {
                self.full_redraw_requested = true;
                let count = state::WALLPAPER_VARIANT_CHOICES;
                let current = self.preferences.wallpaper_variant as usize;
                let step = direction.unsigned_abs().max(1) as usize % count;
                self.preferences.wallpaper_variant = if direction < 0 {
                    (current + count - step) as u8
                } else {
                    ((current + step) % count) as u8
                };
                self.preferences.wallpaper =
                    WallpaperChoice::from_persisted(self.preferences.wallpaper_variant % 7);
                slog!(
                    "EXPOS_SETTING_CHANGED key=wallpaper value={}\r\n",
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
                    "EXPOS_SETTING_CHANGED key=accent value={}\r\n",
                    self.preferences.accent.label()
                );
                self.settings_notice = "Accent color updated.";
            }
            (SettingsCategory::Appearance, 3) => {
                self.full_redraw_requested = true;
                self.preferences.backdrop = self.preferences.backdrop.shifted(direction);
                slog!(
                    "EXPOS_SETTING_CHANGED key=background-tone value={}\r\n",
                    self.preferences.backdrop.label()
                );
                self.settings_notice = "Wallpaper tone updated.";
            }
            (SettingsCategory::Appearance, 4) => {
                self.full_redraw_requested = true;
                self.preferences.pure_black_apps = !self.preferences.pure_black_apps;
                slog!(
                    "EXPOS_SETTING_CHANGED key=pure-black-apps value={}\r\n",
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
                    "EXPOS_SETTING_CHANGED key=rounded-controls value={}\r\n",
                    if self.preferences.rounded_controls {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Control shape updated.";
            }
            (SettingsCategory::Appearance, 6) => {
                self.full_redraw_requested = true;
                let id = shift_index(
                    self.preferences.font_face.persisted(),
                    state::FONT_FACE_CHOICES,
                    direction,
                );
                self.preferences.font_face = framebuffer::FontFace::from_persisted(id)
                    .unwrap_or(framebuffer::FontFace::System);
                framebuffer::set_font_style(framebuffer::FontStyle::new(
                    self.preferences.font_face,
                    self.preferences.font_weight,
                ));
                slog!(
                    "EXPOS_SETTING_CHANGED key=font-face value={}\r\n",
                    state::FONT_FACE_NAMES[id as usize]
                );
                self.settings_notice = "Interface font face updated.";
            }
            (SettingsCategory::Appearance, 7) => {
                self.full_redraw_requested = true;
                let id = shift_index(
                    self.preferences.font_weight.persisted(),
                    state::FONT_WEIGHT_CHOICES,
                    direction,
                );
                self.preferences.font_weight = framebuffer::FontWeight::from_persisted(id)
                    .unwrap_or(framebuffer::FontWeight::Regular);
                framebuffer::set_font_style(framebuffer::FontStyle::new(
                    self.preferences.font_face,
                    self.preferences.font_weight,
                ));
                slog!(
                    "EXPOS_SETTING_CHANGED key=font-weight value={}\r\n",
                    state::FONT_WEIGHT_NAMES[id as usize]
                );
                self.settings_notice = "Interface font weight updated.";
            }
            (SettingsCategory::Accessibility, 0) => {
                self.preferences.high_contrast = !self.preferences.high_contrast;
                self.full_redraw_requested = true;
                self.settings_notice = "Contrast rendering updated.";
            }
            (SettingsCategory::Accessibility, 1) => {
                let id = shift_index(
                    self.preferences.font_face.persisted(),
                    state::FONT_FACE_CHOICES,
                    direction,
                );
                self.preferences.font_face = framebuffer::FontFace::from_persisted(id)
                    .unwrap_or(framebuffer::FontFace::System);
                framebuffer::set_font_style(framebuffer::FontStyle::new(
                    self.preferences.font_face,
                    self.preferences.font_weight,
                ));
                self.full_redraw_requested = true;
                self.settings_notice = "Readable font face updated.";
            }
            (SettingsCategory::Accessibility, 2) => {
                let id = shift_index(
                    self.preferences.font_weight.persisted(),
                    state::FONT_WEIGHT_CHOICES,
                    direction,
                );
                self.preferences.font_weight = framebuffer::FontWeight::from_persisted(id)
                    .unwrap_or(framebuffer::FontWeight::Regular);
                framebuffer::set_font_style(framebuffer::FontStyle::new(
                    self.preferences.font_face,
                    self.preferences.font_weight,
                ));
                self.full_redraw_requested = true;
                self.settings_notice = "Readable font weight updated.";
            }
            (SettingsCategory::Accessibility, 3) => {
                self.preferences.titlebar_density = shift_index(
                    self.preferences.titlebar_density,
                    state::TITLEBAR_DENSITY_CHOICES,
                    direction,
                );
                self.reconstrain_windows();
                self.full_redraw_requested = true;
                self.settings_notice = "Window drag-target size updated.";
            }
            (SettingsCategory::Accessibility, 4) => {
                self.preferences.taskbar_size = shift_index(
                    self.preferences.taskbar_size,
                    state::TASKBAR_SIZE_CHOICES,
                    direction,
                );
                self.sync_desktop_geometry();
                self.reconstrain_windows();
                self.settings_notice = "Taskbar target size updated.";
            }
            (SettingsCategory::Accessibility, 5) => {
                self.preferences.cursor = self.preferences.cursor.shifted(direction);
                self.cursor
                    .set_style(self.preferences.cursor, self.preferences.accent.color());
                self.settings_notice = "Pointer shape updated.";
            }
            (SettingsCategory::Accessibility, 6) => {
                self.preferences.cursor_shadow = !self.preferences.cursor_shadow;
                self.cursor.set_shadow(self.preferences.cursor_shadow);
                self.full_redraw_requested = true;
                self.settings_notice = "Pointer separation updated.";
            }
            (SettingsCategory::Accessibility, 7) => {
                self.preferences.animation_level = shift_index(
                    self.preferences.animation_level,
                    state::ANIMATION_LEVEL_CHOICES,
                    direction,
                );
                self.settings_notice = "Motion level updated.";
            }
            (SettingsCategory::Accessibility, 8) => {
                self.preferences.tooltips = !self.preferences.tooltips;
                self.settings_notice = "Interface hints updated.";
            }
            (SettingsCategory::Accessibility, 9) => {
                self.preferences.reduce_transparency = !self.preferences.reduce_transparency;
                self.full_redraw_requested = true;
                self.settings_notice = "Transparency accessibility updated.";
            }
            (SettingsCategory::Accessibility, 10) => {
                self.preferences.focus_ring = !self.preferences.focus_ring;
                self.full_redraw_requested = true;
                self.settings_notice = "Keyboard focus ring updated.";
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
                    "EXPOS_SETTING_CHANGED key=resolution value={}\r\n",
                    selected.label()
                );
                self.settings_notice = "Resolution applies when the desktop is reopened.";
            }
            (SettingsCategory::Display, 1) => {
                self.preferences.refresh_rate =
                    shift_refresh_rate(self.preferences.refresh_rate, direction);
                self.reconfigure_presentation();
                slog!(
                    "EXPOS_SETTING_CHANGED key=refresh-rate value={}\r\n",
                    refresh_rate_label(self.preferences.refresh_rate)
                );
                self.settings_notice = "Compositor presentation rate updated.";
            }
            (SettingsCategory::Display, 2) => {
                self.preferences.vsync = !self.preferences.vsync;
                self.reconfigure_presentation();
                slog!(
                    "EXPOS_SETTING_CHANGED key=vsync value={}\r\n",
                    if self.preferences.vsync { "on" } else { "off" }
                );
                self.settings_notice = "Page-flip synchronization updated.";
            }
            (SettingsCategory::Display, 4) => {
                self.full_redraw_requested = true;
                self.preferences.window_borders = !self.preferences.window_borders;
                if self.preferences.window_borders && self.preferences.window_border_width == 0 {
                    self.preferences.window_border_width = 1;
                }
                slog!(
                    "EXPOS_SETTING_CHANGED key=window-borders value={}\r\n",
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
                    "EXPOS_SETTING_CHANGED key=high-contrast value={}\r\n",
                    if self.preferences.high_contrast {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Contrast rendering updated.";
            }
            (SettingsCategory::Audio, 0) => {
                let step = if direction < 0 { -5 } else { 5 };
                self.preferences.audio_volume = self
                    .preferences
                    .audio_volume
                    .saturating_add_signed(step)
                    .min(100);
                audio::set_volume(self.preferences.audio_volume);
                self.settings_notice = "Output volume updated.";
                slog!(
                    "EXPOS_SETTING_CHANGED key=audio-volume value={}\r\n",
                    self.preferences.audio_volume
                );
            }
            (SettingsCategory::Audio, 1) => {
                self.preferences.audio_muted = !self.preferences.audio_muted;
                audio::set_muted(self.preferences.audio_muted);
                self.settings_notice = "Audio mute updated.";
                slog!(
                    "EXPOS_SETTING_CHANGED key=audio-muted value={}\r\n",
                    self.preferences.audio_muted
                );
            }
            (SettingsCategory::Audio, 2) => {
                self.settings_notice = if audio::play_tone(660, 120).is_ok() {
                    "Playing a bounded PCM test tone."
                } else {
                    "No supported audio output is active."
                };
                slog!("EXPOS_SETTING_CHANGED key=audio-test action=play\r\n");
            }
            (SettingsCategory::Audio, 3) => {
                audio::stop();
                self.settings_notice = "Audio playback stopped.";
                slog!("EXPOS_SETTING_CHANGED key=audio-test action=stop\r\n");
            }
            (SettingsCategory::Performance, 0) => {
                self.full_redraw_requested = true;
                self.preferences.window_shadows = !self.preferences.window_shadows;
                slog!(
                    "EXPOS_SETTING_CHANGED key=window-shadows value={}\r\n",
                    if self.preferences.window_shadows {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Window shadow rendering updated.";
            }
            (SettingsCategory::Performance, 1) => {
                self.full_redraw_requested = true;
                self.preferences.wallpaper_effects = !self.preferences.wallpaper_effects;
                slog!(
                    "EXPOS_SETTING_CHANGED key=wallpaper-effects value={}\r\n",
                    if self.preferences.wallpaper_effects {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Procedural wallpaper rendering updated.";
            }
            (SettingsCategory::Performance, 2) => {
                self.preferences.responsive_presentation =
                    !self.preferences.responsive_presentation;
                self.frame_pacer.reset_phase(crate::hardware::timestamp());
                slog!(
                    "EXPOS_SETTING_CHANGED key=presentation-policy value={}\r\n",
                    self.preferences.presentation_policy_label()
                );
                self.settings_notice = if self.preferences.responsive_presentation {
                    "Damaged commits now bypass software pacing."
                } else {
                    "All commits now follow the efficient software cadence."
                };
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
                    "EXPOS_SETTING_CHANGED key=pointer-speed value={}\r\n",
                    self.preferences.pointer_speed_label()
                );
                self.settings_notice = "Pointer speed updated.";
            }
            (SettingsCategory::Input, 1) => {
                self.preferences.cursor = self.preferences.cursor.shifted(direction);
                self.cursor
                    .set_style(self.preferences.cursor, self.preferences.accent.color());
                slog!(
                    "EXPOS_SETTING_CHANGED key=cursor-theme value={}\r\n",
                    self.preferences.cursor.label()
                );
                self.settings_notice = "Cursor theme updated.";
            }
            (SettingsCategory::Input, 4) => {
                self.preferences.cursor_shadow = !self.preferences.cursor_shadow;
                self.cursor.set_shadow(self.preferences.cursor_shadow);
                self.full_redraw_requested = true;
                slog!(
                    "EXPOS_SETTING_CHANGED key=cursor-shadow value={}\r\n",
                    if self.preferences.cursor_shadow {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Cursor shadow updated.";
            }
            (SettingsCategory::Windows, 0) => {
                self.preferences.window_corner_radius = shift_index(
                    self.preferences.window_corner_radius,
                    state::WINDOW_CORNER_RADIUS_CHOICES,
                    direction,
                );
                self.full_redraw_requested = true;
                slog!(
                    "EXPOS_SETTING_CHANGED key=window-radius value={}\r\n",
                    CORNER_RADIUS_LABELS[self.preferences.window_corner_radius as usize]
                );
                self.settings_notice = "Window corner geometry updated.";
            }
            (SettingsCategory::Windows, 1) => {
                self.preferences.window_border_width = shift_index(
                    self.preferences.window_border_width,
                    state::WINDOW_BORDER_WIDTH_CHOICES,
                    direction,
                );
                self.preferences.window_borders = self.preferences.window_border_width != 0;
                self.full_redraw_requested = true;
                slog!(
                    "EXPOS_SETTING_CHANGED key=window-border-width value={}\r\n",
                    BORDER_WIDTH_LABELS[self.preferences.window_border_width as usize]
                );
                self.settings_notice = "Window border width updated.";
            }
            (SettingsCategory::Windows, 2) => {
                self.preferences.titlebar_density = shift_index(
                    self.preferences.titlebar_density,
                    state::TITLEBAR_DENSITY_CHOICES,
                    direction,
                );
                self.reconstrain_windows();
                self.full_redraw_requested = true;
                slog!(
                    "EXPOS_SETTING_CHANGED key=titlebar-size value={}\r\n",
                    TITLEBAR_LABELS[self.preferences.titlebar_density as usize]
                );
                self.settings_notice = "Titlebar density updated.";
            }
            (SettingsCategory::Windows, 3) => {
                self.preferences.window_opacity = shift_index(
                    self.preferences.window_opacity,
                    state::WINDOW_OPACITY_CHOICES,
                    direction,
                );
                self.full_redraw_requested = true;
                slog!(
                    "EXPOS_SETTING_CHANGED key=window-opacity value={}\r\n",
                    WINDOW_OPACITY_LABELS[self.preferences.window_opacity as usize]
                );
                self.settings_notice = "Window translucency updated.";
            }
            (SettingsCategory::Windows, 4) => {
                self.preferences.window_offscreen_allowance = shift_index(
                    self.preferences.window_offscreen_allowance,
                    state::WINDOW_OFFSCREEN_ALLOWANCE_CHOICES,
                    direction,
                );
                self.reconstrain_windows();
                self.full_redraw_requested = true;
                slog!(
                    "EXPOS_SETTING_CHANGED key=offscreen-allowance value={}\r\n",
                    OFFSCREEN_LABELS[self.preferences.window_offscreen_allowance as usize]
                );
                self.settings_notice = "Window edge travel updated.";
            }
            (SettingsCategory::Windows, 5) => {
                self.preferences.window_snap = !self.preferences.window_snap;
                slog!(
                    "EXPOS_SETTING_CHANGED key=window-snap value={}\r\n",
                    if self.preferences.window_snap {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Window edge snapping updated.";
            }
            (SettingsCategory::Windows, 6) => {
                self.preferences.window_snap_distance = shift_index(
                    self.preferences.window_snap_distance,
                    state::WINDOW_SNAP_DISTANCE_CHOICES,
                    direction,
                );
                slog!(
                    "EXPOS_SETTING_CHANGED key=snap-distance value={}\r\n",
                    SNAP_DISTANCE_LABELS[self.preferences.window_snap_distance as usize]
                );
                self.settings_notice = "Window snap distance updated.";
            }
            (SettingsCategory::Windows, 7) => {
                self.preferences.focus_policy = shift_index(
                    self.preferences.focus_policy,
                    state::FOCUS_POLICY_CHOICES,
                    direction,
                );
                slog!(
                    "EXPOS_SETTING_CHANGED key=focus-policy value={}\r\n",
                    self.preferences.focus_policy_label()
                );
                self.settings_notice = "Window focus policy updated.";
            }
            (SettingsCategory::Windows, 8) => {
                self.preferences.window_tiling = shift_index(
                    self.preferences.window_tiling,
                    WINDOW_TILING_LABELS.len() as u8,
                    direction,
                );
                self.settings_notice = "Keyboard tiling layout updated.";
            }
            (SettingsCategory::Taskbar, 0) => {
                self.preferences.taskbar_placement =
                    self.preferences.taskbar_placement.shifted(direction);
                self.sync_desktop_geometry();
                self.reconstrain_windows();
                slog!(
                    "EXPOS_SETTING_CHANGED key=taskbar-placement value={}\r\n",
                    self.preferences.taskbar_placement.label()
                );
                self.settings_notice = "Taskbar edge updated.";
            }
            (SettingsCategory::Taskbar, 1) => {
                self.preferences.taskbar_size = shift_index(
                    self.preferences.taskbar_size,
                    state::TASKBAR_SIZE_CHOICES,
                    direction,
                );
                self.sync_desktop_geometry();
                self.reconstrain_windows();
                slog!(
                    "EXPOS_SETTING_CHANGED key=taskbar-size value={}\r\n",
                    TASKBAR_SIZE_LABELS[self.preferences.taskbar_size as usize]
                );
                self.settings_notice = "Taskbar size updated.";
            }
            (SettingsCategory::Taskbar, 2) => {
                self.preferences.taskbar_alignment =
                    self.preferences.taskbar_alignment.shifted(direction);
                self.full_redraw_requested = true;
                slog!(
                    "EXPOS_SETTING_CHANGED key=taskbar-alignment value={}\r\n",
                    self.preferences.taskbar_alignment.label()
                );
                self.settings_notice = "Running-app alignment updated.";
            }
            (SettingsCategory::Taskbar, 3) => {
                self.preferences.taskbar_autohide = !self.preferences.taskbar_autohide;
                self.sync_desktop_geometry();
                self.reconstrain_windows();
                slog!(
                    "EXPOS_SETTING_CHANGED key=taskbar-autohide value={}\r\n",
                    if self.preferences.taskbar_autohide {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Taskbar auto-hide updated; touch its edge to reveal.";
            }
            (SettingsCategory::Taskbar, 4) => {
                self.preferences.taskbar_translucent = !self.preferences.taskbar_translucent;
                self.full_redraw_requested = true;
                slog!(
                    "EXPOS_SETTING_CHANGED key=taskbar-translucent value={}\r\n",
                    if self.preferences.taskbar_translucent {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Taskbar translucency updated.";
            }
            (SettingsCategory::Taskbar, 5) => {
                self.preferences.taskbar_labels = !self.preferences.taskbar_labels;
                self.full_redraw_requested = true;
                slog!(
                    "EXPOS_SETTING_CHANGED key=taskbar-labels value={}\r\n",
                    if self.preferences.taskbar_labels {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Taskbar application labels updated.";
            }
            (SettingsCategory::Taskbar, 6) => {
                self.preferences.clock_seconds = !self.preferences.clock_seconds;
                self.full_redraw_requested = true;
                slog!(
                    "EXPOS_SETTING_CHANGED key=clock-seconds value={}\r\n",
                    if self.preferences.clock_seconds {
                        "on"
                    } else {
                        "off"
                    }
                );
                self.settings_notice = "Taskbar clock precision updated.";
            }
            (SettingsCategory::Taskbar, row @ 7..=12) => {
                if self.preferences.taskbar_placement.vertical() {
                    self.settings_notice = "Small widgets are shown on horizontal taskbars.";
                    self.save_preferences();
                    return;
                }
                let flag = match row {
                    7 => TASKBAR_WIDGET_DATE,
                    8 => TASKBAR_WIDGET_ACTIVE_APP,
                    9 => TASKBAR_WIDGET_WEATHER,
                    10 => TASKBAR_WIDGET_PERFORMANCE,
                    11 => TASKBAR_WIDGET_AUDIO,
                    _ => TASKBAR_WIDGET_TIMEZONE,
                };
                self.preferences.taskbar_widgets ^= flag;
                self.full_redraw_requested = true;
                self.settings_notice = "Taskbar widget visibility updated.";
                slog!(
                    "EXPOS_SETTING_CHANGED key=taskbar-widget-{} value={}\r\n",
                    row - 7,
                    if self.preferences.taskbar_widgets & flag != 0 {
                        "on"
                    } else {
                        "off"
                    }
                );
            }
            (SettingsCategory::Menu, 0) => {
                self.preferences.menu_layout = shift_index(
                    self.preferences.menu_layout,
                    MENU_LAYOUT_LABELS.len() as u8,
                    direction,
                );
                self.preferences.menu_grid = self.preferences.menu_layout != 0;
                self.launcher_scroll = 0;
                self.sync_desktop_geometry();
                self.settings_notice = "Launcher layout updated.";
                slog!(
                    "EXPOS_SETTING_CHANGED key=menu-layout value={}\r\n",
                    MENU_LAYOUT_LABELS[self.preferences.menu_layout as usize]
                );
            }
            (SettingsCategory::Menu, 1) => {
                self.preferences.menu_density = shift_index(
                    self.preferences.menu_density,
                    state::MENU_DENSITY_CHOICES,
                    direction,
                );
                self.launcher_scroll = 0;
                self.full_redraw_requested = true;
                self.settings_notice = "Launcher row density updated.";
            }
            (SettingsCategory::Menu, 2) => {
                self.preferences.ui_scale = shift_index(
                    self.preferences.ui_scale,
                    state::UI_SCALE_CHOICES,
                    direction,
                );
                self.full_redraw_requested = true;
                self.settings_notice = "Launcher icon scale updated.";
            }
            (SettingsCategory::Menu, 3) => {
                self.preferences.menu_show_builtins = !self.preferences.menu_show_builtins;
                self.launcher_scroll = 0;
                self.settings_notice = "Built-in launcher entries updated.";
            }
            (SettingsCategory::Menu, 4) => {
                self.preferences.menu_show_installed = !self.preferences.menu_show_installed;
                self.launcher_scroll = 0;
                self.settings_notice = "Installed Ayo launcher entries updated.";
            }
            (SettingsCategory::Menu, 5) => {
                self.preferences.menu_categories = !self.preferences.menu_categories;
                self.full_redraw_requested = true;
                self.settings_notice = "Launcher category labels updated.";
            }
            (SettingsCategory::Menu, 6) => {
                self.preferences.animation_level = shift_index(
                    self.preferences.animation_level,
                    state::ANIMATION_LEVEL_CHOICES,
                    direction,
                );
                self.settings_notice = "Menu animation timing updated.";
            }
            (SettingsCategory::Menu, 7) => {
                self.preferences.tooltips = !self.preferences.tooltips;
                self.settings_notice = "Launcher tooltips updated.";
            }
            (SettingsCategory::Menu, 8) => {
                self.preferences.notification_animations =
                    !self.preferences.notification_animations;
                self.settings_notice = "Menu install feedback updated.";
            }
            (SettingsCategory::Terminal, 0) => {
                let next = shift_index(
                    self.preferences.terminal_font_face.persisted(),
                    state::FONT_FACE_CHOICES,
                    direction,
                );
                self.preferences.terminal_font_face =
                    framebuffer::FontFace::from_persisted(next).unwrap_or_default();
                self.settings_notice = "Shell font updated.";
            }
            (SettingsCategory::Terminal, 1) => {
                let next = shift_index(
                    self.preferences.terminal_font_weight.persisted(),
                    state::FONT_WEIGHT_CHOICES,
                    direction,
                );
                self.preferences.terminal_font_weight =
                    framebuffer::FontWeight::from_persisted(next).unwrap_or_default();
                self.settings_notice = "Shell font weight updated.";
            }
            (SettingsCategory::Terminal, 2) => {
                self.preferences.terminal_scale =
                    shift_index(self.preferences.terminal_scale, 3, direction);
                self.settings_notice = "Shell font size updated.";
            }
            (SettingsCategory::Terminal, 3) => {
                self.preferences.terminal_foreground =
                    shift_index(self.preferences.terminal_foreground, 8, direction);
                self.settings_notice = "Shell text color updated.";
            }
            (SettingsCategory::Terminal, 4) => {
                self.preferences.terminal_background =
                    shift_index(self.preferences.terminal_background, 8, direction);
                self.settings_notice = "Shell background updated.";
            }
            (SettingsCategory::Language, row @ 0..=4) => {
                self.preferences.locale = row as u8;
                crate::locale::set_active(Locale::from_persisted(row as u8));
                self.full_redraw_requested = true;
                self.settings_notice = "System language updated.";
                slog!(
                    "EXPOS_SETTING_CHANGED key=language value={}\r\n",
                    Locale::ALL[row].name()
                );
            }
            (SettingsCategory::Time, 0) => {
                self.preferences.timezone = shift_index(
                    self.preferences.timezone,
                    TIMEZONE_LABELS.len() as u8,
                    direction,
                );
                self.full_redraw_requested = true;
                self.settings_notice = "Time zone and regional clock updated.";
                slog!(
                    "EXPOS_SETTING_CHANGED key=timezone value={}\r\n",
                    TIMEZONE_LABELS[self.preferences.timezone as usize]
                );
            }
            (SettingsCategory::Time, 1) => {
                self.preferences.clock_24h = !self.preferences.clock_24h;
                self.full_redraw_requested = true;
                self.settings_notice = "Clock format updated across the taskbar.";
            }
            (SettingsCategory::Time, 2) => {
                self.preferences.clock_offset_quarters = if direction < 0 {
                    self.preferences
                        .clock_offset_quarters
                        .saturating_sub(1)
                        .max(-48)
                } else {
                    self.preferences
                        .clock_offset_quarters
                        .saturating_add(1)
                        .min(48)
                };
                self.full_redraw_requested = true;
                self.settings_notice = "Manual displayed-time correction updated by 15 minutes.";
            }
            (SettingsCategory::Time, 3) => {
                self.preferences.date_format = shift_index(
                    self.preferences.date_format,
                    DATE_FORMAT_LABELS.len() as u8,
                    direction,
                );
                self.full_redraw_requested = true;
                self.settings_notice = "Regional date format updated.";
            }
            (SettingsCategory::Time, 4) => {
                self.preferences.week_starts_monday = !self.preferences.week_starts_monday;
                self.settings_notice = "Calendar week start updated.";
            }
            (SettingsCategory::Time, 5) => {
                self.preferences.clock_offset_quarters = 0;
                self.full_redraw_requested = true;
                self.settings_notice = "Manual clock correction reset to zero.";
            }
            (SettingsCategory::Privacy, 0) => {
                self.browser_line.fill(0);
                self.browser_len = 0;
                self.browser_editing = false;
                self.browser_find_line.fill(0);
                self.browser_find_len = 0;
                self.browser_find_editing = false;
                self.browser_find_match = None;
                self.browser_tabs = [BrowserTab::EMPTY; BROWSER_TAB_CAPACITY];
                self.browser_tabs[0] = BrowserTab::home();
                self.browser_tab_count = 1;
                self.browser_active_tab = 0;
                self.browser_bookmarks = [BrowserBookmark::EMPTY; BROWSER_BOOKMARK_CAPACITY];
                self.browser_bookmark_count = 0;
                self.browser_web_state = BrowserWebState::new();
                self.browser_resources = BrowserResourceStats::default();
                self.browser_media.clear();
                audio::stop();
                self.persist_browser_web_state();
                self.browser_history_locked = true;
                self.navigate("expos://home", HOME);
                slog!("EXPOS_SETTING_CHANGED key=browser-data value=cleared\r\n");
                self.settings_notice = "Browser session data cleared.";
            }
            (SettingsCategory::Privacy, 1) => {
                self.native_apps.clear_weather_data();
                self.persist_native_apps();
                slog!("EXPOS_SETTING_CHANGED key=weather-data value=cleared\r\n");
                self.settings_notice = "Saved weather location and forecast cleared.";
            }
            _ => {}
        }
        self.save_preferences();
    }

    fn handle_browser_key(&mut self, key: u8) -> bool {
        if self.browser_find_editing {
            match key {
                b'\n' => self.browser_find_next(),
                0x08 => {
                    self.browser_find_len = self.browser_find_len.saturating_sub(1);
                    self.browser_find_match = None;
                    self.browser_find_next();
                }
                byte if (byte.is_ascii_graphic() || byte == b' ')
                    && self.browser_find_len < self.browser_find_line.len() =>
                {
                    self.browser_find_line[self.browser_find_len] = byte;
                    self.browser_find_len += 1;
                    self.browser_find_match = None;
                    self.browser_find_next();
                }
                _ => return false,
            }
            return true;
        }
        if !self.browser_editing {
            let step =
                state::SCROLL_STEPS[self.preferences.scroll_speed.min(6) as usize] as i32 * 18;
            match key {
                KEY_UP => self.browser_scroll = self.browser_scroll.saturating_sub(step).max(0),
                KEY_DOWN => self.browser_scroll = (self.browser_scroll + step).min(8_192),
                b' ' => self.browser_scroll = (self.browser_scroll + 280).min(8_192),
                b'h' => self.browser_scroll = 0,
                b'[' => self.browser_history_move(-1),
                b']' => self.browser_history_move(1),
                b'r' => self.browser_reload(),
                b'n' => self.browser_new_tab(),
                b'x' => self.browser_close_tab(self.browser_active_tab),
                b'\t' => self.browser_cycle_tab(),
                b'b' => self.browser_toggle_bookmark(),
                b'f' => {
                    self.browser_find_editing = true;
                    self.browser_find_match = None;
                }
                b'/' | b'l' => {
                    self.browser_len = 0;
                    self.browser_editing = true;
                    self.browser_find_editing = false;
                }
                _ => return false,
            }
            self.browser_tabs[self.browser_active_tab].scroll = self.browser_scroll;
            return true;
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
        if self.notes_editing_name {
            match key {
                b'\n' => self.notes_editing_name = false,
                0x08 => self.notes_name_len = self.notes_name_len.saturating_sub(1),
                byte if (byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
                    && self.notes_name_len < self.notes_name.len() =>
                {
                    self.notes_name[self.notes_name_len] = byte;
                    self.notes_name_len += 1;
                }
                _ => return false,
            }
            self.notes_status = "Filename changed; save to commit";
            return true;
        }
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
        self.notes_status = "Unsaved changes";
        true
    }

    fn notes_click(&mut self, x: i16, y: i16, rect: Rect) -> bool {
        let local_x = x - rect.x;
        let local_y = y - rect.y;
        if !(48..86).contains(&local_y) {
            self.notes_editing_name = false;
            return false;
        }
        if (18..rect.width as i16 - 220).contains(&local_x) {
            self.notes_editing_name = true;
            return true;
        }
        if (rect.width as i16 - 204..rect.width as i16 - 112).contains(&local_x) {
            self.notes.fill(0);
            self.notes_len = 0;
            self.notes_name.fill(0);
            self.notes_name[..12].copy_from_slice(b"Untitled.txt");
            self.notes_name_len = 12;
            self.notes_editing_name = false;
            self.notes_status = "New unsaved Data Form";
            return true;
        }
        if (rect.width as i16 - 100..rect.width as i16 - 18).contains(&local_x) {
            self.save_note();
            return true;
        }
        false
    }

    fn save_note(&mut self) {
        if !self.notes_name[..self.notes_name_len].ends_with(b".txt") {
            if self.notes_name_len + 4 > self.notes_name.len() {
                self.notes_status = "Filename must end in .txt";
                return;
            }
            self.notes_name[self.notes_name_len..self.notes_name_len + 4].copy_from_slice(b".txt");
            self.notes_name_len += 4;
        }
        let name = core::str::from_utf8(&self.notes_name[..self.notes_name_len]).unwrap_or("");
        match crate::expfs_store::save_text_form(
            self.broker.cfc(),
            name,
            &self.notes[..self.notes_len],
        ) {
            Ok((_fin, revision)) => {
                self.notes_status = "Saved atomically to ExpFS";
                slog!(
                    "EXPOS_NOTES_SAVED name={} revision={} bytes={}\r\n",
                    name,
                    revision,
                    self.notes_len
                );
            }
            Err(error) => {
                self.notes_status = error.message();
                slog!("EXPOS_NOTES_SAVE_ERROR error={:?}\r\n", error);
            }
        }
    }

    fn persist_native_apps(&mut self) {
        let mut state = [0_u8; crate::apps::NativeApps::STATE_CAPACITY];
        let length = self.native_apps.encode_state(&mut state);
        match crate::expfs_store::save_data_form(
            self.broker.cfc(),
            "AyoApps.state",
            &state[..length],
        ) {
            Ok((_fin, revision)) => {
                slog!(
                    "EXPOS_APPS_STATE_SAVED revision={} bytes={}\r\n",
                    revision,
                    length
                );
            }
            Err(error) => slog!("EXPOS_APPS_STATE_ERROR error={:?}\r\n", error),
        }
    }

    fn complete_manager_action(&mut self, action: crate::apps::ManagerAction) {
        match action {
            crate::apps::ManagerAction::Changed => {}
            crate::apps::ManagerAction::Open => self.switch_to(AppKind::Apps),
            crate::apps::ManagerAction::InstallRequested => {
                let package = self.native_apps.selected_index();
                self.transact_native_package(package, true);
            }
        }
    }

    fn transact_native_package(&mut self, package: usize, install: bool) -> bool {
        let Some(handle_id) = self.ayo_transaction_handle else {
            slog!("EXPOS_ABI_CALL call=PACKAGE_TRANSACTION status=denied reason=no-handle\r\n");
            return false;
        };
        let operation = if install { 1 } else { 2 };
        let request = AbiRequest {
            version: FORM_ABI_VERSION,
            call: AbiCall::PackageTransaction as u16,
            caller: PACKAGES_FIN,
            handle_id,
            arguments: [operation, package as u64, 0, 0, 0, 0],
        };
        let response = NativeCallGate::new(&self.broker, self.broker.cfc(), AYO_FIN, STABLE_FIN)
            .dispatch(request, crate::hardware::timestamp(), |call, arguments| {
                if call != AbiCall::PackageTransaction
                    || !matches!(arguments[0], 1 | 2)
                    || arguments[1] >= crate::apps::PACKAGE_COUNT as u64
                {
                    AbiResponse::status(AbiStatus::Unsupported)
                } else {
                    AbiResponse::ok([arguments[1], arguments[0], 0, 0])
                }
            });
        if AbiStatus::from_raw(response.status) != Some(AbiStatus::Ok) {
            slog!(
                "EXPOS_ABI_CALL call=PACKAGE_TRANSACTION status={} package={} handle={}\r\n",
                response.status,
                package,
                handle_id
            );
            return false;
        }
        if install {
            self.native_apps.select_package(package);
            self.native_apps.commit_install_selected();
        } else {
            self.native_apps.uninstall(package);
        }
        self.persist_native_apps();
        slog!(
            "EXPOS_ABI_CALL call=PACKAGE_TRANSACTION status=ok operation={} package={} name={} handle={}\r\n",
            operation,
            package,
            crate::apps::package_name(package).unwrap_or("invalid"),
            handle_id
        );
        true
    }

    fn complete_native_app_action(&mut self, action: crate::apps::AppAction) {
        match action {
            crate::apps::AppAction::Changed => {}
            crate::apps::AppAction::WeatherRefresh => self.refresh_weather(),
        }
    }

    fn refresh_weather(&mut self) {
        render_active_window(self);
        if !radio::snapshot().network_enabled {
            self.native_apps
                .weather_failed("Enable Network in Settings, then press Enter again.");
            slog!("EXPOS_WEATHER_ERROR error=PolicyDisabled\r\n");
            return;
        }
        let Some(handle_id) = self.apps_network_handle else {
            self.native_apps
                .weather_failed("Weather did not receive a Network Handle.");
            slog!("EXPOS_WEATHER_ERROR error=CapabilityDenied\r\n");
            return;
        };
        let now = crate::hardware::timestamp();
        let cooldown = crate::hardware::clock_info()
            .tsc_hz
            .max(1)
            .saturating_mul(WEATHER_REQUEST_COOLDOWN_SECONDS);
        if self.last_weather_request_ticks != 0
            && now.saturating_sub(self.last_weather_request_ticks) < cooldown
        {
            self.native_apps
                .weather_failed("Refresh is limited to one request every three seconds.");
            slog!("EXPOS_WEATHER_ERROR error=RateLimited\r\n");
            return;
        }
        self.last_weather_request_ticks = now;
        if self.native_apps.weather_needs_location() {
            let mut search_url = [0_u8; 512];
            let Some(search_length) = self.native_apps.weather_search_url(&mut search_url) else {
                self.native_apps
                    .weather_failed("The city name is too long.");
                return;
            };
            let search_url = core::str::from_utf8(&search_url[..search_length]).unwrap_or("");
            let search = match network::https_get_allowlisted(
                &self.broker,
                handle_id,
                APPS_FIN,
                STABLE_FIN,
                search_url,
                &WEATHER_ALLOWED_HOSTS,
            ) {
                Ok(response) if (200..300).contains(&response.status) => Box::new(response),
                Ok(response) => {
                    self.native_apps
                        .weather_failed("The Open-Meteo city search returned an error.");
                    slog!(
                        "EXPOS_WEATHER_ERROR stage=geocoding status={}\r\n",
                        response.status
                    );
                    return;
                }
                Err(error) => {
                    self.native_apps.weather_failed(error.message());
                    slog!("EXPOS_WEATHER_ERROR stage=geocoding error={:?}\r\n", error);
                    return;
                }
            };
            if !self.native_apps.apply_weather_location(search.body()) {
                slog!("EXPOS_WEATHER_ERROR stage=geocoding error=InvalidResponse\r\n");
                return;
            }
            slog!("EXPOS_WEATHER_LOCATION source=open-meteo\r\n");
        } else {
            slog!("EXPOS_WEATHER_LOCATION source=cache\r\n");
        }
        render_active_window(self);
        let mut forecast_url = [0_u8; 512];
        let Some(forecast_length) = self.native_apps.weather_forecast_url(&mut forecast_url) else {
            self.native_apps
                .weather_failed("The forecast request was too long.");
            return;
        };
        let forecast_url = core::str::from_utf8(&forecast_url[..forecast_length]).unwrap_or("");
        let forecast = match network::https_get_allowlisted(
            &self.broker,
            handle_id,
            APPS_FIN,
            STABLE_FIN,
            forecast_url,
            &WEATHER_ALLOWED_HOSTS,
        ) {
            Ok(response) if (200..300).contains(&response.status) => Box::new(response),
            Ok(response) => {
                self.native_apps
                    .weather_failed("The Open-Meteo forecast returned an error.");
                slog!(
                    "EXPOS_WEATHER_ERROR stage=forecast status={}\r\n",
                    response.status
                );
                return;
            }
            Err(error) => {
                self.native_apps.weather_failed(error.message());
                slog!("EXPOS_WEATHER_ERROR stage=forecast error={:?}\r\n", error);
                return;
            }
        };
        if self.native_apps.apply_weather_forecast(forecast.body()) {
            self.persist_native_apps();
            slog!(
                "EXPOS_WEATHER_READY bytes={} provider=open-meteo condition={} days={}\r\n",
                forecast.body_len,
                self.native_apps.weather_condition(),
                self.native_apps.weather_forecast_days()
            );
        } else {
            slog!("EXPOS_WEATHER_ERROR stage=forecast error=InvalidResponse\r\n");
        }
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
        self.terminal_push(
            "  help clear status version hostname pwd whoami id uname uptime neofetch",
        );
        self.terminal_push(
            "  users display resolution network netstat storage theme history cpus audio",
        );
        self.terminal_push("  apps ls ps forms read <name> write <name.txt> <text>");
        self.terminal_push("  ayo list | ayo install <app> | ayo remove <app>");
        self.terminal_push(
            "  echo <text> audiotest audiostop windowreset displaydebug [on|off] displayrepair",
        );
        self.terminal_push("  open <app> close console shutdown reboot");
        self.terminal_push("Use Up/Down for command history.");
    }

    fn terminal_print_neofetch(&mut self) {
        self.terminal_push("        .-------------------------------.");
        self.terminal_push("       /                                 \\");
        self.terminal_push("      |      .--------.   .--------.      |");
        self.terminal_push("      |     /          \\ /          \\     |");
        self.terminal_push("      |    |     O      |     O      |    |");
        self.terminal_push("      |     \\          / \\          /     |");
        self.terminal_push("      |      '--------'   '--------'      |");
        self.terminal_push("       \\_________________________________/");
        self.terminal_push_parts(&["ExpOS v", env!("CARGO_PKG_VERSION")]);
        self.terminal_push("x86_64");
    }

    fn terminal_print_status(&mut self) {
        self.terminal_push("ExpOS desktop is ready.");
        let mut user = [0_u8; 24];
        let user_length = self.session.name().len().min(user.len());
        user[..user_length].copy_from_slice(&self.session.name().as_bytes()[..user_length]);
        let user = core::str::from_utf8(&user[..user_length]).unwrap_or("unknown");
        let authority = self.session.authority_name();
        self.terminal_push_parts(&["user: ", user, " (", authority, ")"]);
        self.terminal_push_parts(&["display: ", framebuffer::active_output_label()]);
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
        self.terminal_push_parts(&[
            "presentation policy: ",
            self.preferences.presentation_policy_label(),
        ]);
        let pacing = self.frame_pacer.stats();
        let scanout = framebuffer::presentation_stats();
        self.terminal_push_number("frames presented: ", scanout.frames, "");
        self.terminal_push_number("pacing misses: ", pacing.missed_frames, "");
        self.terminal_push_number("responsive damage commits: ", self.responsive_commits, "");
        self.terminal_push_number("full-frame commits: ", self.full_frame_commits, "");
        self.terminal_push_number("damaged commits: ", self.damaged_frame_commits, "");
        self.terminal_push_number("surface callbacks: ", self.frame_callbacks, "");
        self.terminal_push_number("pointer packets merged: ", self.pointer_packets_merged, "");
        self.terminal_push_number("deferred presents: ", self.deferred_presents, "");
        self.terminal_push_number("vblank timeouts: ", scanout.vblank_timeouts, "");
        self.terminal_push_number("damage regions submitted: ", scanout.submitted_regions, "");
        self.terminal_push_number("damage regions copied: ", scanout.copied_regions, "");
        self.terminal_push_number("damage pixels copied: ", scanout.copied_pixels, "");
        self.terminal_push_number("damage collapses: ", scanout.damage_collapses, "");
        self.terminal_push_number("damage promotions: ", scanout.damage_promotions, "");
        self.terminal_push_number("empty presents skipped: ", scanout.empty_presents, "");
        self.terminal_push_number("GOP full publications: ", scanout.gop_full_presents, "");
        self.terminal_push_number(
            "GOP partial publications: ",
            scanout.gop_partial_presents,
            "",
        );
        self.terminal_push_number(
            "scanout readback failures: ",
            scanout.gop_readback_failures,
            "",
        );
        self.terminal_push_number("scanout recoveries: ", scanout.gop_recoveries, "");
        self.terminal_push_number("last copy ticks: ", scanout.last_copy_ticks, "");
        self.terminal_push_number("maximum copy ticks: ", scanout.max_copy_ticks, "");
        let portal = self.server.diagnostics();
        self.terminal_push("ExpDisplay Portal protocol: v3");
        self.terminal_push_number("portal surfaces: ", portal.surfaces as u64, "");
        self.terminal_push_number("portal queued events: ", portal.queued_events as u64, "");
        self.terminal_push_number(
            "portal motion events coalesced: ",
            portal.coalesced_pointer_motion,
            "",
        );
        self.terminal_push_number(
            "portal event slots recovered: ",
            portal.recovered_event_slots,
            "",
        );
        self.terminal_push_number("portal events dropped: ", portal.dropped_events, "");
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
                    app.localized_label(self.locale()),
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
        self.terminal_push_number(
            "working Appearance/Windows/Taskbar choices: ",
            CUSTOMIZATION_VALUE_COUNT as u64,
            "",
        );
        self.terminal_push_number(
            "state-schema choices (including reserved): ",
            state::CUSTOMIZATION_SELECTABLE_VALUES as u64,
            "",
        );
        self.terminal_push_parts(&["theme: ", self.preferences.theme.label()]);
        self.terminal_push_parts(&["wallpaper: ", self.preferences.wallpaper.label()]);
        self.terminal_push_parts(&["cursor: ", self.preferences.cursor.label()]);
        self.terminal_push_parts(&["accent: ", self.preferences.accent.label()]);
        self.terminal_push_parts(&[
            "font: ",
            state::FONT_FACE_NAMES[self.preferences.font_face.persisted() as usize],
            " ",
            state::FONT_WEIGHT_NAMES[self.preferences.font_weight.persisted() as usize],
        ]);
        self.terminal_push_parts(&[
            "taskbar: ",
            self.preferences.taskbar_placement.label(),
            " / ",
            self.preferences.taskbar_alignment.label(),
        ]);
        self.terminal_push_parts(&[
            "wallpaper effects: ",
            if self.preferences.wallpaper_effects {
                "on"
            } else {
                "off"
            },
        ]);
        self.terminal_push_parts(&[
            "window shadows: ",
            if self.preferences.window_shadows {
                "on"
            } else {
                "off"
            },
        ]);
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

    fn terminal_print_cpus(&mut self) {
        self.terminal_push_number("online CPUs: ", crate::smp::online_count() as u64, "");
        self.terminal_push("scheduler: preemptive per-CPU Form contexts");
    }

    fn terminal_print_forms(&mut self) {
        let Some(snapshot) = crate::expfs_store::loaded_snapshot(self.cfc) else {
            self.terminal_push("ExpFS Form graph is unavailable.");
            return;
        };
        self.terminal_push("Forms:");
        let mut count = 0;
        for stored in snapshot.forms.iter().flatten() {
            self.terminal_push_parts(&[
                "  ",
                stored.form.name.as_str(),
                "  ",
                form_kind_label(stored.form.kind),
            ]);
            count += 1;
        }
        if count == 0 {
            self.terminal_push("  none");
        }
    }

    fn terminal_read_form(&mut self, arguments: &[u8]) {
        let Ok(name) = core::str::from_utf8(trim_ascii(arguments)) else {
            self.terminal_push("Form name must be UTF-8.");
            return;
        };
        if name.is_empty() {
            self.terminal_push("usage: read <form-name>");
            return;
        }
        let Some(snapshot) = crate::expfs_store::loaded_snapshot(self.cfc) else {
            self.terminal_push("ExpFS Form graph is unavailable.");
            return;
        };
        let Some(stored) = snapshot
            .forms
            .iter()
            .flatten()
            .find(|stored| stored.form.name.as_str() == name)
        else {
            self.terminal_push("Form not found.");
            return;
        };
        self.terminal_push_parts(&[
            stored.form.name.as_str(),
            "  ",
            form_kind_label(stored.form.kind),
        ]);
        self.terminal_push_bytes(&stored.content[..stored.content_len as usize]);
    }

    fn terminal_write_form(&mut self, arguments: &[u8]) {
        let (name, content) = split_command(arguments);
        let Ok(name) = core::str::from_utf8(name) else {
            self.terminal_push("Form name must be UTF-8.");
            return;
        };
        if name.is_empty() || content.is_empty() {
            self.terminal_push("usage: write <name.txt> <text>");
            return;
        }
        match crate::expfs_store::save_text_form(self.cfc, name, content) {
            Ok((_fin, revision)) => {
                self.terminal_push_number("saved revision ", revision as u64, "");
            }
            Err(error) => self.terminal_push_parts(&["write failed: ", error.message()]),
        }
    }

    fn terminal_ayo(&mut self, arguments: &[u8]) {
        let (operation, package) = split_command(arguments);
        if operation.is_empty()
            || operation.eq_ignore_ascii_case(b"help")
            || operation.eq_ignore_ascii_case(b"glance")
        {
            self.terminal_push("Ayo: list, install/slap <app>, remove/yeet <app>, open <app>");
            self.terminal_push("Apps are not installed until you choose them.");
            return;
        }
        if operation.eq_ignore_ascii_case(b"list") {
            self.terminal_push("Ayo catalog:");
            for index in 0..crate::apps::PACKAGE_COUNT {
                let installed = if self.native_apps.installed(index) {
                    " [installed]"
                } else {
                    ""
                };
                self.terminal_push_parts(&[
                    "  ",
                    crate::apps::package_name(index).unwrap_or("App"),
                    " / ",
                    crate::apps::package_category(index).unwrap_or("Other"),
                    installed,
                ]);
            }
            return;
        }
        let Ok(package) = core::str::from_utf8(trim_ascii(package)) else {
            self.terminal_push("Package name must be UTF-8.");
            return;
        };
        let Some(index) = crate::apps::NativeApps::find_package(package) else {
            self.terminal_push("Package not found. Run: ayo list");
            return;
        };
        if operation.eq_ignore_ascii_case(b"install") || operation.eq_ignore_ascii_case(b"slap") {
            if self.transact_native_package(index, true) {
                self.terminal_push_parts(&[
                    "Installed ",
                    crate::apps::package_name(index).unwrap_or("App"),
                    ".",
                ]);
            } else {
                self.terminal_push("Ayo install was denied by the package Handle.");
            }
        } else if operation.eq_ignore_ascii_case(b"remove")
            || operation.eq_ignore_ascii_case(b"yeet")
        {
            if !self.native_apps.installed(index) {
                self.terminal_push("That app is not installed.");
            } else if self.transact_native_package(index, false) {
                self.terminal_push_parts(&[
                    "Removed ",
                    crate::apps::package_name(index).unwrap_or("App"),
                    ".",
                ]);
            }
        } else if operation.eq_ignore_ascii_case(b"open") {
            if self.native_apps.activate_installed(index) {
                self.terminal_push_parts(&[
                    "Opening ",
                    crate::apps::package_name(index).unwrap_or("App"),
                    ".",
                ]);
                self.switch_to(AppKind::Apps);
            } else {
                self.terminal_push("Install that app first with: ayo install <app>");
            }
        } else {
            self.terminal_push("usage: ayo <list|install|remove|open> [app]");
        }
    }

    fn execute_terminal_command(&mut self, command: &[u8]) {
        let (name, arguments) = split_command(command);
        if let Ok(name) = core::str::from_utf8(name) {
            slog!("EXPOS_TERMINAL_COMMAND name={}\r\n", name);
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
            self.terminal_push("ExpOS expos-kernel x86_64");
        } else if name.eq_ignore_ascii_case(b"uptime") {
            self.terminal_push_number("monotonic ticks: ", crate::hardware::timestamp(), "");
        } else if name.eq_ignore_ascii_case(b"neofetch") || name.eq_ignore_ascii_case(b"sysinfo") {
            self.terminal_print_neofetch();
        } else if name.eq_ignore_ascii_case(b"users") {
            self.terminal_print_users();
        } else if name.eq_ignore_ascii_case(b"display") || name.eq_ignore_ascii_case(b"resolution")
        {
            self.terminal_print_display();
        } else if name.eq_ignore_ascii_case(b"network") || name.eq_ignore_ascii_case(b"netstat") {
            self.terminal_print_network();
        } else if name.eq_ignore_ascii_case(b"storage") {
            self.terminal_print_storage();
        } else if name.eq_ignore_ascii_case(b"audio") {
            let status = audio::status();
            slog!(
                "EXPOS_AUDIO_STATUS backend={} volume={} muted={} playing={} submitted={} completed={} underruns={}\r\n",
                if status.backend == audio::Backend::Ac97 {
                    "ac97"
                } else {
                    "unavailable"
                },
                status.volume,
                status.muted,
                status.playing,
                status.submitted_frames,
                status.completed_buffers,
                status.underruns
            );
            self.terminal_push_parts(&["audio backend: ", status.backend.label()]);
            self.terminal_push_number("volume: ", status.volume as u64, "%");
            self.terminal_push_parts(&[
                "muted: ",
                if status.muted { "yes" } else { "no" },
                "  playing: ",
                if status.playing { "yes" } else { "no" },
            ]);
            self.terminal_push_number("submitted frames: ", status.submitted_frames, "");
        } else if name.eq_ignore_ascii_case(b"audiotest") {
            if audio::play_tone(660, 120).is_ok() {
                self.terminal_push("Playing a bounded 660 Hz stereo test tone.");
            } else {
                self.terminal_push("Audio output is unavailable on this device.");
            }
        } else if name.eq_ignore_ascii_case(b"audiostop") {
            audio::stop();
            self.terminal_push("Audio output stopped.");
        } else if name.eq_ignore_ascii_case(b"cpus") || name.eq_ignore_ascii_case(b"smp") {
            self.terminal_print_cpus();
        } else if name.eq_ignore_ascii_case(b"theme") {
            self.terminal_print_theme();
        } else if name.eq_ignore_ascii_case(b"history") {
            self.terminal_print_history();
        } else if name.eq_ignore_ascii_case(b"forms") {
            self.terminal_print_forms();
        } else if name.eq_ignore_ascii_case(b"read") {
            self.terminal_read_form(arguments);
        } else if name.eq_ignore_ascii_case(b"write") {
            self.terminal_write_form(arguments);
        } else if name.eq_ignore_ascii_case(b"ayo") {
            self.terminal_ayo(arguments);
        } else if name.eq_ignore_ascii_case(b"apps") || name.eq_ignore_ascii_case(b"ls") {
            self.terminal_push("browser terminal forms packages settings system games notes");
        } else if name.eq_ignore_ascii_case(b"ps") {
            self.terminal_print_running_apps();
        } else if name.eq_ignore_ascii_case(b"echo") {
            self.terminal_push_bytes(arguments);
        } else if name.eq_ignore_ascii_case(b"windowreset")
            || name.eq_ignore_ascii_case(b"resetwindows")
        {
            self.reset_window_layout();
            self.terminal_push("All application windows returned to their default positions.");
        } else if name.eq_ignore_ascii_case(b"displaydebug") {
            self.display_debug_overlay = if arguments.eq_ignore_ascii_case(b"on") {
                true
            } else if arguments.eq_ignore_ascii_case(b"off") {
                false
            } else {
                !self.display_debug_overlay
            };
            self.full_redraw_requested = true;
            self.terminal_push_parts(&[
                "ExpDisplay v3 debug overlay: ",
                if self.display_debug_overlay {
                    "on"
                } else {
                    "off"
                },
            ]);
            slog!(
                "EXPOS_DISPLAY_DEBUG_OVERLAY enabled={}\r\n",
                self.display_debug_overlay
            );
        } else if name.eq_ignore_ascii_case(b"displayrepair") {
            framebuffer::request_full_reconcile();
            self.full_redraw_requested = true;
            self.terminal_push("Full shadow-to-scanout reconciliation requested.");
            slog!("EXPOS_DISPLAY_V3_REPAIR_REQUESTED\r\n");
        } else if name.eq_ignore_ascii_case(b"open") {
            if let Some(app) = parse_app(arguments) {
                self.terminal_push_parts(&["Opening ", app.localized_label(self.locale()), "."]);
                self.switch_to(app);
            } else if let Ok(name) = core::str::from_utf8(trim_ascii(arguments)) {
                if let Some(index) = crate::apps::NativeApps::find_package(name) {
                    if self.native_apps.activate_installed(index) {
                        self.terminal_push_parts(&[
                            "Opening ",
                            crate::apps::package_name(index).unwrap_or("App"),
                            ".",
                        ]);
                        self.switch_to(AppKind::Apps);
                    } else {
                        self.terminal_push("That Ayo app is not installed.");
                    }
                } else {
                    self.terminal_push("usage: open <built-in or installed Ayo app>");
                }
            } else {
                self.terminal_push("usage: open <built-in or installed Ayo app>");
            }
        } else if name.eq_ignore_ascii_case(b"close") || name.eq_ignore_ascii_case(b"exit") {
            self.close_active();
        } else if name.eq_ignore_ascii_case(b"console") {
            self.should_exit = true;
            slog!("EXPOS_TERMINAL_CONSOLE_REQUESTED\r\n");
        } else if name.eq_ignore_ascii_case(b"shutdown") || name.eq_ignore_ascii_case(b"halt") {
            slog!("EXPOS_COMMAND_OK shutdown\r\n");
            crate::port::shutdown();
        } else if name.eq_ignore_ascii_case(b"reboot") {
            slog!("EXPOS_COMMAND_OK reboot\r\n");
            crate::port::reboot();
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

pub fn run_with_network(
    input: &mut Input,
    start_browser: bool,
    session: crate::session::Session,
    allow_network: bool,
    cfc: CfcFin,
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
        cfc,
    );
}

pub fn run_games(input: &mut Input, session: crate::session::Session, cfc: CfcFin) {
    run_session(input, Some(AppKind::Games), session, false, cfc);
}

fn run_session(
    input: &mut Input,
    start_app: Option<AppKind>,
    session: crate::session::Session,
    allow_network: bool,
    cfc: CfcFin,
) {
    let mouse_ready = input.enable_mouse();
    if !framebuffer::enter() {
        crate::println!("ExpDisplay unavailable: no Bochs/QEMU VBE framebuffer.");
        slog!("EXPOS_DISPLAY_UNAVAILABLE\r\n");
        return;
    }

    let mut desktop = DesktopState::new(start_app, session, allow_network, cfc);
    // Desktop construction can include capability setup and document parsing;
    // begin presentation timing only when the first frame is ready to draw.
    desktop
        .frame_pacer
        .reset_phase(crate::hardware::timestamp());
    render(&mut desktop);
    if desktop.should_exit {
        slog!("EXPOS_DISPLAY_SESSION_ABORTED reason=scanout-not-visible\r\n");
        framebuffer::exit();
        crate::clear_console();
        crate::println!("ExpDisplay could not confirm scanout; command environment restored.");
        return;
    }
    slog!("EXPOS_DISPLAY_READY surfaces=12 commit=12\r\n");
    let portal = expos_core::display_protocol_info();
    slog!(
        "EXPOS_DISPLAY_PORTAL version={} features={:#x} max_surfaces={} max_events={}\r\n",
        portal.version,
        portal.features.bits(),
        portal.max_surfaces,
        portal.max_events
    );
    if start_app.is_none() {
        slog!("EXPOS_DESKTOP_EMPTY open_apps=0 pinned_apps=0\r\n");
    }
    slog!("EXPOS_MOUSE_READY enabled={}\r\n", mouse_ready);
    slog!(
        "EXPOS_DESKTOP_PREFS theme={} wallpaper={} cursor={} accent={}\r\n",
        desktop.preferences.theme.label(),
        desktop.preferences.wallpaper.label(),
        desktop.preferences.cursor.label(),
        desktop.preferences.accent.label()
    );
    slog!(
        "EXPOS_CUSTOMIZATION font={} weight={} radius={} border={} titlebar={} opacity={} offscreen={} snap={} focus={} taskbar={} size={} align={} autohide={} translucent={} labels={} seconds={}\r\n",
        state::FONT_FACE_NAMES[desktop.preferences.font_face.persisted() as usize],
        state::FONT_WEIGHT_NAMES[desktop.preferences.font_weight.persisted() as usize],
        CORNER_RADIUS_LABELS[desktop.preferences.window_corner_radius as usize],
        BORDER_WIDTH_LABELS[desktop.preferences.window_border_width as usize],
        TITLEBAR_LABELS[desktop.preferences.titlebar_density as usize],
        WINDOW_OPACITY_LABELS[desktop.preferences.window_opacity as usize],
        OFFSCREEN_LABELS[desktop.preferences.window_offscreen_allowance as usize],
        desktop.preferences.window_snap,
        desktop.preferences.focus_policy_label(),
        desktop.preferences.taskbar_placement.label(),
        TASKBAR_SIZE_LABELS[desktop.preferences.taskbar_size as usize],
        desktop.preferences.taskbar_alignment.label(),
        desktop.preferences.taskbar_autohide,
        desktop.preferences.taskbar_translucent,
        desktop.preferences.taskbar_labels,
        desktop.preferences.clock_seconds,
    );
    let clock = crate::hardware::clock_info();
    slog!(
        "EXPOS_PRESENTATION_READY rate={} vsync={} pageflip={} clock_hz={} source={}\r\n",
        refresh_rate_label(desktop.preferences.refresh_rate),
        desktop.preferences.vsync,
        framebuffer::presentation_stats().page_flip_available,
        clock.tsc_hz,
        clock.source.label()
    );
    slog!(
        "EXPOS_RENDER_POLICY mode={} damage=true shadows={} wallpaper_effects={}\r\n",
        desktop.preferences.presentation_policy_label(),
        desktop.preferences.window_shadows,
        desktop.preferences.wallpaper_effects
    );

    let mut rendered_clock = crate::hardware::rtc_time();
    let clock_poll_ticks = clock.tsc_hz.max(1);
    let mut next_clock_poll = crate::hardware::timestamp().saturating_add(clock_poll_ticks);
    let weather_animation_ticks = (clock.tsc_hz / 4).max(1);
    let mut next_weather_animation =
        crate::hardware::timestamp().saturating_add(weather_animation_ticks);

    while !desktop.should_exit {
        let Some(event) = input.poll_event() else {
            let now = crate::hardware::timestamp();
            let clock_changed = if now >= next_clock_poll {
                next_clock_poll = now.saturating_add(clock_poll_ticks);
                let current_clock = crate::hardware::rtc_time();
                let changed = if desktop.preferences.clock_seconds {
                    current_clock != rendered_clock
                } else {
                    current_clock.hour != rendered_clock.hour
                        || current_clock.minute != rendered_clock.minute
                };
                rendered_clock = current_clock;
                changed
            } else {
                false
            };
            if clock_changed && desktop.preferences.status_visible && desktop.taskbar_should_show()
            {
                render_taskbar_damage(&mut desktop);
            } else if desktop.active == AppKind::Games
                && desktop.app_is_visible(AppKind::Games)
                && desktop.games.tick(now)
            {
                render_active_window(&mut desktop);
            } else if now >= next_weather_animation
                && desktop.preferences.animation_level != 0
                && desktop.active == AppKind::Apps
                && desktop.app_is_visible(AppKind::Apps)
                && desktop.native_apps.weather_animating()
            {
                next_weather_animation = now.saturating_add(weather_animation_ticks);
                render_active_window(&mut desktop);
            } else {
                let _ = desktop.frame_pacer.decide(now, false);
                core::hint::spin_loop();
            }
            continue;
        };
        let key = match event {
            InputEvent::Key(key) => key,
            InputEvent::Pointer(pointer) => {
                let (pointer, merged) = input.coalesce_pointer_motion(pointer);
                desktop.pointer_packets_merged =
                    desktop.pointer_packets_merged.saturating_add(merged as u64);
                match desktop.handle_pointer(pointer) {
                    PointerRender::None => {}
                    PointerRender::Cursor(damage) => {
                        present_frame_damage(&mut desktop, &[damage]);
                    }
                    PointerRender::Full => render(&mut desktop),
                }
                continue;
            }
        };
        if desktop.shell_confirmation {
            match key.to_ascii_lowercase() {
                b'y' | b'\n' => {
                    desktop.shell_confirmation = false;
                    slog!("EXPOS_SHELL_CONFIRMATION state=accepted\r\n");
                    desktop.should_exit = true;
                    slog!("EXPOS_DESKTOP_SHELL_REQUESTED\r\n");
                }
                b'n' | 0x1B | KEY_DESKTOP_SHELL_CONFIRM => {
                    desktop.shell_confirmation = false;
                    slog!("EXPOS_SHELL_CONFIRMATION state=cancelled\r\n");
                }
                _ => {}
            }
            render(&mut desktop);
            continue;
        }
        if key == KEY_DESKTOP_SHELL_CONFIRM {
            desktop.close_launcher();
            desktop.shell_confirmation = true;
            slog!("EXPOS_SHELL_CONFIRMATION state=open\r\n");
            render(&mut desktop);
            continue;
        }
        desktop.route_key(key);

        if key == 0x1B {
            if desktop.browser_editing || desktop.browser_find_editing {
                desktop.browser_editing = false;
                desktop.browser_find_editing = false;
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
            continue;
        }
        if key == b'\x60' || key == b'~' || key == KEY_SUPER_LAUNCHER {
            desktop.toggle_launcher();
            render(&mut desktop);
            continue;
        }
        if desktop.launcher_open {
            let columns = launcher_columns(desktop.preferences) as isize;
            match key {
                KEY_UP => desktop.move_launcher_selection(-columns),
                KEY_DOWN => desktop.move_launcher_selection(columns),
                KEY_LEFT => desktop.move_launcher_selection(-1),
                KEY_RIGHT => desktop.move_launcher_selection(1),
                b'\t' => desktop.cycle_launcher_selection(),
                b'\n' | b' ' => desktop.activate_launcher_selection(),
                _ => {}
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
        if desktop.active == AppKind::Packages && desktop.app_is_visible(AppKind::Packages) {
            let generation = desktop.native_apps.persistence_generation();
            if let Some(action) = desktop.native_apps.handle_manager_key(key) {
                if desktop.native_apps.persistence_generation() != generation {
                    desktop.persist_native_apps();
                }
                let open = action == crate::apps::ManagerAction::Open;
                desktop.complete_manager_action(action);
                if open {
                    render(&mut desktop);
                } else {
                    render_active_window(&mut desktop);
                }
                continue;
            }
        }
        if desktop.active == AppKind::Apps && desktop.app_is_visible(AppKind::Apps) {
            let generation = desktop.native_apps.persistence_generation();
            if let Some(action) = desktop.native_apps.handle_app_key(key) {
                if desktop.native_apps.persistence_generation() != generation {
                    desktop.persist_native_apps();
                }
                desktop.complete_native_app_action(action);
                render_active_window(&mut desktop);
                continue;
            }
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
            KEY_SUPER_CLOSE => desktop.close_active(),
            KEY_SUPER_CYCLE => desktop.cycle_app(),
            KEY_SUPER_FULLSCREEN => desktop.toggle_fullscreen(),
            KEY_SUPER_LEFT => desktop.move_active(-12, 0),
            KEY_SUPER_RIGHT => desktop.move_active(12, 0),
            KEY_SUPER_UP => desktop.move_active(0, -12),
            KEY_SUPER_DOWN => desktop.move_active(0, 12),
            KEY_SUPER_ALT_LEFT => desktop.tile_active(false),
            KEY_SUPER_ALT_RIGHT => desktop.tile_active(true),
            KEY_SUPER_ALT_UP => desktop.toggle_fullscreen(),
            KEY_SUPER_ALT_DOWN => desktop.minimize_active(),
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
            b'q' | b'Q' => {}
            _ => {
                if desktop.active == AppKind::Browser {
                    match key.to_ascii_lowercase() {
                        b'/' | b'l' => {
                            desktop.browser_len = 0;
                            desktop.browser_editing = true;
                        }
                        b'h' => desktop.navigate("expos://home", HOME),
                        b'1' | b'a' => desktop.navigate("expos://about", ABOUT),
                        b'2' => desktop.navigate("expos://packages", BROWSER_PACKAGES),
                        b'3' => desktop.navigate("expos://system", BROWSER_SYSTEM),
                        b'n' => desktop.navigate("expos://blocked", NETWORK_BLOCKED),
                        _ => {}
                    }
                }
            }
        }
        render(&mut desktop);
    }

    let pacing = desktop.frame_pacer.stats();
    let presentation = framebuffer::presentation_stats();
    let portal = desktop.server.diagnostics();
    slog!(
        "EXPOS_PRESENTATION_STATS frames={} missed={} idle={} vblank_timeouts={} responsive_commits={}\r\n",
        presentation.frames,
        pacing.missed_frames,
        pacing.idle_frames,
        presentation.vblank_timeouts,
        desktop.responsive_commits
    );
    slog!(
        "EXPOS_RENDER_STATS full={} damaged={} callbacks={} surface_frames={} pointer_merged={} submitted_regions={} copied_regions={} copied_pixels={} collapses={} promotions={} gop_full={} gop_partial={} readback_failures={} recoveries={} max_copy_ticks={} deferred={}\r\n",
        desktop.full_frame_commits,
        desktop.damaged_frame_commits,
        desktop.frame_callbacks,
        desktop.server.frame_sequence(),
        desktop.pointer_packets_merged,
        presentation.submitted_regions,
        presentation.copied_regions,
        presentation.copied_pixels,
        presentation.damage_collapses,
        presentation.damage_promotions,
        presentation.gop_full_presents,
        presentation.gop_partial_presents,
        presentation.gop_readback_failures,
        presentation.gop_recoveries,
        presentation.max_copy_ticks,
        desktop.deferred_presents
    );
    slog!(
        "EXPOS_PORTAL_V3_STATS commits={} frames={} callbacks={} queued={} pending={} frame_coalesced={} motion_coalesced={} recovered_slots={} dropped={}\r\n",
        portal.commits,
        portal.frames,
        portal.callbacks,
        portal.queued_events,
        portal.pending_frame_callbacks,
        portal.coalesced_frame_callbacks,
        portal.coalesced_pointer_motion,
        portal.recovered_event_slots,
        portal.dropped_events
    );
    framebuffer::exit();
    crate::clear_console();
    crate::println!("ExpDisplay session closed; command environment restored.");
    slog!("EXPOS_DISPLAY_CLOSED\r\n");
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

const fn form_kind_label(kind: FormKind) -> &'static str {
    match kind {
        FormKind::Root => "root",
        FormKind::Service => "service",
        FormKind::Interface => "interface",
        FormKind::Package => "package",
        FormKind::Driver => "driver",
        FormKind::Data => "data",
        FormKind::Policy => "policy",
        FormKind::Executable => "executable",
    }
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
    } else if value.eq_ignore_ascii_case(b"apps") || value.eq_ignore_ascii_case(b"a") {
        Some(AppKind::Apps)
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

fn default_rect(app: AppKind, preferences: DesktopPreferences) -> Rect {
    let offset = (app.index() % 4) as i16;
    let area = work_area(preferences);
    let (app_width, app_height) = app_dimensions(preferences);
    let centered_x = area.x
        + ((area.width as i32 - app_width as i32) / 2)
            .max(4)
            .min(i16::MAX as i32) as i16;
    let centered_y = area.y
        + ((area.height as i32 - app_height as i32) / 2)
            .max(4)
            .min(i16::MAX as i32) as i16;
    let max_x = (area.x + area.width as i16 - app_width as i16).max(area.x + 4);
    let max_y = (area.y + area.height as i16 - app_height as i16).max(area.y + 4);
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
    if !preferences.wallpaper_effects {
        framebuffer::clear(base);
        return;
    }
    match preferences.wallpaper {
        WallpaperChoice::Solid => {
            framebuffer::clear(base);
            framebuffer::alpha_rect(0, 0, width, height, preferences.wallpaper_color(), 34);
        }
        WallpaperChoice::Gradient => {
            let (top, bottom) = match preferences.backdrop {
                BackdropChoice::Graphite => (0x0019_1D20, 0x0007_090B),
                BackdropChoice::Midnight => (0x0009_1828, 0x0002_060B),
                BackdropChoice::Black => (0x0008_0A0D, 0x0000_0000),
                BackdropChoice::Custom(value) => (
                    blend_color(0x0007_0A0D, spectrum_color(value), 64),
                    blend_color(0x0000_0102, spectrum_color(value), 24),
                ),
            };
            framebuffer::vertical_gradient(0, 0, width, height, top, bottom);
            framebuffer::alpha_rect(0, 0, width, height, preferences.wallpaper_color(), 24);
        }
        WallpaperChoice::Horizon => {
            let accent = preferences.wallpaper_color();
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
            let grid = preferences.wallpaper_color();
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
                preferences.wallpaper_color(),
                28,
            );
        }
        WallpaperChoice::Aurora => {
            framebuffer::vertical_gradient(0, 0, width, height, 0x0004_101B, 0x0002_060B);
            // V2 used overlapping, fully saturated slabs here. With a bright
            // custom palette those slabs looked like framebuffer corruption.
            // V3 keeps separated low-alpha glows over the dark gradient.
            let accent = blend_color(base, preferences.wallpaper_color(), 72);
            let secondary = blend_color(base, color::CYAN, 56);
            let band_height = (height / 12).max(22);
            let gap = (height / 7).max(band_height + 12);
            for band in 0..4 {
                let y = height / 8 + band * gap;
                let inset = band * width / 20;
                framebuffer::alpha_rounded_rect(
                    inset - width / 8,
                    y,
                    width - inset / 3,
                    band_height,
                    band_height / 2,
                    if band % 2 == 0 { accent } else { secondary },
                    88,
                );
            }
            framebuffer::alpha_rect(0, 0, width, height, base, 70);
        }
        WallpaperChoice::Mesh => {
            framebuffer::vertical_gradient(0, 0, width, height, base, 0x0002_0508);
            let accent = preferences.wallpaper_color();
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
    desktop.full_redraw_requested = false;
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
    if desktop.taskbar_should_show() {
        draw_dock(desktop);
    }
    if desktop.launcher_open {
        draw_launcher(desktop);
    }
    if desktop.shell_confirmation {
        draw_shell_confirmation(desktop);
    }
    if desktop.display_debug_overlay {
        draw_display_debug_overlay(desktop);
    }
    desktop.cursor.draw();
    present_frame(desktop);
}

fn render_active_window(desktop: &mut DesktopState) {
    // Translucent surfaces depend on every layer below them. Reblending only the
    // active rectangle would gradually change its color on each damaged commit.
    // A requested composition change (open/close/reset/focus) likewise needs the
    // wallpaper, z-order and taskbar to be rebuilt as one coherent scene.
    if desktop.full_redraw_requested || desktop.preferences.window_alpha() < u8::MAX {
        render(desktop);
        return;
    }
    desktop.cursor.restore();
    let mut damage = [desktop.cursor.damage_region(); 2];
    let mut damage_count = 1;
    if desktop.app_is_visible(desktop.active) {
        let rect = desktop
            .server
            .surface(desktop.active_surface())
            .map(|surface| surface.current.rect)
            .unwrap_or_else(|| {
                let (width, height) = app_dimensions(desktop.preferences);
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
    present_frame_damage(desktop, &damage[..damage_count]);
}

fn render_taskbar_damage(desktop: &mut DesktopState) {
    // An opaque taskbar can be updated independently when only the RTC text
    // changes. This avoids rebuilding the wallpaper and every visible window
    // once per second. A translucent panel still needs the coherent backdrop.
    if desktop.full_redraw_requested || desktop.preferences.taskbar_translucent {
        render(desktop);
        return;
    }
    desktop.cursor.restore();
    let cursor_damage = desktop.cursor.damage_region();
    draw_dock(desktop);
    let dock = taskbar_rect(desktop.preferences);
    desktop.cursor.draw();
    let regions = [
        cursor_damage,
        framebuffer::DamageRegion::new(
            dock.x as i32,
            dock.y as i32,
            dock.width as i32,
            dock.height as i32,
        ),
    ];
    present_frame_damage(desktop, &regions);
}

fn present_frame(desktop: &mut DesktopState) {
    pace_frame(desktop, false);
    let visible = framebuffer::present(desktop.preferences.vsync);
    desktop.full_frame_commits = desktop.full_frame_commits.saturating_add(1);
    complete_visible_frame(desktop, visible);
    desktop.drain_protocol_events();
}

fn present_frame_damage(desktop: &mut DesktopState, damage: &[framebuffer::DamageRegion]) {
    pace_frame(desktop, true);
    let mut debug_damage = [framebuffer::DamageRegion::new(0, 0, 0, 0); 8];
    let damage = if desktop.display_debug_overlay && damage.len() < debug_damage.len() {
        desktop.cursor.restore();
        draw_display_debug_overlay(desktop);
        desktop.cursor.draw();
        debug_damage[..damage.len()].copy_from_slice(damage);
        debug_damage[damage.len()] = display_debug_region();
        &debug_damage[..damage.len() + 1]
    } else {
        damage
    };
    let visible = framebuffer::present_damage(desktop.preferences.vsync, damage);
    desktop.damaged_frame_commits = desktop.damaged_frame_commits.saturating_add(1);
    complete_visible_frame(desktop, visible);
    desktop.drain_protocol_events();
}

fn display_debug_region() -> framebuffer::DamageRegion {
    let screen_width = framebuffer::width() as i32;
    let width = (screen_width - 20).clamp(220, 318);
    framebuffer::DamageRegion::new(screen_width - width - 10, 10, width, 126)
}

/// Draw a deliberately opaque, self-contained diagnostics surface.
///
/// Keeping the overlay independent of the wallpaper and application stack
/// makes it safe to include in small damage commits. It is intentionally
/// backed by live compositor, portal and scanout counters rather than static
/// status text so a corrupted or stalled presentation path is observable from
/// inside the graphical session.
fn draw_display_debug_overlay(desktop: &DesktopState) {
    let region = display_debug_region();
    let x = region.x;
    let y = region.y;
    let width = region.width;
    let scanout = framebuffer::presentation_stats();
    let portal = desktop.server.diagnostics();
    let accent = desktop.preferences.accent.color();
    let text = color::INK;
    let muted = color::MUTED;

    framebuffer::rounded_rect(x, y, width, region.height, 8, 0x0008_0c12);
    framebuffer::rounded_outline(x, y, width, region.height, 8, accent);
    framebuffer::text(x + 12, y + 10, "EXPDISPLAY PORTAL V3", text, 1);

    framebuffer::text(x + 12, y + 30, "FRAME", muted, 1);
    draw_number(x + 82, y + 30, scanout.frames, text);
    framebuffer::text(x + 158, y + 30, "COPY", muted, 1);
    draw_number(x + 214, y + 30, scanout.last_copy_ticks, text);

    framebuffer::text(x + 12, y + 48, "DAMAGE PX", muted, 1);
    draw_number(x + 100, y + 48, scanout.last_copied_pixels, text);
    framebuffer::text(x + 190, y + 48, "REG", muted, 1);
    draw_number(x + 230, y + 48, scanout.last_copied_regions, text);

    framebuffer::text(x + 12, y + 66, "SURFACES", muted, 1);
    draw_number(x + 100, y + 66, portal.visible_surfaces as u64, text);
    framebuffer::text(x + 158, y + 66, "EVENTS", muted, 1);
    draw_number(x + 222, y + 66, portal.queued_events as u64, text);

    framebuffer::text(x + 12, y + 84, "MERGED", muted, 1);
    draw_number(x + 82, y + 84, portal.coalesced_pointer_motion, text);
    framebuffer::text(x + 158, y + 84, "DROPPED", muted, 1);
    draw_number(x + 230, y + 84, portal.dropped_events, text);

    framebuffer::text(x + 12, y + 102, "RECOVERIES", muted, 1);
    draw_number(x + 116, y + 102, scanout.gop_recoveries, text);
    framebuffer::text(x + 190, y + 102, "PROMOTE", muted, 1);
    draw_number(x + 262, y + 102, scanout.damage_promotions, text);
}

fn complete_visible_frame(desktop: &mut DesktopState, visible: bool) {
    if visible {
        desktop.frame_callbacks = desktop
            .frame_callbacks
            .saturating_add(desktop.server.complete_frame() as u64);
    } else {
        desktop.deferred_presents = desktop.deferred_presents.saturating_add(1);
        desktop.should_exit = true;
        slog!("EXPOS_FRAME_DEFERRED reason=scanout-not-visible\r\n");
    }
}

fn pace_frame(desktop: &mut DesktopState, damaged_commit: bool) {
    if bypass_software_pacing(desktop.preferences.responsive_presentation, damaged_commit) {
        let now = crate::hardware::timestamp();
        desktop.frame_pacer.reset_phase(now);
        desktop.responsive_commits = desktop.responsive_commits.saturating_add(1);
        return;
    }
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
                    slog!("EXPOS_FRAME_MISSED count={}\r\n", missed_frames);
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
            let (width, height) = app_dimensions(desktop.preferences);
            Rect::new(8, 8, width, height)
        });
    let title = if app == AppKind::Apps && desktop.native_apps.installed_count() != 0 {
        crate::apps::NativeApps::app_name(desktop.native_apps.active_index())
    } else {
        app.localized_label(desktop.locale())
    };
    draw_window(rect, title, focused, desktop.preferences, draw_shadow);
    let responsive_full = matches!(
        app,
        AppKind::Browser
            | AppKind::Terminal
            | AppKind::Packages
            | AppKind::Settings
            | AppKind::Notes
            | AppKind::Apps
    ) && rect.width >= 480
        && rect.height >= 360;
    if responsive_full || (rect.width >= 600 && rect.height >= 380) {
        match app {
            AppKind::Browser => draw_browser(rect, desktop),
            AppKind::Terminal => draw_terminal(rect, desktop),
            AppKind::Forms => draw_forms(rect),
            AppKind::Packages => desktop.native_apps.render_manager(rect),
            AppKind::Settings => draw_settings(rect, desktop),
            AppKind::System => draw_system(rect, desktop),
            AppKind::Games => desktop.games.render(rect),
            AppKind::Notes => draw_notes(rect, desktop),
            AppKind::Apps => desktop.native_apps.render_app(rect),
        }
    }
    draw_window_border(rect, focused, desktop.preferences);
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
    if draw_shadow && preferences.window_shadows && width > 24 && height > 24 {
        framebuffer::alpha_rect(x + 8, y + 10, width, height, 0x0000_0000, 105);
    }
    let radius = if preferences.rounded_controls {
        preferences.window_corner_radius()
    } else {
        0
    };
    let alpha = preferences.window_alpha();
    if alpha < u8::MAX && radius > 0 {
        framebuffer::alpha_rounded_rect(
            x,
            y,
            width,
            height,
            radius,
            preferences.window_color(),
            alpha,
        );
    } else if alpha < u8::MAX {
        framebuffer::alpha_rect(x, y, width, height, preferences.window_color(), alpha);
    } else if radius > 0 {
        framebuffer::rounded_rect(x, y, width, height, radius, preferences.window_color());
    } else {
        framebuffer::rect(x, y, width, height, preferences.window_color());
    }
    if focused {
        framebuffer::rect(x + 1, y + 1, width - 2, 2, preferences.accent.color());
    }
    let titlebar_height = preferences.titlebar_height() as i32;
    if alpha < u8::MAX && radius > 0 {
        framebuffer::alpha_rounded_rect(
            x + 1,
            y + 3,
            width - 2,
            titlebar_height - 3,
            radius.min((titlebar_height - 3) / 2),
            preferences.chrome_color(),
            alpha,
        );
        framebuffer::alpha_rect(
            x + 1,
            y + titlebar_height / 2,
            width - 2,
            titlebar_height / 2,
            preferences.chrome_color(),
            alpha,
        );
    } else if alpha < u8::MAX {
        framebuffer::alpha_rect(
            x + 1,
            y + 3,
            width - 2,
            titlebar_height - 3,
            preferences.chrome_color(),
            alpha,
        );
    } else if radius > 0 {
        framebuffer::rounded_rect(
            x + 1,
            y + 3,
            width - 2,
            titlebar_height - 3,
            radius.min((titlebar_height - 3) / 2),
            preferences.chrome_color(),
        );
        framebuffer::rect(
            x + 1,
            y + titlebar_height / 2,
            width - 2,
            titlebar_height / 2,
            preferences.chrome_color(),
        );
    } else {
        framebuffer::rect(
            x + 1,
            y + 3,
            width - 2,
            titlebar_height - 3,
            preferences.chrome_color(),
        );
    }
    let text_y = y + ((titlebar_height - 8) / 2).max(5);
    framebuffer::text(x + 12, text_y, title, color::INK, 1);
    framebuffer::line(
        x + width - 126,
        y + 3,
        x + width - 126,
        y + titlebar_height - 1,
        color::BORDER,
    );
    framebuffer::line(
        x + width - 84,
        y + 3,
        x + width - 84,
        y + titlebar_height - 1,
        color::BORDER,
    );
    framebuffer::line(
        x + width - 42,
        y + 3,
        x + width - 42,
        y + titlebar_height - 1,
        color::BORDER,
    );
    let button_y = y + 5;
    let button_height = (titlebar_height - 9).max(14);
    settings_rect(
        preferences,
        x + width - 120,
        button_y,
        32,
        button_height,
        5,
        preferences.card_color(),
    );
    settings_rect(
        preferences,
        x + width - 79,
        button_y,
        32,
        button_height,
        5,
        preferences.card_color(),
    );
    settings_rect(
        preferences,
        x + width - 38,
        button_y,
        30,
        button_height,
        5,
        if focused {
            0x0066_3038
        } else {
            preferences.card_color()
        },
    );
    framebuffer::text(x + width - 109, text_y, "-", color::MUTED, 1);
    framebuffer::outline(x + width - 69, text_y - 1, 12, 8, color::MUTED);
    framebuffer::text(x + width - 28, text_y, "x", color::INK, 1);
}

fn draw_window_border(rect: Rect, focused: bool, preferences: DesktopPreferences) {
    if !preferences.window_borders {
        return;
    }
    let width = rect.width as i32;
    let height = rect.height as i32;
    let radius = if preferences.rounded_controls {
        preferences.window_corner_radius()
    } else {
        0
    };
    for inset in 0..preferences.window_border_width() {
        if width <= inset * 2 || height <= inset * 2 {
            break;
        }
        framebuffer::rounded_outline(
            rect.x as i32 + inset,
            rect.y as i32 + inset,
            width - inset * 2,
            height - inset * 2,
            (radius - inset).max(0),
            preferences.border_color(focused),
        );
    }
}

/// Fill a rectangular app region while clipping its pixels to the configured
/// outer window shape. This keeps app bodies from repainting rounded corners.
#[allow(clippy::too_many_arguments)]
fn fill_window_region(
    rect: Rect,
    preferences: DesktopPreferences,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    value: u32,
) {
    let outer_width = rect.width as i32;
    let outer_height = rect.height as i32;
    let radius = if preferences.rounded_controls {
        preferences
            .window_corner_radius()
            .max(0)
            .min(outer_width / 2)
            .min(outer_height / 2)
    } else {
        0
    };
    let first_row = top.max(0);
    let last_row = top.saturating_add(height).min(outer_height);
    let region_left = left.max(0);
    let region_right = left.saturating_add(width).min(outer_width);
    for row in first_row..last_row {
        let corner_row = if row < radius {
            row
        } else if row >= outer_height - radius {
            outer_height - row - 1
        } else {
            radius
        };
        let shape_inset = if corner_row < radius {
            let dy = radius - corner_row;
            let mut dx = 0;
            while (dx + 1) * (dx + 1) + dy * dy <= radius * radius {
                dx += 1;
            }
            radius - dx
        } else {
            0
        };
        let row_left = region_left.max(shape_inset);
        let row_right = region_right.min(outer_width - shape_inset);
        if row_right > row_left {
            framebuffer::rect(
                rect.x as i32 + row_left,
                rect.y as i32 + row,
                row_right - row_left,
                1,
                value,
            );
        }
    }
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

fn browser_layout(rect: Rect, styled: expos_core::StyledNode<'_>, content_y: i32) -> BrowserLayout {
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
    } else if matches!(
        styled.node.kind,
        NodeKind::Image | NodeKind::Audio | NodeKind::Video
    ) {
        34
    } else if matches!(styled.node.kind, NodeKind::Link | NodeKind::ListItem) {
        18
    } else {
        0
    };
    let x = rect.x as i32 + 34 + margin_left;
    let available_width = (rect.width as i32 - 68 - margin_left - margin_right).max(48);
    let mut width = if button {
        available_width.min(280)
    } else {
        available_width
    };
    if style.max_width != 0 {
        width = width.min(style.max_width as i32);
    }
    let text_width = (width - border * 2 - padding_left - padding_right - prefix).max(24);
    let text_height = browser_wrapped_height(
        styled.node.text.as_str(),
        text_width,
        scale,
        style.line_height as i32,
    );
    let minimum_height = if button {
        30
    } else if matches!(
        styled.node.kind,
        NodeKind::Image | NodeKind::Audio | NodeKind::Video
    ) {
        42
    } else {
        0
    };
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

fn browser_wrapped_height(value: &str, width: i32, scale: i32, requested_line_height: i32) -> i32 {
    let advance = framebuffer::text_advance(scale);
    let line_height = if requested_line_height == 0 {
        if scale <= 1 {
            10
        } else {
            18
        }
    } else {
        requested_line_height.max(if scale <= 1 { 8 } else { 16 })
    };
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

const fn browser_color(value: expos_core::CssColor) -> u32 {
    ((value.red as u32) << 16) | ((value.green as u32) << 8) | value.blue as u32
}

fn browser_tab_width(rect: Rect, count: usize) -> i32 {
    ((rect.width as i32 - 72) / count.max(1) as i32).clamp(72, 180)
}

fn browser_bookmark_width(rect: Rect, count: usize) -> i32 {
    ((rect.width as i32 - 36) / count.max(1) as i32).clamp(58, 140)
}

const fn browser_bookmark_top() -> i32 {
    120
}

fn browser_find_top(desktop: &DesktopState) -> i32 {
    browser_bookmark_top()
        + if desktop.browser_bookmark_count == 0 {
            0
        } else {
            34
        }
}

fn browser_content_top(desktop: &DesktopState) -> i32 {
    browser_find_top(desktop)
        + if desktop.browser_find_len != 0 || desktop.browser_find_editing {
            40
        } else {
            8
        }
}

fn browser_text_contains(value: &str, query: &str) -> bool {
    let value = value.as_bytes();
    let query = query.as_bytes();
    if query.is_empty() || query.len() > value.len() {
        return false;
    }
    value.windows(query.len()).any(|candidate| {
        candidate
            .iter()
            .zip(query)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

fn browser_text_prefix(value: &str, capacity: usize) -> &str {
    let end = value
        .char_indices()
        .nth(capacity)
        .map_or(value.len(), |(index, _)| index);
    &value[..end]
}

fn browser_find_stats(desktop: &DesktopState) -> (usize, usize) {
    if desktop.browser_find_len == 0 {
        return (0, 0);
    }
    let query =
        core::str::from_utf8(&desktop.browser_find_line[..desktop.browser_find_len]).unwrap_or("");
    let mut count = 0;
    let mut ordinal = 0;
    for styled in desktop.document.styled_nodes() {
        if styled.style.is_rendered() && browser_text_contains(styled.node.text.as_str(), query) {
            count += 1;
            if desktop.browser_find_match == Some(styled.index) {
                ordinal = count;
            }
        }
    }
    (ordinal, count)
}

fn draw_browser(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let bottom = y + rect.height as i32;
    let tab_width = browser_tab_width(rect, desktop.browser_tab_count);
    for index in 0..desktop.browser_tab_count {
        let left = x + 18 + index as i32 * tab_width;
        let active = index == desktop.browser_active_tab;
        framebuffer::rounded_rect(
            left,
            y + 42,
            tab_width - 2,
            30,
            7,
            if active {
                desktop.preferences.panel_color()
            } else {
                0x000E_141B
            },
        );
        framebuffer::rounded_outline(
            left,
            y + 42,
            tab_width - 2,
            30,
            7,
            if active {
                desktop.preferences.accent.color()
            } else {
                desktop.preferences.border_color(false)
            },
        );
        let capacity = ((tab_width - 38) / framebuffer::text_advance(1)).max(1) as usize;
        framebuffer::text(
            left + 10,
            y + 53,
            browser_text_prefix(desktop.browser_tabs[index].title(), capacity),
            if active { color::INK } else { color::MUTED },
            1,
        );
        if desktop.browser_tab_count > 1 {
            framebuffer::text(left + tab_width - 20, y + 53, "x", color::MUTED, 1);
        }
    }
    let new_tab_left = x + 18 + desktop.browser_tab_count as i32 * tab_width;
    if desktop.browser_tab_count < BROWSER_TAB_CAPACITY && new_tab_left + 30 < x + width - 10 {
        framebuffer::rounded_rect(new_tab_left, y + 42, 30, 30, 7, 0x0019_2028);
        framebuffer::text(new_tab_left + 11, y + 53, "+", color::INK, 1);
    }
    let active_tab = &desktop.browser_tabs[desktop.browser_active_tab];
    for (offset, label, enabled) in [
        (18, "<", active_tab.history_cursor > 0),
        (
            56,
            ">",
            active_tab.history_cursor + 1 < active_tab.history_count,
        ),
        (94, "R", true),
        (132, "H", true),
    ] {
        framebuffer::rounded_rect(
            x + offset,
            y + 78,
            36,
            36,
            6,
            if enabled { 0x0019_2028 } else { 0x000D_1116 },
        );
        framebuffer::outline(
            x + offset,
            y + 78,
            36,
            36,
            desktop.preferences.border_color(false),
        );
        framebuffer::text(
            x + offset + 14,
            y + 92,
            label,
            if enabled { color::INK } else { color::BORDER },
            1,
        );
    }
    framebuffer::rounded_rect(
        x + 176,
        y + 78,
        width - 276,
        36,
        7,
        desktop.preferences.panel_color(),
    );
    framebuffer::outline(
        x + 176,
        y + 78,
        width - 276,
        36,
        if desktop.browser_editing {
            color::CYAN
        } else {
            desktop.preferences.border_color(false)
        },
    );
    framebuffer::rect(
        x + 188,
        y + 93,
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
    let scheme = if address.starts_with("https://") {
        "TLS"
    } else if address.starts_with("http://") {
        "WEB"
    } else {
        "FORM"
    };
    framebuffer::text(x + 200, y + 92, scheme, color::MUTED, 1);
    let address_capacity = ((width - 336) / framebuffer::text_advance(1)).max(1) as usize;
    let (address_text, address_color) = if desktop.browser_editing && address.is_empty() {
        ("Search DuckDuckGo or enter an address", color::MUTED)
    } else if desktop.browser_editing && address.len() > address_capacity {
        (&address[address.len() - address_capacity..], color::INK)
    } else if address.len() > address_capacity {
        (&address[..address_capacity], color::INK)
    } else {
        (address, color::INK)
    };
    framebuffer::text(x + 236, y + 92, address_text, address_color, 1);
    if desktop.browser_editing {
        let caret_x =
            (x + 236 + address.len().min(address_capacity) as i32 * framebuffer::text_advance(1))
                .min(x + width - 106);
        framebuffer::rect(caret_x, y + 89, 2, 14, color::GREEN);
    }
    framebuffer::rounded_rect(x + width - 94, y + 78, 36, 36, 6, 0x0019_2028);
    framebuffer::text(
        x + width - 81,
        y + 92,
        "F",
        if desktop.browser_find_editing {
            color::CYAN
        } else {
            color::MUTED
        },
        1,
    );
    let bookmarked = desktop
        .browser_bookmark_index(desktop.document.url())
        .is_some();
    framebuffer::rounded_rect(
        x + width - 52,
        y + 78,
        34,
        36,
        6,
        if bookmarked { 0x0036_2A18 } else { 0x0019_2028 },
    );
    framebuffer::text(
        x + width - 41,
        y + 92,
        if bookmarked { "*" } else { "+" },
        if bookmarked {
            0x00F0_C46B
        } else {
            color::MUTED
        },
        1,
    );
    if desktop.browser_bookmark_count != 0 {
        let top = y + browser_bookmark_top();
        let item_width = browser_bookmark_width(rect, desktop.browser_bookmark_count);
        for index in 0..desktop.browser_bookmark_count {
            let left = x + 18 + index as i32 * item_width;
            framebuffer::rounded_rect(left, top, item_width - 4, 28, 5, 0x0012_1920);
            let capacity = ((item_width - 20) / framebuffer::text_advance(1)).max(1) as usize;
            framebuffer::text(
                left + 8,
                top + 10,
                browser_text_prefix(desktop.browser_bookmarks[index].title(), capacity),
                color::MUTED,
                1,
            );
        }
    }
    if desktop.browser_find_len != 0 || desktop.browser_find_editing {
        let top = y + browser_find_top(desktop);
        framebuffer::rounded_rect(x + width - 310, top, 292, 32, 6, 0x0019_2028);
        framebuffer::rounded_outline(
            x + width - 310,
            top,
            292,
            32,
            6,
            if desktop.browser_find_editing {
                color::CYAN
            } else {
                color::BORDER
            },
        );
        let query = core::str::from_utf8(&desktop.browser_find_line[..desktop.browser_find_len])
            .unwrap_or("");
        framebuffer::text(
            x + width - 296,
            top + 11,
            if query.is_empty() {
                "Find in page"
            } else {
                query
            },
            if query.is_empty() {
                color::MUTED
            } else {
                color::INK
            },
            1,
        );
        let (ordinal, count) = browser_find_stats(desktop);
        draw_number(x + width - 100, top + 11, ordinal as u64, color::MUTED);
        framebuffer::text(x + width - 84, top + 11, "/", color::MUTED, 1);
        draw_number(x + width - 72, top + 11, count as u64, color::MUTED);
        framebuffer::text(x + width - 38, top + 11, "x", color::MUTED, 1);
    }
    let mut content_y = y + browser_content_top(desktop) - desktop.browser_scroll;
    for styled in desktop.document.styled_nodes() {
        if content_y > bottom - 46 {
            break;
        }
        if styled.node.kind == NodeKind::Title || !styled.style.is_rendered() {
            continue;
        }
        let layout = browser_layout(rect, styled, content_y);
        if layout.y > bottom - 46 {
            break;
        }
        let button = styled.tag.eq_ignore_ascii_case("button");
        let radius = if styled.style.border_radius == 0 && button {
            5
        } else {
            styled.style.border_radius.min(32) as i32
        };
        if styled.style.background.alpha != 0 || button {
            let background = if styled.style.background.alpha == 0 {
                desktop.preferences.accent.color()
            } else {
                browser_color(styled.style.background)
            };
            if radius != 0 && styled.style.background.alpha == u8::MAX {
                framebuffer::rounded_rect(
                    layout.x,
                    layout.y,
                    layout.width,
                    layout.height,
                    radius,
                    background,
                );
            } else if radius != 0 {
                framebuffer::alpha_rounded_rect(
                    layout.x,
                    layout.y,
                    layout.width,
                    layout.height,
                    radius,
                    background,
                    styled.style.background.alpha,
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
        if desktop.browser_find_match == Some(styled.index) {
            framebuffer::alpha_rounded_rect(
                layout.x,
                layout.y,
                layout.width,
                layout.height,
                5,
                0x00F0_C46B,
                88,
            );
        }
        let border_width = styled.style.border.width.min(4) as i32;
        for inset in 0..border_width {
            if radius != 0 {
                framebuffer::rounded_outline(
                    layout.x + inset,
                    layout.y + inset,
                    layout.width - inset * 2,
                    layout.height - inset * 2,
                    (radius - inset).max(1),
                    browser_color(styled.style.border.color),
                );
            } else {
                framebuffer::outline(
                    layout.x + inset,
                    layout.y + inset,
                    layout.width - inset * 2,
                    layout.height - inset * 2,
                    browser_color(styled.style.border.color),
                );
            }
        }
        match styled.node.kind {
            NodeKind::Title => {}
            NodeKind::ListItem => {
                framebuffer::rect(layout.x + 5, layout.text_y + 3, 5, 5, color::GREEN);
            }
            NodeKind::Link if !button => {
                framebuffer::text(layout.x + 3, layout.text_y, ">", color::GREEN, 1);
            }
            NodeKind::Image => {
                if desktop.browser_media.image_width != 0
                    && styled.node.target == desktop.browser_media.image_url
                {
                    draw_browser_image(layout.x + 5, layout.y + 5, 28, 28, &desktop.browser_media);
                } else {
                    framebuffer::outline(layout.x + 7, layout.y + 7, 22, 18, color::MUTED);
                    framebuffer::line(
                        layout.x + 9,
                        layout.y + 22,
                        layout.x + 17,
                        layout.y + 14,
                        color::MUTED,
                    );
                    framebuffer::line(
                        layout.x + 17,
                        layout.y + 14,
                        layout.x + 27,
                        layout.y + 22,
                        color::MUTED,
                    );
                }
            }
            NodeKind::Audio => {
                framebuffer::outline(layout.x + 7, layout.y + 7, 24, 18, color::GREEN);
                framebuffer::text(layout.x + 14, layout.y + 12, ">", color::WHITE, 1);
            }
            NodeKind::Video => {
                framebuffer::outline(layout.x + 7, layout.y + 7, 26, 18, color::CYAN);
                framebuffer::text(layout.x + 16, layout.y + 12, ">", color::WHITE, 1);
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
    framebuffer::rect(x + 1, bottom - 28, width - 2, 27, 0x000B_1016);
    framebuffer::line(
        x + 1,
        bottom - 28,
        x + width - 1,
        bottom - 28,
        color::BORDER,
    );
    let status_capacity = ((width - 360) / framebuffer::text_advance(1)).max(1) as usize;
    framebuffer::text(
        x + 18,
        bottom - 18,
        browser_text_prefix(desktop.document.title(), status_capacity),
        color::INK,
        1,
    );
    framebuffer::text(
        x + width - 190,
        bottom - 18,
        if desktop.document.url().starts_with("https://") {
            "TLS VERIFIED"
        } else if desktop.document.url().starts_with("http://") {
            "HTTP DOCUMENT"
        } else {
            "LOCAL FORM"
        },
        if desktop.document.url().starts_with("https://") {
            color::GREEN
        } else {
            color::MUTED
        },
        1,
    );
    if desktop.document.rejected_loads != 0 {
        framebuffer::text(x + width - 310, bottom - 18, "BOX RECOVERED", color::RED, 1);
    } else if desktop.browser_scroll > 0 {
        framebuffer::text(x + width - 270, bottom - 18, "SCROLLED", color::CYAN, 1);
    } else {
        framebuffer::text(x + width - 270, bottom - 18, "BOXED PAGE", color::MUTED, 1);
    }
}

fn draw_browser_image(x: i32, y: i32, width: i32, height: i32, image: &BrowserMediaCache) {
    let source_width = image.image_width as usize;
    let source_height = image.image_height as usize;
    if source_width == 0 || source_height == 0 {
        return;
    }
    for output_y in 0..height {
        let source_y = output_y as usize * source_height / height as usize;
        for output_x in 0..width {
            let source_x = output_x as usize * source_width / width as usize;
            framebuffer::pixel(
                x + output_x,
                y + output_y,
                image.image_pixels[source_y * 64 + source_x],
            );
        }
    }
}

fn draw_terminal(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    let titlebar_height = desktop.preferences.titlebar_height() as i32;
    let terminal_background = match desktop.preferences.terminal_background {
        1 => 0x0015_171B,
        2 => 0x0006_170F,
        3 => 0x0005_1419,
        4 => 0x001B_1405,
        5 => 0x001B_0911,
        6 => 0x0008_1020,
        7 => 0x0000_0000,
        _ => desktop.preferences.theme.terminal(),
    };
    let terminal_foreground = match desktop.preferences.terminal_foreground {
        1 => color::WHITE,
        2 => color::GREEN,
        3 => color::CYAN,
        4 => 0x00F0_C46B,
        5 => 0x00F0_8AA4,
        6 => 0x008E_B8FF,
        7 => 0x0030_343A,
        _ => color::INK,
    };
    let previous_style = framebuffer::font_style();
    framebuffer::set_font_style(framebuffer::FontStyle::new(
        desktop.preferences.terminal_font_face,
        desktop.preferences.terminal_font_weight,
    ));
    fill_window_region(
        rect,
        desktop.preferences,
        1,
        titlebar_height,
        width - 2,
        height - titlebar_height - 1,
        terminal_background,
    );
    let prompt_y = y + height - 34;
    let output_top = y + titlebar_height + 16;
    let text_scale = desktop.preferences.terminal_scale as i32 + 1;
    let line_height = if text_scale == 1 { 12 } else { 20 };
    let visible_lines = ((prompt_y - output_top - 8) / line_height).max(0) as usize;
    let lines = visible_lines.min(desktop.terminal_output_count);
    for visual_index in 0..lines {
        let reverse_index = lines - visual_index - 1;
        if let Some(line) = desktop.terminal_output_entry(reverse_index) {
            let visible_length = line
                .len()
                .min(((width - 40) / framebuffer::text_advance(text_scale)).max(1) as usize);
            framebuffer::text(
                x + 20,
                output_top + visual_index as i32 * line_height,
                &line[..visible_length],
                if line.starts_with("$ ") {
                    desktop.preferences.accent.color()
                } else {
                    terminal_foreground
                },
                text_scale,
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
    let prompt_scale = text_scale;
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
        framebuffer::text(
            input_x,
            prompt_y,
            visible,
            terminal_foreground,
            prompt_scale,
        );
        framebuffer::rect(
            input_x + visible.len() as i32 * advance,
            prompt_y,
            if prompt_scale == 1 { 5 } else { 8 },
            if prompt_scale == 1 { 9 } else { 16 },
            color::MUTED,
        );
    }
    framebuffer::set_font_style(previous_style);
}

fn draw_notes(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(x + 12, y + 44, width - 24, 42, 0x000A_0D13);
    framebuffer::rect(x + 18, y + 50, width - 238, 30, 0x0012_171D);
    framebuffer::outline(
        x + 18,
        y + 50,
        width - 238,
        30,
        if desktop.notes_editing_name {
            color::PURPLE
        } else {
            color::BORDER
        },
    );
    let name = core::str::from_utf8(&desktop.notes_name[..desktop.notes_name_len])
        .unwrap_or("Untitled.txt");
    framebuffer::text(x + 30, y + 61, name, color::WHITE, 1);
    if desktop.notes_editing_name {
        framebuffer::rect(
            x + 31 + name.len() as i32 * framebuffer::text_advance(1),
            y + 58,
            2,
            14,
            color::PURPLE,
        );
    }
    framebuffer::rounded_rect(x + width - 204, y + 50, 92, 30, 5, 0x0020_2730);
    framebuffer::text(x + width - 173, y + 61, "NEW", color::INK, 1);
    framebuffer::rounded_rect(x + width - 100, y + 50, 82, 30, 5, color::PURPLE);
    framebuffer::text(x + width - 76, y + 61, "SAVE", color::WHITE, 1);
    framebuffer::rect(x + 12, y + 88, width - 24, height - 146, 0x0000_0000);
    framebuffer::outline(x + 12, y + 88, width - 24, height - 146, color::BORDER);

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
    framebuffer::text(
        x + 18,
        y + height - 42,
        desktop.notes_status,
        color::MUTED,
        1,
    );
    framebuffer::text(
        x + width - 154,
        y + height - 42,
        "EXPFS DATA FORM",
        color::CYAN,
        1,
    );
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
        ("EXPOSDISPLAY", "SERVICE", "ACTIVE", color::GREEN),
        ("BROWSER", "INTERFACE", "FOCUSED", color::PURPLE),
        ("GO ABI V1", "INTERFACE", "READY", color::CYAN),
        ("EXPFS", "STORAGE", "JOURNALED", color::GREEN),
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

#[derive(Clone, Copy)]
enum SettingControl {
    Toggle { on: bool, available: bool },
    Choice,
    Palette { id: u8, color: u32 },
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
    let titlebar_height = desktop.preferences.titlebar_height() as i32;

    fill_window_region(
        rect,
        desktop.preferences,
        1,
        titlebar_height,
        sidebar_width,
        height - titlebar_height - 1,
        desktop.preferences.chrome_color(),
    );
    framebuffer::line(
        x + sidebar_width,
        y + titlebar_height,
        x + sidebar_width,
        y + height - 1,
        desktop.preferences.border_color(false),
    );
    framebuffer::text(
        x + 20,
        y + if compact { 44 } else { 54 },
        desktop.locale().text(LocalText::Settings),
        color::INK,
        if compact { 1 } else { 2 },
    );

    let category_start = settings_category_view_start(rect, desktop.settings_category.index());
    let category_end =
        (category_start + settings_category_capacity(rect)).min(SETTINGS_CATEGORY_COUNT);
    for category in SettingsCategory::ALL[category_start..category_end]
        .iter()
        .copied()
    {
        let slot = category.index() - category_start;
        let row_y = y + category_top + slot as i32 * category_step;
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
            desktop.locale().category(category.index()),
            if selected { color::WHITE } else { color::MUTED },
            1,
        );
    }
    if category_start > 0 {
        framebuffer::text(x + sidebar_width - 24, y + 48, "^", accent, 1);
    }
    if category_end < SETTINGS_CATEGORY_COUNT {
        framebuffer::text(x + sidebar_width - 24, y + height - 18, "+", accent, 1);
    }

    let content_x = x + sidebar_width + 28;
    let content_width = width - sidebar_width - 50;
    framebuffer::text(
        content_x,
        y + if compact { 45 } else { 55 },
        desktop.locale().category(desktop.settings_category.index()),
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
        SettingsCategory::Profiles => {
            for (row, profile) in CustomizationProfile::ALL.iter().copied().enumerate() {
                settings_row(
                    desktop,
                    content_x,
                    y,
                    content_width,
                    row,
                    profile.label(),
                    profile.description(),
                    "Apply",
                    SettingControl::Action { available: true },
                );
            }
        }
        SettingsCategory::Appearance => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Theme palette",
                "Six named plus 250 procedural palettes; ,/. jumps 16",
                desktop.preferences.theme.label(),
                SettingControl::Palette {
                    id: desktop.preferences.theme.persisted(),
                    color: desktop.preferences.theme.panel(),
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Wallpaper variant",
                "252 persisted pattern and spectrum combinations; ,/. jumps 16",
                desktop.preferences.wallpaper.label(),
                SettingControl::Palette {
                    id: desktop.preferences.wallpaper_variant,
                    color: desktop.preferences.wallpaper_color(),
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Accent color",
                "256 persisted focus and control colors; ,/. jumps 16",
                desktop.preferences.accent.label(),
                SettingControl::Palette {
                    id: desktop.preferences.accent.persisted(),
                    color: desktop.preferences.accent.color(),
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Wallpaper tone",
                "Three named plus 253 procedural base tones; ,/. jumps 16",
                desktop.preferences.backdrop.label(),
                SettingControl::Palette {
                    id: desktop.preferences.backdrop.persisted(),
                    color: desktop.preferences.backdrop.color(),
                },
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
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                6,
                "Font face",
                "Change the built-in allocation-free bitmap rasterizer",
                state::FONT_FACE_NAMES[desktop.preferences.font_face.persisted() as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                7,
                "Font weight",
                "Light, regular, or bold strokes without changing layout",
                state::FONT_WEIGHT_NAMES[desktop.preferences.font_weight.persisted() as usize],
                SettingControl::Choice,
            );
        }
        SettingsCategory::Accessibility => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "High contrast",
                "Strengthen edges and active-window focus",
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
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Font face",
                "Choose the most readable bitmap letter shapes",
                state::FONT_FACE_NAMES[desktop.preferences.font_face.persisted() as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Font weight",
                "Adjust stroke weight without changing text layout",
                state::FONT_WEIGHT_NAMES[desktop.preferences.font_weight.persisted() as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Titlebar target",
                "Enlarge the draggable area and window controls",
                TITLEBAR_LABELS[desktop.preferences.titlebar_density as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
                "Taskbar target",
                "Enlarge launcher and running-application controls",
                TASKBAR_SIZE_LABELS[desktop.preferences.taskbar_size as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                5,
                "Pointer shape",
                "Choose arrows or a precise crosshair",
                desktop.preferences.cursor.label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                6,
                "Pointer shadow",
                "Separate the pointer from similarly colored content",
                if desktop.preferences.cursor_shadow {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.cursor_shadow,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                7,
                "Motion",
                "Turn motion off or choose a comfortable duration",
                ANIMATION_LABELS[desktop.preferences.animation_level as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                8,
                "Interface hints",
                "Show keyboard and application guidance",
                if desktop.preferences.tooltips {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.tooltips,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                9,
                "Reduce transparency",
                "Force opaque windows and panels for easier reading",
                if desktop.preferences.reduce_transparency {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.reduce_transparency,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                10,
                "Focus ring",
                "Use the selected accent around the active window",
                if desktop.preferences.focus_ring {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.focus_ring,
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
                "Double-buffered XRGB8888 ExpDisplay composition",
                framebuffer::active_output_label(),
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
        SettingsCategory::Audio => {
            let status = audio::status();
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Output volume",
                "Master PCM level for system and browser audio",
                AUDIO_VOLUME_LABELS[(desktop.preferences.audio_volume / 5) as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Mute",
                "Silence the hardware mixer without losing volume",
                if desktop.preferences.audio_muted {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.audio_muted,
                    available: audio::available(),
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Test output",
                "Play a short generated 48 kHz stereo PCM tone",
                "Play",
                SettingControl::Action {
                    available: audio::available(),
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Stop playback",
                "Halt the active DMA stream immediately",
                "Stop",
                SettingControl::Action {
                    available: status.playing,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
                "Output backend",
                "ExpAudio hardware path and current stream state",
                status.backend.label(),
                SettingControl::Status {
                    ready: status.backend != audio::Backend::Unavailable,
                },
            );
        }
        SettingsCategory::Performance => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Window shadows",
                "Blend a soft offset behind windows during full repaints",
                if desktop.preferences.window_shadows {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.window_shadows,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Wallpaper effects",
                "Render the selected procedural wallpaper instead of a solid fill",
                if desktop.preferences.wallpaper_effects {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.wallpaper_effects,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Presentation policy",
                "Efficient pacing or lower-latency damaged commits",
                desktop.preferences.presentation_policy_label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Damage tracking",
                "Repaint only changed regions for cursors and active windows",
                "Active",
                SettingControl::Status { ready: true },
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
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
                "Cursor shadow",
                "Add a low-cost two-pixel pointer shadow",
                if desktop.preferences.cursor_shadow {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.cursor_shadow,
                    available: true,
                },
            );
        }
        SettingsCategory::Windows => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Corner radius",
                "Square through strongly rounded window geometry",
                CORNER_RADIUS_LABELS[desktop.preferences.window_corner_radius as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Border width",
                "Independent window outline thickness",
                BORDER_WIDTH_LABELS[desktop.preferences.window_border_width as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Titlebar size",
                "Compact through tall draggable window chrome",
                TITLEBAR_LABELS[desktop.preferences.titlebar_density as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Backdrop opacity",
                "Blend the window backdrop and titlebar over the desktop",
                WINDOW_OPACITY_LABELS[desktop.preferences.window_opacity as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
                "Off-screen travel",
                "Allow windows beyond an edge while retaining recovery space",
                OFFSCREEN_LABELS[desktop.preferences.window_offscreen_allowance as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                5,
                "Edge snapping",
                "Snap dragged windows to the current work area",
                if desktop.preferences.window_snap {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.window_snap,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                6,
                "Snap distance",
                "Choose the edge attraction distance",
                SNAP_DISTANCE_LABELS[desktop.preferences.window_snap_distance as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                7,
                "Focus policy",
                "Click focus, calm hover, or continuous pointer focus",
                desktop.preferences.focus_policy_label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                8,
                "Keyboard tiling",
                "Choose halves, thirds, or quarters for Super+Alt+Arrow",
                WINDOW_TILING_LABELS[desktop.preferences.window_tiling as usize],
                SettingControl::Choice,
            );
        }
        SettingsCategory::Taskbar => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Placement",
                "Attach the taskbar to any display edge",
                desktop.preferences.taskbar_placement.label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Size",
                "Panel thickness from compact to touch friendly",
                TASKBAR_SIZE_LABELS[desktop.preferences.taskbar_size as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "App alignment",
                "Place running applications at start, center, or end",
                desktop.preferences.taskbar_alignment.label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Auto-hide",
                "Reveal the taskbar by touching its configured edge",
                if desktop.preferences.taskbar_autohide {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.taskbar_autohide,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
                "Translucent panel",
                "Blend the panel with the wallpaper",
                if desktop.preferences.taskbar_translucent {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.taskbar_translucent,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                5,
                "Application labels",
                "Show names alongside icons on horizontal panels",
                if desktop.preferences.taskbar_labels {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.taskbar_labels,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                6,
                "Clock seconds",
                "Show seconds in the hardware RTC taskbar clock",
                if desktop.preferences.clock_seconds {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.clock_seconds,
                    available: true,
                },
            );
            for (row, label, description, flag) in [
                (
                    7,
                    "Date widget",
                    "Show the selected regional date beside the clock",
                    TASKBAR_WIDGET_DATE,
                ),
                (
                    8,
                    "Active app widget",
                    "Show the focused application name",
                    TASKBAR_WIDGET_ACTIVE_APP,
                ),
                (
                    9,
                    "Weather widget",
                    "Show the latest Weather app temperature",
                    TASKBAR_WIDGET_WEATHER,
                ),
                (
                    10,
                    "Performance widget",
                    "Show the selected compositor frame target",
                    TASKBAR_WIDGET_PERFORMANCE,
                ),
                (
                    11,
                    "Audio widget",
                    "Show mute or active PCM playback state",
                    TASKBAR_WIDGET_AUDIO,
                ),
                (
                    12,
                    "Time zone widget",
                    "Show the selected UTC offset beside the clock",
                    TASKBAR_WIDGET_TIMEZONE,
                ),
            ] {
                settings_row(
                    desktop,
                    content_x,
                    y,
                    content_width,
                    row,
                    label,
                    description,
                    if desktop.preferences.taskbar_widgets & flag != 0 {
                        "On"
                    } else {
                        "Off"
                    },
                    SettingControl::Toggle {
                        on: desktop.preferences.taskbar_widgets & flag != 0,
                        available: !desktop.preferences.taskbar_placement.vertical(),
                    },
                );
            }
        }
        SettingsCategory::Menu => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Layout",
                "List, grid, compact grid, or dashboard",
                MENU_LAYOUT_LABELS[desktop.preferences.menu_layout as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Density",
                "Five row heights from dense to touch friendly",
                MENU_DENSITY_LABELS[desktop.preferences.menu_density as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Icon scale",
                "Seven persisted launcher scale choices",
                UI_SCALE_LABELS[desktop.preferences.ui_scale as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Built-in apps",
                "Show ExpOS system applications in the launcher",
                if desktop.preferences.menu_show_builtins {
                    "Shown"
                } else {
                    "Hidden"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.menu_show_builtins,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
                "Installed Ayo apps",
                "Put each installed app directly in the launcher",
                if desktop.preferences.menu_show_installed {
                    "Shown"
                } else {
                    "Hidden"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.menu_show_installed,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                5,
                "Category labels",
                "Show Utilities, Graphics, Networking, Editors, and Developer tools",
                if desktop.preferences.menu_categories {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.menu_categories,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                6,
                "Animation speed",
                "Five persisted motion levels; Off is the fastest",
                ANIMATION_LABELS[desktop.preferences.animation_level as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                7,
                "Tooltips",
                "Show launcher hints for keyboard and app categories",
                if desktop.preferences.tooltips {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.tooltips,
                    available: true,
                },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                8,
                "Install feedback",
                "Animate package-install feedback when motion is enabled",
                if desktop.preferences.notification_animations {
                    "On"
                } else {
                    "Off"
                },
                SettingControl::Toggle {
                    on: desktop.preferences.notification_animations,
                    available: true,
                },
            );
        }
        SettingsCategory::Terminal => {
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Font",
                "Independent command-shell typeface",
                state::FONT_FACE_NAMES[desktop.preferences.terminal_font_face.persisted() as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Weight",
                "Light, regular, or bold shell text",
                state::FONT_WEIGHT_NAMES
                    [desktop.preferences.terminal_font_weight.persisted() as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Text size",
                "Shell-only text scale",
                TERMINAL_SCALE_LABELS[desktop.preferences.terminal_scale as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Text color",
                "Readable foreground palette",
                TERMINAL_COLOR_LABELS[desktop.preferences.terminal_foreground as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
                "Background",
                "Independent shell background palette",
                TERMINAL_COLOR_LABELS[desktop.preferences.terminal_background as usize],
                SettingControl::Choice,
            );
        }
        SettingsCategory::Language => {
            for (row, language) in Locale::ALL.iter().map(|locale| locale.name()).enumerate() {
                settings_row(
                    desktop,
                    content_x,
                    y,
                    content_width,
                    row,
                    language,
                    if Locale::ALL[row].is_rtl() {
                        desktop.locale().text(LocalText::RtlInterface)
                    } else {
                        desktop.locale().text(LocalText::SystemInstallerLanguage)
                    },
                    if desktop.preferences.locale as usize == row {
                        desktop.locale().text(LocalText::Selected)
                    } else {
                        desktop.locale().text(LocalText::Available)
                    },
                    SettingControl::Choice,
                );
            }
        }
        SettingsCategory::Time => {
            let mut offset_buffer = [0_u8; 16];
            let offset_label = format_signed_minutes(
                desktop.preferences.clock_offset_quarters as i16 * 15,
                &mut offset_buffer,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                0,
                "Time zone",
                "Apply a real regional UTC offset to the hardware clock",
                TIMEZONE_LABELS[desktop.preferences.timezone as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Clock format",
                "Use a 12-hour clock with AM/PM or a 24-hour clock",
                if desktop.preferences.clock_24h {
                    "24 hour"
                } else {
                    "12 hour"
                },
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "Manual time correction",
                "Adjust displayed local time in 15-minute steps",
                offset_label,
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Date format",
                "Choose international, day-first, or month-first dates",
                DATE_FORMAT_LABELS[desktop.preferences.date_format as usize],
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
                "Week starts",
                "Choose the first day used by calendars",
                if desktop.preferences.week_starts_monday {
                    "Monday"
                } else {
                    "Sunday"
                },
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                5,
                "Reset correction",
                "Return the manual display correction to zero",
                "Reset",
                SettingControl::Action { available: true },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                6,
                "Hardware clock",
                "CMOS remains the trusted base; regional changes are non-destructive",
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
                "Reset tabs, histories, cookies, storage, media, and the current document",
                "Clear",
                SettingControl::Action { available: true },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Clear weather data",
                "Remove the saved city, coordinates, and cached forecast",
                "Clear",
                SettingControl::Action { available: true },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
                "App network isolation",
                "Weather is restricted to two exact HTTPS provider hosts",
                "Enforced",
                SettingControl::Status { ready: true },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                3,
                "Request throttling",
                "Weather refreshes are rate-limited before any DNS or TCP work",
                "3 seconds",
                SettingControl::Status { ready: true },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                4,
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
                "ExpDisplay",
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
    let view_start =
        settings_row_view_start(desktop.settings_category, desktop.settings_row, width);
    let view_end = view_start + settings_row_capacity(width);
    if index < view_start || index >= view_end {
        return;
    }
    let slot = index - view_start;
    let y = window_y + row_top + slot as i32 * row_step;
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
        SettingControl::Palette { id, color } => {
            let control_width = 154;
            let control_x = x + width - control_width - 20;
            let control_y = y + if compact { 6 } else { 12 };
            settings_rect(
                desktop.preferences,
                control_x,
                control_y,
                control_width,
                30,
                5,
                0x0018_1D21,
            );
            framebuffer::outline(control_x, control_y, control_width, 30, color::BORDER);
            framebuffer::text(control_x + 10, control_y + 10, "<", color::MUTED, 1);
            framebuffer::rect(control_x + 30, control_y + 7, 16, 16, color);
            framebuffer::outline(control_x + 30, control_y + 7, 16, 16, color::WHITE);
            framebuffer::text(control_x + 56, control_y + 10, "#", color::MUTED, 1);
            draw_number(control_x + 68, control_y + 10, id as u64, color::INK);
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
        "Portal v3",
        color::GREEN,
    );
    metric(
        right_x,
        y + 98,
        metric_width,
        "Framebuffer",
        framebuffer::active_output_label(),
        color::CYAN,
    );
    metric(
        x + 28,
        y + 174,
        metric_width,
        "Surfaces",
        "12 Form-owned",
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
    framebuffer::text(
        x + 430,
        y + 343,
        desktop.active.localized_label(desktop.locale()),
        color::CYAN,
        1,
    );
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
    let dock = taskbar_rect(desktop.preferences);
    let x = dock.x as i32;
    let y = dock.y as i32;
    let width = dock.width as i32;
    let height = dock.height as i32;
    let accent = desktop.preferences.accent.color();
    if desktop.preferences.taskbar_translucent && !desktop.preferences.reduce_transparency {
        framebuffer::alpha_rect(x, y, width, height, desktop.preferences.panel_color(), 204);
    } else {
        framebuffer::rect(x, y, width, height, desktop.preferences.panel_color());
    }
    framebuffer::outline(x, y, width, height, desktop.preferences.border_color(false));
    let start_color = if desktop.launcher_open {
        0x0032_2654
    } else {
        0x0017_1B25
    };
    let start = taskbar_start_rect(desktop.preferences);
    settings_rect(
        desktop.preferences,
        start.x as i32,
        start.y as i32,
        start.width as i32,
        start.height as i32,
        7,
        start_color,
    );
    let logo_x = start.x as i32 + (start.width as i32 - 16) / 2;
    let logo_y = start.y as i32 + (start.height as i32 - 16) / 2;
    settings_rect(desktop.preferences, logo_x, logo_y, 16, 16, 4, accent);
    framebuffer::text(logo_x + 4, logo_y + 4, "E", color::WHITE, 1);

    let running_count = desktop.app_open.iter().filter(|open| **open).count();
    let mut running_index = 0_usize;
    for app in AppKind::ALL {
        if !desktop.app_open[app.index()] {
            continue;
        }
        let item = taskbar_app_rect(desktop.preferences, running_index, running_count);
        let item_x = item.x as i32;
        let item_y = item.y as i32;
        if app == desktop.active && desktop.app_is_visible(app) {
            settings_rect(
                desktop.preferences,
                item_x,
                item_y,
                item.width as i32,
                item.height as i32,
                7,
                0x0024_292F,
            );
        }
        let icon_x = item_x + 5;
        let icon_y = item_y + (item.height as i32 - 18) / 2;
        settings_rect(desktop.preferences, icon_x, icon_y, 18, 18, 4, app.accent());
        framebuffer::text(icon_x + 5, icon_y + 4, app.shortcut(), color::WHITE, 1);
        if desktop.preferences.taskbar_labels
            && !desktop.preferences.taskbar_placement.vertical()
            && item.width >= 48
        {
            let capacity =
                ((item.width as i32 - 34) / framebuffer::text_advance(1)).max(0) as usize;
            let label = app.localized_label(desktop.locale());
            if capacity > 0 {
                framebuffer::text(
                    item_x + 30,
                    item_y + (item.height as i32 - 8) / 2,
                    browser_text_prefix(label, capacity),
                    color::INK,
                    1,
                );
            }
        }
        let indicator_color = if desktop.app_minimized[app.index()] {
            color::MUTED
        } else {
            accent
        };
        if desktop.preferences.taskbar_placement.vertical() {
            framebuffer::rect(
                item_x,
                item_y + 4,
                2,
                item.height as i32 - 8,
                indicator_color,
            );
        } else {
            framebuffer::rect(
                item_x + 5,
                item_y + item.height as i32 - 2,
                18,
                2,
                indicator_color,
            );
        }
        running_index += 1;
    }
    if desktop.preferences.status_visible {
        let connectivity = radio::snapshot();
        let status_color = if connectivity.network_enabled && connectivity.ethernet.connected() {
            accent
        } else {
            color::MUTED
        };
        let clock = adjusted_time(desktop.preferences);
        if desktop.preferences.taskbar_placement.vertical() {
            let clock_y = y + height
                - if desktop.preferences.clock_seconds {
                    64
                } else {
                    46
                };
            let mut hour = clock.hour;
            if !desktop.preferences.clock_24h {
                hour %= 12;
                if hour == 0 {
                    hour = 12;
                }
            }
            draw_two_digits(x + (width - 12) / 2, clock_y, hour, color::INK);
            draw_two_digits(x + (width - 12) / 2, clock_y + 16, clock.minute, color::INK);
            if desktop.preferences.clock_seconds {
                draw_two_digits(
                    x + (width - 12) / 2,
                    clock_y + 32,
                    clock.second,
                    color::MUTED,
                );
            }
            if !desktop.preferences.clock_24h {
                framebuffer::text(
                    x + (width - 6) / 2,
                    clock_y
                        + if desktop.preferences.clock_seconds {
                            46
                        } else {
                            32
                        },
                    if clock.hour >= 12 { "P" } else { "A" },
                    color::MUTED,
                    1,
                );
            }
            framebuffer::rect(x + (width - 7) / 2, clock_y - 13, 7, 7, status_color);
        } else {
            let clock_width = if desktop.preferences.clock_seconds {
                48
            } else {
                30
            } + if desktop.preferences.clock_24h { 0 } else { 18 };
            let clock_x = x + width - clock_width - 12;
            let clock_y = y + (height - 8) / 2;
            draw_clock(
                clock_x,
                clock_y,
                clock,
                desktop.preferences.clock_seconds,
                desktop.preferences.clock_24h,
            );
            framebuffer::rect(clock_x - 15, clock_y, 7, 7, status_color);
            let mut widget_right = clock_x - 22;
            if desktop.preferences.taskbar_widgets & TASKBAR_WIDGET_DATE != 0 {
                widget_right -= 66;
                draw_short_date(
                    widget_right + 4,
                    clock_y,
                    clock,
                    desktop.preferences.date_format,
                    color::MUTED,
                );
            }
            if desktop.preferences.taskbar_widgets & TASKBAR_WIDGET_ACTIVE_APP != 0 {
                widget_right -= 86;
                let label = desktop.active.localized_label(desktop.locale());
                framebuffer::text(
                    widget_right + 4,
                    clock_y,
                    browser_text_prefix(label, 12),
                    desktop.active.accent(),
                    1,
                );
            }
            if desktop.preferences.taskbar_widgets & TASKBAR_WIDGET_WEATHER != 0 {
                widget_right -= 68;
                if let Some(temperature) = desktop.native_apps.weather_temperature_display() {
                    draw_compact_temperature(
                        widget_right + 4,
                        clock_y,
                        temperature,
                        desktop.native_apps.weather_unit_label(),
                        color::CYAN,
                    );
                } else {
                    framebuffer::text(widget_right + 4, clock_y, "WEATHER --", color::MUTED, 1);
                }
            }
            if desktop.preferences.taskbar_widgets & TASKBAR_WIDGET_PERFORMANCE != 0 {
                widget_right -= 58;
                framebuffer::text(widget_right + 4, clock_y, "FPS", color::MUTED, 1);
                draw_number(
                    widget_right + 30,
                    clock_y,
                    desktop.preferences.refresh_rate.hz() as u64,
                    color::GREEN,
                );
            }
            if desktop.preferences.taskbar_widgets & TASKBAR_WIDGET_AUDIO != 0 {
                widget_right -= 54;
                let status = audio::status();
                framebuffer::text(
                    widget_right + 4,
                    clock_y,
                    if status.muted {
                        "MUTE"
                    } else if status.playing {
                        "PLAY"
                    } else if status.backend != audio::Backend::Unavailable {
                        "AUDIO"
                    } else {
                        "NO AUD"
                    },
                    if status.playing {
                        color::GREEN
                    } else {
                        color::MUTED
                    },
                    1,
                );
            }
            if desktop.preferences.taskbar_widgets & TASKBAR_WIDGET_TIMEZONE != 0 {
                widget_right -= 62;
                framebuffer::text(
                    widget_right + 4,
                    clock_y,
                    browser_text_prefix(TIMEZONE_LABELS[desktop.preferences.timezone as usize], 9),
                    color::CYAN,
                    1,
                );
            }
        }
    }
}

fn draw_compact_temperature(x: i32, y: i32, value: i16, unit: &str, ink: u32) {
    if value < 0 {
        framebuffer::text(x, y, "-", ink, 1);
    }
    draw_number(
        x + if value < 0 { 6 } else { 0 },
        y,
        (value.unsigned_abs() / 10) as u64,
        ink,
    );
    framebuffer::text(x + 28, y, unit, ink, 1);
}

fn draw_two_digits(x: i32, y: i32, value: u8, color: u32) {
    let bytes = [b'0' + value / 10, b'0' + value % 10];
    let text = core::str::from_utf8(&bytes).unwrap_or("??");
    framebuffer::text(x, y, text, color, 1);
}

fn days_in_desktop_month(year: u16, month: u8) -> u8 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn adjusted_time(preferences: DesktopPreferences) -> crate::hardware::RtcTime {
    let mut time = crate::hardware::rtc_time();
    let timezone = TIMEZONE_OFFSETS_MINUTES[preferences.timezone.min(15) as usize] as i32;
    let manual = preferences.clock_offset_quarters as i32 * 15;
    let mut minutes = time.hour as i32 * 60 + time.minute as i32 + timezone + manual;
    while minutes < 0 {
        minutes += 24 * 60;
        if time.day > 1 {
            time.day -= 1;
        } else {
            if time.month > 1 {
                time.month -= 1;
            } else {
                time.month = 12;
                time.year = time.year.saturating_sub(1);
            }
            time.day = days_in_desktop_month(time.year, time.month);
        }
    }
    while minutes >= 24 * 60 {
        minutes -= 24 * 60;
        if time.day < days_in_desktop_month(time.year, time.month) {
            time.day += 1;
        } else {
            time.day = 1;
            if time.month < 12 {
                time.month += 1;
            } else {
                time.month = 1;
                time.year = time.year.saturating_add(1);
            }
        }
    }
    time.hour = (minutes / 60) as u8;
    time.minute = (minutes % 60) as u8;
    time
}

fn draw_clock(x: i32, y: i32, time: crate::hardware::RtcTime, seconds: bool, clock_24h: bool) {
    let mut bytes = [b'0'; 11];
    let mut hour = time.hour;
    if !clock_24h {
        hour %= 12;
        if hour == 0 {
            hour = 12;
        }
    }
    bytes[0] = b'0' + hour / 10;
    bytes[1] = b'0' + hour % 10;
    bytes[2] = b':';
    bytes[3] = b'0' + time.minute / 10;
    bytes[4] = b'0' + time.minute % 10;
    let mut length = if seconds {
        bytes[5] = b':';
        bytes[6] = b'0' + time.second / 10;
        bytes[7] = b'0' + time.second % 10;
        8
    } else {
        5
    };
    if !clock_24h {
        bytes[length] = b' ';
        bytes[length + 1] = if time.hour >= 12 { b'P' } else { b'A' };
        bytes[length + 2] = b'M';
        length += 3;
    }
    let text = core::str::from_utf8(&bytes[..length]).unwrap_or("--:--");
    framebuffer::text(x, y, text, color::INK, 1);
}

fn draw_short_date(x: i32, y: i32, time: crate::hardware::RtcTime, format: u8, ink: u32) {
    let mut bytes = [b'0'; 10];
    let year = time.year;
    let (first, second, separator) = match format {
        1 => (time.day, time.month, b'/'),
        2 => (time.month, time.day, b'/'),
        _ => {
            bytes[0] = b'0' + ((year / 1000) % 10) as u8;
            bytes[1] = b'0' + ((year / 100) % 10) as u8;
            bytes[2] = b'0' + ((year / 10) % 10) as u8;
            bytes[3] = b'0' + (year % 10) as u8;
            bytes[4] = b'-';
            bytes[5] = b'0' + time.month / 10;
            bytes[6] = b'0' + time.month % 10;
            bytes[7] = b'-';
            bytes[8] = b'0' + time.day / 10;
            bytes[9] = b'0' + time.day % 10;
            framebuffer::text(x, y, core::str::from_utf8(&bytes).unwrap_or("----"), ink, 1);
            return;
        }
    };
    bytes[0] = b'0' + first / 10;
    bytes[1] = b'0' + first % 10;
    bytes[2] = separator;
    bytes[3] = b'0' + second / 10;
    bytes[4] = b'0' + second % 10;
    bytes[5] = separator;
    bytes[6] = b'0' + ((year / 1000) % 10) as u8;
    bytes[7] = b'0' + ((year / 100) % 10) as u8;
    bytes[8] = b'0' + ((year / 10) % 10) as u8;
    bytes[9] = b'0' + (year % 10) as u8;
    framebuffer::text(x, y, core::str::from_utf8(&bytes).unwrap_or("----"), ink, 1);
}

fn format_signed_minutes(minutes: i16, output: &mut [u8; 16]) -> &str {
    let absolute = minutes.unsigned_abs();
    output[0] = if minutes < 0 { b'-' } else { b'+' };
    output[1] = b'0' + ((absolute / 60) / 10) as u8;
    output[2] = b'0' + ((absolute / 60) % 10) as u8;
    output[3] = b':';
    output[4] = b'0' + ((absolute % 60) / 10) as u8;
    output[5] = b'0' + (absolute % 10) as u8;
    core::str::from_utf8(&output[..6]).unwrap_or("+00:00")
}

fn draw_launcher(desktop: &DesktopState) {
    let launcher = launcher_rect(desktop.preferences);
    let x = launcher.x as i32;
    let y = launcher.y as i32;
    let width = launcher.width as i32;
    let height = launcher.height as i32;
    framebuffer::rounded_rect(x, y, width, height, 9, desktop.preferences.panel_color());
    framebuffer::outline(x, y, width, height, desktop.preferences.border_color(false));
    framebuffer::text(
        x + 16,
        y + 17,
        desktop.locale().app(AppKind::Apps.index()),
        color::INK,
        1,
    );
    let installed = desktop.native_apps.installed_count();
    if installed != 0 {
        framebuffer::text(x + width - 92, y + 17, "AYO DIRECT", color::CYAN, 1);
    }
    framebuffer::rect(x + 12, y + 39, width - 24, 1, color::BORDER);
    let columns = launcher_columns(desktop.preferences);
    let row_height = launcher_row_height(desktop.preferences);
    let rows = launcher_visible_rows(desktop.preferences);
    let cell_width = (width - 16) / columns as i32;
    let scale = state::UI_SCALE_PERCENT[desktop.preferences.ui_scale.min(6) as usize] as i32;
    let icon_size = (((row_height - 7) * scale) / 100).clamp(12, (row_height - 3).max(12));
    let capacity = rows * columns;
    for slot in 0..capacity {
        let ordinal = desktop.launcher_scroll + slot;
        let Some(item) = launcher_item(desktop, ordinal) else {
            break;
        };
        let column = slot % columns;
        let row = slot / columns;
        let row_x = x + 8 + column as i32 * cell_width;
        let row_y = y + 48 + row as i32 * row_height;
        let (label, category, accent, shortcut, running) = match item {
            LauncherItem::BuiltIn(app) => (
                app.localized_label(desktop.locale()),
                "ExpOS",
                app.accent(),
                app.shortcut(),
                desktop.app_open[app.index()],
            ),
            LauncherItem::Installed(index) => {
                let name = crate::apps::NativeApps::app_name(index);
                (
                    name,
                    crate::apps::NativeApps::app_category(index),
                    crate::apps::NativeApps::app_accent(index),
                    &name[..1],
                    desktop.app_open[AppKind::Apps.index()]
                        && desktop.native_apps.active_index() == index,
                )
            }
        };
        let selected = ordinal == desktop.launcher_selection;
        framebuffer::rounded_rect(
            row_x,
            row_y,
            cell_width - 4,
            row_height - 3,
            5,
            if selected {
                blend_color(
                    desktop.preferences.panel_color(),
                    desktop.preferences.accent.color(),
                    72,
                )
            } else if running {
                0x0024_292F
            } else {
                desktop.preferences.panel_color()
            },
        );
        if selected {
            framebuffer::outline(
                row_x,
                row_y,
                cell_width - 4,
                row_height - 3,
                desktop.preferences.accent.color(),
            );
        }
        let icon_x = row_x + 7;
        let icon_y = row_y + (row_height - icon_size) / 2;
        framebuffer::rounded_rect(icon_x, icon_y, icon_size, icon_size, 4, accent);
        framebuffer::text(
            icon_x + 4,
            icon_y + (icon_size - 8) / 2,
            shortcut,
            color::WHITE,
            1,
        );
        framebuffer::text(row_x + icon_size + 14, row_y + 6, label, color::INK, 1);
        if desktop.preferences.menu_categories && row_height >= 28 {
            framebuffer::text(
                row_x + icon_size + 14,
                row_y + row_height - 11,
                category,
                color::MUTED,
                1,
            );
        }
        if running {
            framebuffer::rect(
                row_x + cell_width - 14,
                row_y + row_height / 2,
                5,
                5,
                color::GREEN,
            );
        }
    }
    let total = launcher_item_count(desktop);
    if desktop.launcher_scroll > 0 {
        framebuffer::text(
            x + width - 22,
            y + 17,
            "^",
            desktop.preferences.accent.color(),
            1,
        );
    }
    if desktop.launcher_scroll + capacity < total {
        framebuffer::text(
            x + width - 22,
            y + height - 16,
            "v",
            desktop.preferences.accent.color(),
            1,
        );
    }
    if total == 0 {
        framebuffer::text(
            x + 18,
            y + 64,
            "No launcher entries are visible.",
            color::MUTED,
            1,
        );
        framebuffer::text(
            x + 18,
            y + 84,
            "Open Settings > Menu to restore them.",
            color::INK,
            1,
        );
    } else if desktop.preferences.tooltips {
        framebuffer::text(
            x + 16,
            y + height - 16,
            "UP/DOWN scroll  //  click an app to open",
            color::MUTED,
            1,
        );
    }
}

fn draw_shell_confirmation(desktop: &DesktopState) {
    let width = 420_i32.min(framebuffer::width() as i32 - 32);
    let height = 126_i32;
    let x = (framebuffer::width() as i32 - width) / 2;
    let y = (framebuffer::height() as i32 - height) / 2;
    framebuffer::alpha_rect(
        0,
        0,
        framebuffer::width() as i32,
        framebuffer::height() as i32,
        0,
        150,
    );
    framebuffer::rounded_rect(x, y, width, height, 8, desktop.preferences.panel_color());
    framebuffer::outline(x, y, width, height, desktop.preferences.accent.color());
    let locale = desktop.locale();
    framebuffer::text(
        x + 20,
        y + 20,
        locale.text(LocalText::OpenShell),
        color::INK,
        2,
    );
    framebuffer::text(
        x + 20,
        y + 56,
        locale.text(LocalText::DesktopWillClose),
        color::MUTED,
        1,
    );
    framebuffer::rounded_rect(
        x + 20,
        y + 84,
        116,
        26,
        4,
        desktop.preferences.accent.color(),
    );
    framebuffer::text(
        x + 34,
        y + 93,
        locale.text(LocalText::Open),
        color::WHITE,
        1,
    );
    framebuffer::outline(x + 150, y + 84, 116, 26, color::BORDER);
    framebuffer::text(
        x + 169,
        y + 93,
        locale.text(LocalText::Cancel),
        color::INK,
        1,
    );
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

fn unwrap_duckduckgo_target(address: &str, output: &mut [u8]) -> Option<usize> {
    let query = address
        .strip_prefix("https://duckduckgo.com/l/?")
        .or_else(|| address.strip_prefix("https://www.duckduckgo.com/l/?"))?;
    let encoded = query
        .split('&')
        .find_map(|part| part.strip_prefix("uddg="))?;
    percent_decode_url(encoded.as_bytes(), output)
}

fn percent_decode_url(input: &[u8], output: &mut [u8]) -> Option<usize> {
    let mut source = 0;
    let mut length = 0;
    while source < input.len() {
        let byte = if input[source] == b'%' {
            let high = hex_digit(*input.get(source + 1)?)?;
            let low = hex_digit(*input.get(source + 2)?)?;
            source += 3;
            (high << 4) | low
        } else {
            let byte = input[source];
            source += 1;
            byte
        };
        if !(0x21..=0x7E).contains(&byte) || length == output.len() {
            return None;
        }
        output[length] = byte;
        length += 1;
    }
    (length != 0).then_some(length)
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn wikipedia_summary_url(address: &str, output: &mut [u8]) -> Option<usize> {
    let rest = address.strip_prefix("https://")?;
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    if host != "wikipedia.org" && !host.ends_with(".wikipedia.org") {
        return None;
    }
    let path = &rest[slash..];
    let title = path.strip_prefix("/wiki/")?.split(['?', '#']).next()?;
    if title.is_empty() {
        return None;
    }
    let mut length = 0;
    for part in [
        "https://".as_bytes(),
        host.as_bytes(),
        "/api/rest_v1/page/summary/".as_bytes(),
        title.as_bytes(),
    ] {
        if length + part.len() > output.len() {
            return None;
        }
        output[length..length + part.len()].copy_from_slice(part);
        length += part.len();
    }
    Some(length)
}

fn wikipedia_document(json: &[u8], output: &mut [u8]) -> Option<usize> {
    let mut title = [0_u8; 512];
    let mut description = [0_u8; 512];
    let mut extract = [0_u8; 8 * 1024];
    let title_len = json_string_field(json, b"title", &mut title)?;
    let description_len = json_string_field(json, b"description", &mut description).unwrap_or(0);
    let extract_len = json_string_field(json, b"extract", &mut extract)?;
    let mut length = 0;
    for part in [
        b"<style>h1{color:#74bcc7}.wiki{background:#111820;border:1px solid #32404a;padding:8px}a{color:#74bcc7}</style><title>Wikipedia</title><h1>Wikipedia // "
            .as_slice(),
        &title[..title_len],
        b"</h1><p class='wiki'>",
        &description[..description_len],
        b"</p>",
    ] {
        append_html(&mut length, output, part)?;
    }
    let mut cursor = 0;
    while cursor < extract_len {
        let remaining = &extract[cursor..extract_len];
        let limit = remaining.len().min(470);
        let split = if limit == remaining.len() {
            limit
        } else {
            remaining[..limit]
                .iter()
                .rposition(|byte| *byte == b' ')
                .unwrap_or(limit)
        };
        append_html(&mut length, output, b"<p>")?;
        append_html(&mut length, output, &remaining[..split])?;
        append_html(&mut length, output, b"</p>")?;
        cursor += split;
        while cursor < extract_len && extract[cursor] == b' ' {
            cursor += 1;
        }
    }
    append_html(
        &mut length,
        output,
        b"<p class='wiki'>Verified HTTPS summary from Wikipedia. Use Up, Down, Space, or H to read.</p>",
    )?;
    Some(length)
}

fn json_string_field(json: &[u8], field: &[u8], output: &mut [u8]) -> Option<usize> {
    let mut pattern = [0_u8; 40];
    if field.len() + 4 > pattern.len() {
        return None;
    }
    pattern[0] = b'"';
    pattern[1..1 + field.len()].copy_from_slice(field);
    pattern[1 + field.len()..4 + field.len()].copy_from_slice(b"\":\"");
    let start = find_bytes_local(json, &pattern[..field.len() + 4])? + field.len() + 4;
    let mut source = start;
    let mut length = 0;
    while source < json.len() && length < output.len() {
        let byte = json[source];
        source += 1;
        if byte == b'"' {
            return Some(length);
        }
        if byte == b'\\' {
            let escaped = *json.get(source)?;
            source += 1;
            let decoded = match escaped {
                b'"' | b'\\' | b'/' => escaped,
                b'n' | b'r' | b't' => b' ',
                b'u' => {
                    source = source.checked_add(4)?;
                    b'?'
                }
                _ => return None,
            };
            output[length] = decoded;
            length += 1;
        } else if byte.is_ascii() && byte >= b' ' {
            output[length] = match byte {
                b'<' | b'>' | b'&' => b' ',
                _ => byte,
            };
            length += 1;
        } else if byte & 0xC0 != 0x80 {
            output[length] = b'?';
            length += 1;
        }
    }
    None
}

fn append_html(length: &mut usize, output: &mut [u8], bytes: &[u8]) -> Option<()> {
    let end = length.checked_add(bytes.len())?;
    if end > output.len() {
        return None;
    }
    output[*length..end].copy_from_slice(bytes);
    *length = end;
    Some(())
}

fn find_bytes_local(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    (!needle.is_empty())
        .then(|| {
            haystack
                .windows(needle.len())
                .position(|window| window == needle)
        })
        .flatten()
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

fn browser_origin<'a>(url: &str, output: &'a mut [u8]) -> Option<&'a str> {
    let parsed = network::parse_http_url(url).ok();
    if let Some(parsed) = parsed {
        let scheme = if parsed.scheme == network::HttpScheme::Https {
            "https://"
        } else {
            "http://"
        };
        let mut length = 0;
        for part in [scheme.as_bytes(), parsed.host.as_bytes()] {
            if length + part.len() > output.len() {
                return None;
            }
            output[length..length + part.len()].copy_from_slice(part);
            length += part.len();
        }
        if parsed.port != parsed.scheme.default_port() {
            if length + 6 > output.len() {
                return None;
            }
            output[length] = b':';
            length += 1;
            let mut digits = [0_u8; 5];
            let mut value = parsed.port;
            let mut count = 0;
            loop {
                digits[digits.len() - 1 - count] = b'0' + (value % 10) as u8;
                count += 1;
                value /= 10;
                if value == 0 {
                    break;
                }
            }
            output[length..length + count].copy_from_slice(&digits[digits.len() - count..]);
            length += count;
        }
        return core::str::from_utf8(&output[..length]).ok();
    }
    if let Some(rest) = url.strip_prefix("expos://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let value = if authority.is_empty() {
            "expos://local"
        } else {
            let prefix = b"expos://";
            if prefix.len() + authority.len() > output.len() {
                return None;
            }
            output[..prefix.len()].copy_from_slice(prefix);
            output[prefix.len()..prefix.len() + authority.len()]
                .copy_from_slice(authority.as_bytes());
            return core::str::from_utf8(&output[..prefix.len() + authority.len()]).ok();
        };
        output[..value.len()].copy_from_slice(value.as_bytes());
        return core::str::from_utf8(&output[..value.len()]).ok();
    }
    None
}

fn flatten_browser_text(input: &[u8], output: &mut [u8]) -> usize {
    let mut length = 0;
    for byte in input.iter().copied() {
        let byte = if byte.is_ascii_whitespace() {
            b' '
        } else {
            byte
        };
        if !byte.is_ascii_graphic() && byte != b' ' {
            continue;
        }
        if byte == b' ' && (length == 0 || output[length - 1] == b' ') {
            continue;
        }
        if length == output.len() {
            break;
        }
        output[length] = byte;
        length += 1;
    }
    while length != 0 && output[length - 1] == b' ' {
        length -= 1;
    }
    length
}

fn decode_browser_image(bytes: &[u8], url: &str, cache: &mut BrowserMediaCache) -> bool {
    if bytes.len() < 54 || &bytes[..2] != b"BM" {
        return false;
    }
    let Some(pixel_offset) = browser_u32(bytes, 10).map(|value| value as usize) else {
        return false;
    };
    let Some(width) = browser_i32(bytes, 18) else {
        return false;
    };
    let Some(height) = browser_i32(bytes, 22) else {
        return false;
    };
    let Some(planes) = browser_u16(bytes, 26) else {
        return false;
    };
    let Some(bits) = browser_u16(bytes, 28) else {
        return false;
    };
    let Some(compression) = browser_u32(bytes, 30) else {
        return false;
    };
    let top_down = height < 0;
    let height = height.unsigned_abs() as usize;
    let width = width.unsigned_abs() as usize;
    if planes != 1
        || !matches!(bits, 24 | 32)
        || compression != 0
        || width == 0
        || height == 0
        || width > 64
        || height > 64
    {
        return false;
    }
    let bytes_per_pixel = bits as usize / 8;
    let row_bytes = (width * bytes_per_pixel).div_ceil(4) * 4;
    if pixel_offset
        .checked_add(row_bytes.saturating_mul(height))
        .is_none_or(|end| end > bytes.len())
    {
        return false;
    }
    for y in 0..height {
        let source_y = if top_down { y } else { height - 1 - y };
        let row = pixel_offset + source_y * row_bytes;
        for x in 0..width {
            let offset = row + x * bytes_per_pixel;
            cache.image_pixels[y * 64 + x] = ((bytes[offset + 2] as u32) << 16)
                | ((bytes[offset + 1] as u32) << 8)
                | bytes[offset] as u32;
        }
    }
    cache.image_width = width as u8;
    cache.image_height = height as u8;
    cache.image_url = BrowserText::new(url).unwrap_or(BrowserText::empty());
    true
}

fn browser_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let value = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([value[0], value[1]]))
}

fn browser_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let value = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn browser_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    browser_u32(bytes, offset).map(|value| value as i32)
}

fn resolve_browser_link(base: &str, target: &str, output: &mut [u8]) -> Option<usize> {
    let target = target.trim();
    if target.is_empty() || !target.is_ascii() {
        return None;
    }
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("expos://")
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
    fn duckduckgo_redirects_are_unwrapped_before_the_network_request() {
        let mut output = [0_u8; 512];
        let input = "https://duckduckgo.com/l/?uddg=https%3A%2F%2Fen.wikipedia.org%2Fwiki%2FRust_%28programming_language%29&rut=ignored";
        let length = unwrap_duckduckgo_target(input, &mut output).expect("wrapped target");
        assert_eq!(
            core::str::from_utf8(&output[..length]).unwrap(),
            "https://en.wikipedia.org/wiki/Rust_(programming_language)"
        );
    }

    #[test]
    fn wikipedia_articles_use_the_live_bounded_summary_endpoint() {
        let mut output = [0_u8; 512];
        let length = wikipedia_summary_url("https://en.wikipedia.org/wiki/NetBSD", &mut output)
            .expect("Wikipedia article URL");
        assert_eq!(
            core::str::from_utf8(&output[..length]).unwrap(),
            "https://en.wikipedia.org/api/rest_v1/page/summary/NetBSD"
        );
        assert!(wikipedia_summary_url("https://example.org/wiki/NetBSD", &mut output).is_none());
    }

    #[test]
    fn wikipedia_json_becomes_a_readable_safe_document() {
        let json = br#"{"title":"NetBSD","description":"Unix-like operating system","extract":"NetBSD is a free and open-source operating system. It emphasizes portability."}"#;
        let mut html = [0_u8; 2048];
        let length = wikipedia_document(json, &mut html).expect("summary document");
        let html = core::str::from_utf8(&html[..length]).unwrap();
        assert!(html.contains("Wikipedia // NetBSD"));
        assert!(html.contains("Unix-like operating system"));
        assert!(html.contains("It emphasizes portability."));
        Document::parse("https://en.wikipedia.org/wiki/NetBSD", html)
            .expect("safe bounded document");
    }

    #[test]
    fn browser_session_limits_and_case_insensitive_find_are_deterministic() {
        let home = BrowserTab::home();
        assert_eq!(home.url(), "expos://home");
        assert_eq!(home.title(), "New tab");
        assert_eq!(home.history_count, 1);
        assert_eq!(BROWSER_TAB_CAPACITY, 6);
        assert_eq!(BROWSER_HISTORY_CAPACITY, 12);
        assert_eq!(BROWSER_BOOKMARK_CAPACITY, 8);
        assert!(browser_text_contains("Chromium-like workflow", "CHROMIUM"));
        assert!(!browser_text_contains("ExpOS Browser", "Firefox"));

        let bookmark = BrowserBookmark::new("https://example.com/", "Example Domain");
        assert_eq!(bookmark.url(), "https://example.com/");
        assert_eq!(bookmark.title(), "Example Domain");
    }

    #[test]
    fn bounded_bmp_decoder_preserves_dimensions_and_rgb_pixels() {
        let mut bitmap = [0_u8; 62];
        bitmap[..2].copy_from_slice(b"BM");
        bitmap[2..6].copy_from_slice(&62_u32.to_le_bytes());
        bitmap[10..14].copy_from_slice(&54_u32.to_le_bytes());
        bitmap[14..18].copy_from_slice(&40_u32.to_le_bytes());
        bitmap[18..22].copy_from_slice(&2_i32.to_le_bytes());
        bitmap[22..26].copy_from_slice(&1_i32.to_le_bytes());
        bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
        bitmap[54..60].copy_from_slice(&[0, 0, 255, 0, 255, 0]);
        let mut cache = BrowserMediaCache::new();
        assert!(decode_browser_image(&bitmap, "/material.bmp", &mut cache));
        assert_eq!(cache.image_width, 2);
        assert_eq!(cache.image_height, 1);
        assert_eq!(cache.image_pixels[0], 0x00ff_0000);
        assert_eq!(cache.image_pixels[1], 0x0000_ff00);
        assert_eq!(cache.image_url.as_str(), "/material.bmp");
    }

    #[test]
    fn performance_settings_are_conservative_and_have_a_dedicated_category() {
        let preferences = DesktopPreferences::from_persistent(state::PersistentPreferences::new());
        assert!(!preferences.window_shadows);
        assert!(!preferences.wallpaper_effects);
        assert!(!preferences.responsive_presentation);
        assert_eq!(preferences.presentation_policy_label(), "Efficient");
        assert_eq!(SettingsCategory::ALL.len(), 18);
        assert_eq!(SettingsCategory::Audio.index(), 5);
        assert_eq!(SettingsCategory::Performance.index(), 6);
        assert_eq!(SettingsCategory::Audio.row_count(), 5);
        assert_eq!(SettingsCategory::Performance.row_count(), 4);
    }

    #[test]
    fn customization_pages_expose_more_than_one_thousand_real_values() {
        assert_eq!(CUSTOMIZATION_VALUE_COUNT, 1_288);
        const { assert!(CUSTOMIZATION_VALUE_COUNT >= 1_000) };
        const { assert!(state::CUSTOMIZATION_SELECTABLE_VALUES >= 1_000) };
        assert_eq!(SettingsCategory::Profiles.row_count(), 8);
        assert_eq!(SettingsCategory::Appearance.row_count(), 8);
        assert_eq!(SettingsCategory::Accessibility.row_count(), 11);
        assert_eq!(SettingsCategory::Windows.row_count(), 9);
        assert_eq!(SettingsCategory::Taskbar.row_count(), 13);
        assert_eq!(SettingsCategory::Menu.row_count(), 9);
        assert_eq!(SettingsCategory::Terminal.row_count(), 5);
        assert_eq!(SettingsCategory::Language.row_count(), 5);
        assert_eq!(SettingsCategory::Time.row_count(), 7);
        let preferences = DesktopPreferences::from_persistent(state::PersistentPreferences::new());
        let extension = preferences.encode_ui_extension();
        assert_eq!(&extension[..4], b"DUI3");
        assert!(preferences.clock_24h);
        let mut label = [0_u8; 16];
        assert_eq!(format_signed_minutes(345, &mut label), "+05:45");
    }

    #[test]
    fn highest_palette_ids_round_trip_and_produce_bounded_dark_surfaces() {
        let theme = ThemeChoice::from_persisted(255);
        let backdrop = BackdropChoice::from_persisted(255);
        assert_eq!(theme.persisted(), 255);
        assert_eq!(backdrop.persisted(), 255);
        assert_ne!(theme.panel(), ThemeChoice::Obsidian.panel());
        assert_ne!(theme.chrome(), theme.window());
        assert!(theme.panel() <= 0x00FF_FFFF);
        assert!(theme.chrome() <= 0x00FF_FFFF);
        assert!(theme.window() <= 0x00FF_FFFF);
        assert!(theme.card() <= 0x00FF_FFFF);
        assert!(theme.terminal() <= 0x00FF_FFFF);
        assert!(backdrop.color() <= 0x00FF_FFFF);
        assert_eq!(ThemeChoice::Obsidian.shifted(16).persisted(), 16);
        assert_eq!(ThemeChoice::from_persisted(250).shifted(16).persisted(), 10);
        assert_eq!(
            AccentChoice::from_persisted(4).shifted(-16).persisted(),
            244
        );
    }

    #[test]
    fn coordinated_profiles_cover_compact_focus_accessible_showcase_and_touch_needs() {
        let mut persistent = state::PersistentPreferences::new();
        persistent.refresh_rate = state::RefreshRate::Hz120;
        persistent.vsync = false;
        let mut current = DesktopPreferences::from_persistent(persistent);
        current.locale = Locale::Hebrew.persisted();
        current.menu_layout = 3;
        current.terminal_scale = 2;

        let accessible = current.with_profile(CustomizationProfile::Accessible);
        assert!(accessible.high_contrast);
        assert_eq!(accessible.font_weight, framebuffer::FontWeight::Bold);
        assert_eq!(accessible.taskbar_size, 8);
        assert_eq!(accessible.titlebar_density, 4);
        assert_eq!(accessible.cursor, CursorChoice::Crosshair);
        assert_eq!(accessible.animation_level, 0);
        assert!(accessible.reduce_transparency);
        assert!(accessible.focus_ring);
        assert_eq!(accessible.refresh_rate, state::RefreshRate::Hz120);
        assert!(!accessible.vsync);
        assert_eq!(accessible.locale, Locale::Hebrew.persisted());
        assert_eq!(accessible.menu_layout, 3);
        assert_eq!(accessible.terminal_scale, 2);

        let showcase = current.with_profile(CustomizationProfile::Showcase);
        assert!(showcase.window_shadows);
        assert!(showcase.wallpaper_effects);
        assert!(showcase.taskbar_translucent);
        assert!(showcase.menu_grid);

        let focus = current.with_profile(CustomizationProfile::Focus);
        assert!(!focus.window_shadows);
        assert!(!focus.wallpaper_effects);
        assert!(focus.taskbar_autohide);
        assert_eq!(focus.animation_level, 0);

        let touch = current.with_profile(CustomizationProfile::Touch);
        assert_eq!(touch.taskbar_size, 8);
        assert_eq!(touch.menu_density, 4);
        assert_eq!(touch.ui_scale, 6);

        let night = current.with_profile(CustomizationProfile::Night);
        assert!(night.reduce_transparency);
        assert!(!night.notification_animations);

        let presentation = current.with_profile(CustomizationProfile::Presentation);
        assert!(presentation.high_contrast);
        assert!(presentation.focus_ring);
        assert!(presentation.reduce_transparency);
    }

    #[test]
    fn compact_settings_keep_selected_categories_and_rows_visible() {
        let compact = Rect::new(0, 0, 480, 360);
        assert_eq!(settings_category_capacity(compact), 7);
        assert_eq!(settings_category_view_start(compact, 13), 7);
        assert_eq!(settings_row_capacity(272), 5);
        assert_eq!(
            settings_row_view_start(SettingsCategory::Windows, 7, 272),
            3
        );
    }

    #[test]
    fn persisted_performance_flags_enable_real_rendering_policies() {
        let mut persistent = state::PersistentPreferences::new();
        persistent.flags |= state::PREF_WINDOW_SHADOWS
            | state::PREF_WALLPAPER_EFFECTS
            | state::PREF_RESPONSIVE_PRESENTATION;
        let preferences = DesktopPreferences::from_persistent(persistent);
        assert!(preferences.window_shadows);
        assert!(preferences.wallpaper_effects);
        assert!(preferences.responsive_presentation);
        assert_eq!(preferences.presentation_policy_label(), "Responsive");
    }

    #[test]
    fn responsive_policy_bypasses_only_damaged_commit_pacing() {
        assert!(!bypass_software_pacing(false, false));
        assert!(!bypass_software_pacing(false, true));
        assert!(!bypass_software_pacing(true, false));
        assert!(bypass_software_pacing(true, true));
    }

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
    fn browser_container_keeps_the_last_valid_document_after_rejection() {
        let home = Document::parse("expos://home", HOME).expect("home document");
        let mut container = BrowserContainer::new(home);
        assert!(core::mem::size_of::<BrowserContainer>() < core::mem::size_of::<Document>());

        container.reject(BrowserError::TooManyNodes);
        assert_eq!(container.url(), "expos://home");
        assert_eq!(container.generation, 1);
        assert_eq!(container.rejected_loads, 1);
        assert_eq!(container.last_error, Some(BrowserError::TooManyNodes));

        let about = Document::parse("expos://about", ABOUT).expect("about document");
        container.replace(about);
        assert_eq!(container.url(), "expos://about");
        assert_eq!(container.generation, 2);
        assert_eq!(container.rejected_loads, 1);
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
