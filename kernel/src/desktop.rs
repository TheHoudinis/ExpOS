use crate::{
    framebuffer,
    input::{
        Input, InputEvent, PointerEvent, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_SUPER_BROWSER,
        KEY_SUPER_CLOSE, KEY_SUPER_CYCLE, KEY_SUPER_DOWN, KEY_SUPER_FULLSCREEN, KEY_SUPER_LAUNCHER,
        KEY_SUPER_LEFT, KEY_SUPER_RIGHT, KEY_SUPER_TERMINAL, KEY_SUPER_UP, KEY_UP,
    },
    network, radio, slog,
};
use framebuffer::color;
use hexa_core::{
    Authority, BufferFormat, BufferHandle, CapabilityBroker, DisplayServer, Document, Fin,
    NodeKind, Operations, Rect, SurfaceRole,
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
const APP_WIDTH: u16 = if framebuffer::WIDTH >= 1600 {
    1280
} else {
    860
};
const APP_HEIGHT: u16 = if framebuffer::HEIGHT >= 1000 {
    800
} else {
    610
};
const BUFFER_WIDTH: u16 = framebuffer::WIDTH as u16;
const BUFFER_HEIGHT: u16 = framebuffer::HEIGHT as u16;
const TERMINAL_HISTORY: usize = 12;
const TERMINAL_CAPACITY: usize = 48;
const NOTES_CAPACITY: usize = 2048;
const CURSOR_WIDTH: usize = 14;
const CURSOR_HEIGHT: usize = 20;
const TASKBAR_HEIGHT: i16 = 48;
const TASKBAR_Y: i16 = framebuffer::HEIGHT as i16 - TASKBAR_HEIGHT;
const START_X: i16 = 8;
const TASK_ICON_X: i16 = 48;
const TASK_ICON_STEP: i16 = 36;
const LAUNCHER_X: i16 = 8;
const LAUNCHER_Y: i16 = TASKBAR_Y - LAUNCHER_HEIGHT as i16 - 8;
const LAUNCHER_WIDTH: u16 = 250;
const LAUNCHER_HEIGHT: u16 = 318;
const SETTINGS_SIDEBAR_WIDTH: i16 = 206;
const SETTINGS_CATEGORY_TOP: i16 = 92;
const SETTINGS_CATEGORY_STEP: i16 = 42;
const SETTINGS_ROW_TOP: i16 = 112;
const SETTINGS_ROW_HEIGHT: i16 = 54;
const SETTINGS_ROW_STEP: i16 = 62;
const DISPLAY_MODE_LABEL: &str = if framebuffer::WIDTH == 1920 && framebuffer::HEIGHT == 1080 {
    "1920 x 1080"
} else {
    "Current framebuffer"
};
const STABLE_FIN: Fin = Fin::from_u128(0x4449_4D00_0000_0000_0000_0000_0000_0001);

const HOME: &str = "<title>Home</title><h1>ExpOS</h1><a href='hexa://about'>About</a><a href='hexa://packages'>Packages</a><a href='hexa://system'>System</a>";
const ABOUT: &str = "<title>About</title><h1>Browser</h1><p>A small native document browser.</p><a href='hexa://home'>Home</a>";
const BROWSER_PACKAGES: &str = "<title>Packages</title><h1>Packages</h1><li>Core tools</li><li>Display</li><li>Notes</li><li>Games</li><a href='hexa://home'>Home</a>";
const BROWSER_SYSTEM: &str = "<title>System</title><h1>System</h1><li>1920 x 1080 display</li><li>Keyboard and mouse</li><li>RTL8139 network</li><a href='hexa://home'>Home</a>";
const NETWORK_BLOCKED: &str = "<title>Offline</title><h1>Offline</h1><p>The address could not be loaded.</p><a href='hexa://home'>Home</a>";
const NETWORK_ERROR: &str = "<title>Load failed</title><h1>Could not load page</h1><p>Check the address and use plain http.</p><a href='hexa://home'>Home</a>";

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
            Self::Appearance => 4,
            Self::Network => 4,
            Self::Bluetooth => 2,
            Self::Display => 4,
            Self::Input => 3,
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
}

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
}

#[derive(Clone, Copy, Debug)]
struct DesktopPreferences {
    accent: AccentChoice,
    backdrop: BackdropChoice,
    pure_black_apps: bool,
    rounded_controls: bool,
    taskbar_visible: bool,
    status_visible: bool,
    window_borders: bool,
    high_contrast: bool,
    pointer_speed: u8,
}

impl DesktopPreferences {
    const fn new() -> Self {
        Self {
            accent: AccentChoice::Green,
            backdrop: BackdropChoice::Graphite,
            pure_black_apps: true,
            rounded_controls: true,
            taskbar_visible: true,
            status_visible: true,
            window_borders: true,
            high_contrast: false,
            pointer_speed: 1,
        }
    }

    const fn window_color(self) -> u32 {
        if self.pure_black_apps {
            0x0000_0000
        } else {
            color::WINDOW
        }
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
}

// Preferences are retained for the current boot. HexaFS-backed account
// settings are still a queued storage integration, so reboot persistence is
// deliberately not claimed here.
static DESKTOP_PREFERENCES: crate::sync::SpinMutex<DesktopPreferences> =
    crate::sync::SpinMutex::new(DesktopPreferences::new());

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
    browser_line: [u8; 96],
    browser_len: usize,
    browser_editing: bool,
    terminal_line: [u8; TERMINAL_CAPACITY],
    terminal_len: usize,
    terminal_message: [u8; 128],
    terminal_message_len: usize,
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
}

struct PointerCursor {
    x: i16,
    y: i16,
    under: [u32; CURSOR_WIDTH * CURSOR_HEIGHT],
    drawn: bool,
}

impl PointerCursor {
    const fn new() -> Self {
        Self {
            x: (framebuffer::WIDTH / 2) as i16,
            y: (framebuffer::HEIGHT / 2) as i16,
            under: [0; CURSOR_WIDTH * CURSOR_HEIGHT],
            drawn: false,
        }
    }

    fn invalidate(&mut self) {
        self.drawn = false;
    }

    fn move_by(&mut self, dx: i16, dy: i16) {
        if dx == 0 && dy == 0 {
            return;
        }
        self.restore();
        self.x = (self.x.saturating_add(dx)).clamp(0, framebuffer::WIDTH as i16 - 1);
        self.y = (self.y.saturating_add(dy)).clamp(0, framebuffer::HEIGHT as i16 - 1);
        self.draw();
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
                    if edge { 0x0012_1822 } else { color::WHITE },
                );
            }
        }
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
        let preferences = *DESKTOP_PREFERENCES.lock();
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
                Rect::new(0, 0, framebuffer::WIDTH as u16, framebuffer::HEIGHT as u16),
            )
            .expect("desktop background surface");
        let panel = server
            .create_surface(
                DISPLAY_FIN,
                "Panel",
                SurfaceRole::Panel,
                Rect::new(
                    0,
                    TASKBAR_Y,
                    framebuffer::WIDTH as u16,
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
                buffer((index + 3) as u32, app.owner(), BUFFER_WIDTH, BUFFER_HEIGHT),
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
                Rect::new(LAUNCHER_X, LAUNCHER_Y, LAUNCHER_WIDTH, LAUNCHER_HEIGHT),
            )
            .expect("desktop launcher surface");
        let _ = server.attach(
            DISPLAY_FIN,
            background,
            buffer(
                1,
                DISPLAY_FIN,
                framebuffer::WIDTH as u16,
                framebuffer::HEIGHT as u16,
            ),
        );
        let _ = server.attach(
            DISPLAY_FIN,
            panel,
            buffer(
                2,
                DISPLAY_FIN,
                framebuffer::WIDTH as u16,
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
        Self {
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
            settings_notice: "Changes apply immediately for this boot.",
            document: Document::parse("hexa://home", HOME).expect("built-in home document"),
            browser_line: [0; 96],
            browser_len: 0,
            browser_editing: false,
            terminal_line: [0; TERMINAL_CAPACITY],
            terminal_len: 0,
            terminal_message: [0; 128],
            terminal_message_len: 0,
            terminal_history: [[0; TERMINAL_CAPACITY]; TERMINAL_HISTORY],
            terminal_history_len: [0; TERMINAL_HISTORY],
            terminal_history_next: 0,
            terminal_history_count: 0,
            terminal_history_cursor: None,
            notes: [0; NOTES_CAPACITY],
            notes_len: 0,
            games: crate::games::GameHub::new(),
            session,
            cursor: PointerCursor::new(),
            dragging: None,
            should_exit: false,
        }
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

    fn save_preferences(&self) {
        *DESKTOP_PREFERENCES.lock() = self.preferences;
    }

    fn work_area_bottom(&self) -> i16 {
        if self.preferences.taskbar_visible {
            TASKBAR_Y
        } else {
            framebuffer::HEIGHT as i16
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

    fn handle_pointer(&mut self, pointer: PointerEvent) -> bool {
        let speed = self.preferences.pointer_speed as i16;
        let motion_x = pointer.dx.saturating_mul(speed);
        let motion_y = pointer.dy.saturating_mul(speed);
        self.cursor.move_by(motion_x, motion_y);
        self.route_pointer(pointer);
        if pointer.released & 1 != 0 {
            self.dragging = None;
        }
        if pointer.buttons & 1 != 0
            && self.dragging.is_some()
            && (pointer.dx != 0 || pointer.dy != 0)
        {
            self.drag_active(motion_x, motion_y);
            return true;
        }
        if pointer.pressed & 1 != 0 {
            return self.pointer_press(self.cursor.x, self.cursor.y);
        }
        false
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
        if self.preferences.taskbar_visible && y >= TASKBAR_Y {
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
                && (LAUNCHER_Y..LAUNCHER_Y + LAUNCHER_HEIGHT as i16).contains(&y)
            {
                for (index, app) in AppKind::ALL.iter().copied().enumerate() {
                    let left = LAUNCHER_X + 8;
                    let top = LAUNCHER_Y + 48 + index as i16 * 32;
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
        let max_x = (framebuffer::WIDTH as i32 - rect.width as i32).max(0);
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
            Rect::new(10, 10, framebuffer::WIDTH as u16 - 20, bottom as u16 - 20)
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
        let max_x = (framebuffer::WIDTH as i32 - rect.width as i32 - 8).max(8);
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
        let surface = self.app_surfaces[AppKind::Browser.index()];
        let _ = self
            .server
            .damage(BROWSER_FIN, surface, Rect::new(0, 0, APP_WIDTH, APP_HEIGHT));
        let _ = self.server.commit(BROWSER_FIN, surface);
    }

    fn navigate_address(&mut self, address: &str) {
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
            self.navigate("hexa://error", NETWORK_ERROR);
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
        match network::http_get(&self.broker, handle_id, BROWSER_FIN, STABLE_FIN, address) {
            Ok(response) => {
                let mut sanitized = [0_u8; network::HTTP_BODY_CAPACITY];
                for (output, byte) in sanitized.iter_mut().zip(response.body().iter().copied()) {
                    *output = if byte.is_ascii_graphic()
                        || matches!(byte, b' ' | b'\n' | b'\r' | b'\t')
                    {
                        byte
                    } else {
                        b' '
                    };
                }
                let source = core::str::from_utf8(&sanitized[..response.body_len]).unwrap_or("");
                let document = Document::parse(address, source).or_else(|_| {
                    let mut wrapped = [0_u8; network::HTTP_BODY_CAPACITY + 7];
                    wrapped[..3].copy_from_slice(b"<p>");
                    wrapped[3..3 + response.body_len]
                        .copy_from_slice(&sanitized[..response.body_len]);
                    wrapped[3 + response.body_len..7 + response.body_len].copy_from_slice(b"</p>");
                    let fallback = core::str::from_utf8(&wrapped[..7 + response.body_len])
                        .unwrap_or("<p>Invalid response body</p>");
                    Document::parse(address, fallback)
                });
                match document {
                    Ok(document) => {
                        self.set_document(document);
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
        if (12..SETTINGS_SIDEBAR_WIDTH).contains(&local_x) && local_y >= SETTINGS_CATEGORY_TOP {
            let index = ((local_y - SETTINGS_CATEGORY_TOP) / SETTINGS_CATEGORY_STEP) as usize;
            if let Some(category) = SettingsCategory::ALL.get(index).copied() {
                if local_y
                    < SETTINGS_CATEGORY_TOP
                        + index as i16 * SETTINGS_CATEGORY_STEP
                        + SETTINGS_CATEGORY_STEP
                        - 4
                {
                    self.select_settings_category(category);
                    return true;
                }
            }
        }

        if local_x >= SETTINGS_SIDEBAR_WIDTH + 24
            && local_x < rect.width as i16 - 20
            && local_y >= SETTINGS_ROW_TOP
        {
            let row = ((local_y - SETTINGS_ROW_TOP) / SETTINGS_ROW_STEP) as usize;
            if row < self.settings_category.row_count()
                && local_y < SETTINGS_ROW_TOP + row as i16 * SETTINGS_ROW_STEP + SETTINGS_ROW_HEIGHT
            {
                self.settings_row = row;
                self.activate_setting(1);
                return true;
            }
        }
        false
    }

    fn handle_settings_key(&mut self, key: u8) -> bool {
        match key {
            KEY_LEFT | b'[' => self.shift_settings_category(-1),
            KEY_RIGHT | b']' => self.shift_settings_category(1),
            KEY_UP => {
                self.settings_row = self.settings_row.saturating_sub(1);
            }
            KEY_DOWN => {
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
                            framebuffer::WIDTH as u16 - 20,
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
                self.preferences.accent = self.preferences.accent.shifted(direction);
                slog!(
                    "HEXA_SETTING_CHANGED key=accent value={}\r\n",
                    self.preferences.accent.label()
                );
                self.settings_notice = "Accent color updated.";
            }
            (SettingsCategory::Appearance, 1) => {
                self.preferences.backdrop = self.preferences.backdrop.shifted(direction);
                slog!(
                    "HEXA_SETTING_CHANGED key=background value={}\r\n",
                    self.preferences.backdrop.label()
                );
                self.settings_notice = "Desktop background updated.";
            }
            (SettingsCategory::Appearance, 2) => {
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
            (SettingsCategory::Appearance, 3) => {
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
            (SettingsCategory::Network, 0) => self.change_network_policy(),
            (SettingsCategory::Network, 2) => self.change_radio_policy(radio::RadioKind::Wifi),
            (SettingsCategory::Bluetooth, 0) => {
                self.change_radio_policy(radio::RadioKind::Bluetooth)
            }
            (SettingsCategory::Display, 2) => {
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
            (SettingsCategory::Display, 3) => {
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
                let mut value = [0_u8; 96];
                value[..self.browser_len].copy_from_slice(&self.browser_line[..self.browser_len]);
                let address = core::str::from_utf8(&value[..self.browser_len]).unwrap_or("");
                self.navigate_address(address);
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

    fn set_terminal_message(&mut self, value: &str) {
        let bytes = value.as_bytes();
        let count = bytes.len().min(self.terminal_message.len());
        self.terminal_message[..count].copy_from_slice(&bytes[..count]);
        self.terminal_message_len = count;
    }

    fn handle_terminal_key(&mut self, key: u8) {
        match key {
            b'\n' => {
                let command = self.terminal_line;
                let command_len = self.terminal_len;
                self.record_terminal_history(&command[..command_len]);
                let action = terminal_action(&command[..command_len]);
                self.terminal_len = 0;
                self.terminal_history_cursor = None;
                match action {
                    TerminalAction::Message(message) => self.set_terminal_message(message),
                    TerminalAction::Clear => self.terminal_message_len = 0,
                    TerminalAction::Close => self.close_active(),
                    TerminalAction::Exit => self.should_exit = true,
                }
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
}

enum TerminalAction {
    Message(&'static str),
    Clear,
    Close,
    Exit,
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
    render(&mut desktop);
    slog!("HEXA_DISPLAY_READY surfaces=11 commit=11\r\n");
    if start_app.is_none() {
        slog!("HEXA_DESKTOP_EMPTY open_apps=0 pinned_apps=0\r\n");
    }
    slog!("HEXA_MOUSE_READY enabled={}\r\n", mouse_ready);

    while !desktop.should_exit {
        let Some(event) = input.poll_event() else {
            if desktop.active == AppKind::Games
                && desktop.app_is_visible(AppKind::Games)
                && desktop.games.tick(crate::hardware::timestamp())
            {
                render_active_window(&mut desktop);
            }
            core::hint::spin_loop();
            continue;
        };
        let InputEvent::Key(key) = event else {
            if let InputEvent::Pointer(pointer) = event {
                if desktop.handle_pointer(pointer) {
                    render(&mut desktop);
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
            render(&mut desktop);
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

fn terminal_action(command: &[u8]) -> TerminalAction {
    let trimmed = trim_ascii(command);
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(b"help") {
        TerminalAction::Message("Commands: help status clear close exit")
    } else if trimmed.eq_ignore_ascii_case(b"status") {
        TerminalAction::Message("ExpOS is ready.")
    } else if trimmed.eq_ignore_ascii_case(b"clear") {
        TerminalAction::Clear
    } else if trimmed.eq_ignore_ascii_case(b"close") {
        TerminalAction::Close
    } else if trimmed.eq_ignore_ascii_case(b"exit") || trimmed.eq_ignore_ascii_case(b"shell") {
        TerminalAction::Exit
    } else {
        TerminalAction::Message("Unknown command. Type help.")
    }
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
    let centered_x = ((framebuffer::WIDTH as i32 - APP_WIDTH as i32) / 2).max(8) as i16;
    let centered_y = ((TASKBAR_Y as i32 - APP_HEIGHT as i32) / 2).max(8) as i16;
    Rect::new(
        centered_x + offset * 18,
        centered_y + offset * 8,
        APP_WIDTH,
        APP_HEIGHT,
    )
}

fn render(desktop: &mut DesktopState) {
    desktop.cursor.invalidate();
    framebuffer::clear(desktop.preferences.backdrop.color());

    for app in AppKind::ALL {
        if app != desktop.active && desktop.app_is_visible(app) {
            draw_app(desktop, app, false);
        }
    }
    if desktop.app_is_visible(desktop.active) {
        draw_app(desktop, desktop.active, true);
    }
    if desktop.preferences.taskbar_visible {
        draw_dock(desktop);
    }
    if desktop.launcher_open {
        draw_launcher(desktop);
    }
    desktop.cursor.draw();
    desktop.drain_protocol_events();
}

fn render_active_window(desktop: &mut DesktopState) {
    desktop.cursor.restore();
    if desktop.app_is_visible(desktop.active) {
        draw_app(desktop, desktop.active, true);
    }
    desktop.cursor.draw();
    desktop.drain_protocol_events();
}

fn draw_app(desktop: &DesktopState, app: AppKind, focused: bool) {
    let rect = desktop
        .server
        .surface(desktop.app_surfaces[app.index()])
        .map(|surface| surface.current.rect)
        .unwrap_or(Rect::new(48, 58, APP_WIDTH, APP_HEIGHT));
    draw_window(rect, app.label(), focused, desktop.preferences);
    let responsive_full = matches!(app, AppKind::Browser | AppKind::Terminal)
        && rect.width >= 480
        && rect.height >= 430;
    if responsive_full || (rect.width >= 620 && rect.height >= 430) {
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

fn draw_window(rect: Rect, title: &str, focused: bool, preferences: DesktopPreferences) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(x, y, width, height, preferences.window_color());
    if preferences.window_borders {
        framebuffer::outline(x, y, width, height, preferences.border_color(focused));
    }
    if focused {
        framebuffer::rect(x + 1, y + 1, width - 2, 2, preferences.accent.color());
    }
    framebuffer::rect(x + 1, y + 3, width - 2, 29, color::PANEL);
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
    framebuffer::text(x + width - 109, y + 12, "-", color::MUTED, 1);
    framebuffer::outline(x + width - 69, y + 11, 12, 10, color::MUTED);
    framebuffer::text(x + width - 25, y + 12, "x", color::MUTED, 1);
}

fn draw_browser(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let bottom = y + rect.height as i32;
    framebuffer::rect(x + 18, y + 50, 40, 36, color::PANEL);
    framebuffer::outline(x + 18, y + 50, 40, 36, color::BORDER);
    framebuffer::text(x + 34, y + 64, "<", color::INK, 1);
    framebuffer::rect(x + 66, y + 50, width - 86, 36, color::PANEL);
    framebuffer::outline(x + 66, y + 50, width - 86, 36, color::BORDER);
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
    framebuffer::text(x + 96, y + 64, address, color::INK, 1);
    if desktop.browser_editing {
        framebuffer::rect(
            x + 96 + address.len() as i32 * framebuffer::text_advance(1),
            y + 61,
            2,
            14,
            color::GREEN,
        );
    }
    let mut content_y = y + 110;
    for node in desktop.document.nodes() {
        if content_y > bottom - 24 {
            break;
        }
        match node.kind {
            NodeKind::Title => {}
            NodeKind::Heading => {
                framebuffer::text(x + 34, content_y, node.text.as_str(), color::INK, 2);
                content_y += 31;
            }
            NodeKind::Paragraph => {
                content_y = wrapped_text(
                    x + 34,
                    content_y,
                    width - 80,
                    node.text.as_str(),
                    color::INK,
                    1,
                ) + 12;
            }
            NodeKind::Link => {
                framebuffer::text(x + 38, content_y, ">", color::GREEN, 1);
                content_y = wrapped_text(
                    x + 54,
                    content_y,
                    width - 105,
                    node.text.as_str(),
                    color::INK,
                    1,
                ) + 10;
            }
            NodeKind::ListItem => {
                framebuffer::rect(x + 40, content_y + 3, 4, 4, color::GREEN);
                content_y = wrapped_text(
                    x + 54,
                    content_y,
                    width - 105,
                    node.text.as_str(),
                    color::INK,
                    1,
                ) + 10;
            }
        }
    }
}

fn draw_terminal(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(x + 1, y + 32, width - 2, height - 33, 0x0009_0B0B);
    if desktop.terminal_message_len > 0 {
        let message =
            core::str::from_utf8(&desktop.terminal_message[..desktop.terminal_message_len])
                .unwrap_or("Invalid terminal output");
        wrapped_text(x + 20, y + 54, width - 40, message, color::INK, 1);
    }
    if height >= 360 {
        for reverse_index in (0..3).rev() {
            if let Some(command) = desktop.terminal_history_entry(reverse_index) {
                let row = y + 116 + (2 - reverse_index) as i32 * 20;
                framebuffer::text(x + 20, row, "$", color::GREEN, 1);
                framebuffer::text(x + 36, row, command, color::MUTED, 1);
            }
        }
    }
    let prompt_y = y + height - 34;
    let advance = framebuffer::text_advance(2);
    framebuffer::text(x + 20, prompt_y, desktop.session.name(), color::GREEN, 2);
    framebuffer::text(
        x + 20 + desktop.session.name().len() as i32 * advance,
        prompt_y,
        "@expos $",
        color::GREEN,
        2,
    );
    if let Ok(line) = core::str::from_utf8(&desktop.terminal_line[..desktop.terminal_len]) {
        let input_x = x + 20 + (desktop.session.name().len() as i32 + 8) * advance;
        framebuffer::text(input_x, prompt_y, line, color::WHITE, 2);
        framebuffer::rect(
            input_x + line.len() as i32 * advance,
            prompt_y,
            8,
            16,
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
    let sidebar_width = SETTINGS_SIDEBAR_WIDTH as i32;
    let accent = desktop.preferences.accent.color();
    let connectivity = radio::snapshot();
    let can_configure_radios = desktop.settings_radio_handle.is_some();

    framebuffer::rect(x + 1, y + 32, sidebar_width, height - 33, 0x0008_0A0D);
    framebuffer::line(
        x + sidebar_width,
        y + 32,
        x + sidebar_width,
        y + height - 1,
        desktop.preferences.border_color(false),
    );
    framebuffer::text(x + 20, y + 54, "Settings", color::INK, 2);

    for category in SettingsCategory::ALL {
        let row_y = y
            + SETTINGS_CATEGORY_TOP as i32
            + category.index() as i32 * SETTINGS_CATEGORY_STEP as i32;
        let selected = category == desktop.settings_category;
        if selected {
            settings_rect(
                desktop.preferences,
                x + 12,
                row_y,
                sidebar_width - 24,
                SETTINGS_CATEGORY_STEP as i32 - 4,
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
        y + 55,
        desktop.settings_category.label(),
        color::INK,
        2,
    );
    framebuffer::text(
        content_x,
        y + 82,
        desktop.settings_category.description(),
        color::MUTED,
        1,
    );

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
                1,
                "Desktop background",
                "Choose a clean solid desktop color",
                desktop.preferences.backdrop.label(),
                SettingControl::Choice,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
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
                3,
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
                "Current HexaDisplay framebuffer mode",
                DISPLAY_MODE_LABEL,
                SettingControl::Plain,
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                1,
                "Renderer",
                "Direct XRGB8888 software composition",
                "HexaDisplay",
                SettingControl::Status { ready: true },
            );
            settings_row(
                desktop,
                content_x,
                y,
                content_width,
                2,
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
                3,
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
                2,
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
                y + SETTINGS_ROW_TOP as i32 + 3 * SETTINGS_ROW_STEP as i32 + 31,
                "#",
                color::MUTED,
                1,
            );
            draw_number(
                content_x + content_width - 28,
                y + SETTINGS_ROW_TOP as i32 + 3 * SETTINGS_ROW_STEP as i32 + 31,
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
    let y = window_y + SETTINGS_ROW_TOP as i32 + index as i32 * SETTINGS_ROW_STEP as i32;
    let selected = desktop.settings_row == index;
    let accent = desktop.preferences.accent.color();
    settings_rect(
        desktop.preferences,
        x,
        y,
        width,
        SETTINGS_ROW_HEIGHT as i32,
        6,
        if selected { 0x0019_2024 } else { 0x000D_1114 },
    );
    if selected {
        framebuffer::outline(x, y, width, SETTINGS_ROW_HEIGHT as i32, accent);
    }
    framebuffer::text(x + 16, y + 12, label, color::INK, 1);
    framebuffer::text(x + 16, y + 33, detail, color::MUTED, 1);

    match control {
        SettingControl::Toggle { on, available } => {
            let control_x = x + width - 62;
            settings_rect(
                desktop.preferences,
                control_x,
                y + 17,
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
                y + 20,
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
                y + 23,
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
                y + 12,
                control_width,
                30,
                5,
                0x0018_1D21,
            );
            framebuffer::outline(control_x, y + 12, control_width, 30, color::BORDER);
            framebuffer::text(control_x + 10, y + 22, "<", color::MUTED, 1);
            framebuffer::text(control_x + 30, y + 22, value, color::INK, 1);
            framebuffer::text(control_x + control_width - 18, y + 22, ">", color::MUTED, 1);
        }
        SettingControl::Status { ready } => {
            let value_x = x + width - 20 - value.len() as i32 * framebuffer::text_advance(1);
            let marker_x = value_x - 16;
            framebuffer::rect(
                marker_x,
                y + 24,
                7,
                7,
                if ready { accent } else { color::MUTED },
            );
            framebuffer::text(value_x, y + 23, value, color::MUTED, 1);
        }
        SettingControl::Action { available } => {
            let button_x = x + width - 104;
            settings_rect(
                desktop.preferences,
                button_x,
                y + 12,
                84,
                30,
                5,
                if available { accent } else { color::BORDER },
            );
            framebuffer::text(button_x + 20, y + 22, value, color::WHITE, 1);
        }
        SettingControl::Plain => {
            let value_x = x + width - 20 - value.len() as i32 * framebuffer::text_advance(1);
            framebuffer::text(value_x, y + 23, value, color::MUTED, 1);
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
        DISPLAY_MODE_LABEL,
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
        "Go ABI",
        "Version 1",
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
        "DNS + TCP + HTTP",
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
}

fn metric(x: i32, y: i32, width: i32, label: &str, value: &str, accent: u32) {
    framebuffer::rect(x, y, width, 60, 0x0015_1922);
    framebuffer::rect(x, y, 5, 60, accent);
    framebuffer::text(x + 18, y + 13, label, color::MUTED, 1);
    framebuffer::text(x + 18, y + 34, value, accent, 1);
}

fn draw_dock(desktop: &DesktopState) {
    let y = TASKBAR_Y as i32;
    let accent = desktop.preferences.accent.color();
    framebuffer::alpha_rect(
        0,
        y,
        framebuffer::WIDTH as i32,
        TASKBAR_HEIGHT as i32,
        0x0010_141D,
        244,
    );
    framebuffer::rect(0, y, framebuffer::WIDTH as i32, 1, color::BORDER);
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
            framebuffer::WIDTH as i32 - 22,
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
    let y = LAUNCHER_Y as i32;
    let width = LAUNCHER_WIDTH as i32;
    let height = LAUNCHER_HEIGHT as i32;
    framebuffer::rounded_rect(x, y, width, height, 9, color::PANEL);
    framebuffer::outline(x, y, width, height, color::BORDER);
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
                color::PANEL
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
