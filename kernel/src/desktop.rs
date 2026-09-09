use crate::{
    framebuffer,
    input::{
        Input, InputEvent, PointerEvent, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_SUPER_BROWSER,
        KEY_SUPER_CLOSE, KEY_SUPER_CYCLE, KEY_SUPER_DOWN, KEY_SUPER_FULLSCREEN, KEY_SUPER_LAUNCHER,
        KEY_SUPER_LEFT, KEY_SUPER_RIGHT, KEY_SUPER_TERMINAL, KEY_SUPER_UP, KEY_UP,
    },
    slog,
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
const APP_WIDTH: u16 = 860;
const APP_HEIGHT: u16 = 610;
const BUFFER_WIDTH: u16 = 1024;
const BUFFER_HEIGHT: u16 = 712;
const TERMINAL_HISTORY: usize = 12;
const TERMINAL_CAPACITY: usize = 48;
const NOTES_CAPACITY: usize = 2048;
const CURSOR_WIDTH: usize = 14;
const CURSOR_HEIGHT: usize = 20;
const TASKBAR_Y: i16 = 712;
const TASKBAR_HEIGHT: i16 = 56;
const START_X: i16 = 14;
const TASK_ICON_X: i16 = 66;
const TASK_ICON_STEP: i16 = 46;
const STABLE_FIN: Fin = Fin::from_u128(0x4449_4D00_0000_0000_0000_0000_0000_0001);

const HOME: &str = "<title>Hexa Home</title><h1>Welcome to ExpOS</h1><p>A Form-native local document browser with a real address field, mouse controls and bounded rendering.</p><h2>Explore</h2><a href='hexa://about'>About this browser</a><a href='hexa://packages'>Ayo v3 packages</a><a href='hexa://system'>System status</a><p>Use the toolbar or type a hexa address. Internet pages remain unavailable until TCP and TLS land.</p>";
const ABOUT: &str = "<title>About</title><h1>Prism Browser</h1><p>This native Interface Form parses bounded local HTML and renders it through HexaDisplay.</p><p>The address field and toolbar are clickable. Surface state stays owner-scoped and becomes visible only after an atomic commit.</p><p>RTL8139 IPv4 and ICMP exist, but DNS TCP TLS CSS JavaScript and media are not falsely claimed as complete.</p><a href='hexa://home'>Home</a>";
const BROWSER_PACKAGES: &str = "<title>Packages</title><h1>Ayo v3 Package Forms</h1><p>Ayo v3 resolves dependencies, verifies checksums and signatures, downloads artifacts and materializes owned files transactionally.</p><li>PrismDE RenderKit and MouseKit</li><li>SessionManager and TextLab Notes</li><li>GameHub Snake and Pong</li><li>DeveloperKit and Go SDK</li><p>The built-in registry contains 21 packages and remote HTTPS registries are supported by the host TUI.</p><a href='hexa://home'>Home</a>";
const BROWSER_SYSTEM: &str = "<title>System</title><h1>System Scope</h1><p>HexaDisplay protocol version 1 is active.</p><li>1024 by 768 XRGB framebuffer</li><li>XRGB ARGB and RGB565 buffer protocols</li><li>Gradient alpha rounded and line primitives</li><li>Atomic commit focus keyboard and pointer routing</li><li>PS2 mouse keyboard and serial input</li><li>RTL8139 Ethernet ARP IPv4 and ICMP echo</li><p>Web transport still needs DNS TCP and TLS.</p><a href='hexa://home'>Home</a>";
const NETWORK_BLOCKED: &str = "<title>Web unavailable</title><h1>Internet page loading is not ready</h1><p>The native RTL8139 driver can exchange Ethernet ARP IPv4 and ICMP echo packets.</p><p>This browser does not pretend ICMP is the web: external pages require DNS TCP TLS and an HTTP engine.</p><p>Use ping in the command environment to test the current network path.</p><a href='hexa://home'>Home</a>";

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

    const fn summary(self) -> &'static str {
        match self {
            Self::Browser => "LOCAL HTML + POLICY NAVIGATION",
            Self::Terminal => "COMMAND INTERFACE FORM",
            Self::Forms => "FIN REGISTRY + RELATIONSHIPS",
            Self::Packages => "AYO CATALOG + TRANSACTIONS",
            Self::Settings => "PIMP SESSION SPECIFICATIONS",
            Self::System => "KERNEL + CAPABILITY SCOPE",
            Self::Games => "NATIVE SNAKE + PONG",
            Self::Notes => "PRIVATE IN-MEMORY TEXT EDITOR",
        }
    }

    const fn accent(self) -> u32 {
        match self {
            Self::Browser => color::CYAN,
            Self::Terminal => color::GREEN,
            Self::Forms => color::PURPLE,
            Self::Packages => 0x00F4_B942,
            Self::Settings => 0x00E9_69A7,
            Self::System => color::RED,
            Self::Games => 0x00FF_5CC8,
            Self::Notes => 0x00F6_C453,
        }
    }
}

struct DesktopState {
    server: DisplayServer,
    broker: CapabilityBroker,
    app_surfaces: [u32; APP_COUNT],
    app_handles: [u32; APP_COUNT],
    app_open: [bool; APP_COUNT],
    app_ever_opened: [bool; APP_COUNT],
    app_minimized: [bool; APP_COUNT],
    launcher_surface: u32,
    active: AppKind,
    launcher_open: bool,
    fullscreen: bool,
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
    fn new(start_app: Option<AppKind>, session: crate::session::Session) -> Self {
        let active = start_app.unwrap_or(AppKind::Terminal);
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
                "Prism Taskbar",
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

        let launcher_surface = server
            .create_surface(
                DISPLAY_FIN,
                "ExpOS Start",
                SurfaceRole::Popup,
                Rect::new(18, 260, 500, 440),
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
        let _ = server.attach(
            DISPLAY_FIN,
            launcher_surface,
            buffer(20, DISPLAY_FIN, 500, 440),
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
            app_open: core::array::from_fn(|index| start_app == Some(AppKind::ALL[index])),
            app_ever_opened: core::array::from_fn(|index| start_app == Some(AppKind::ALL[index])),
            app_minimized: [false; APP_COUNT],
            launcher_surface,
            active,
            launcher_open: false,
            fullscreen: false,
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

    fn route_key(&mut self, key: u8) {
        if !self.launcher_open
            && self.app_is_visible(self.active)
            && self.authorized(self.active, Operations::INPUT)
        {
            let _ = self.server.route_key(key);
        }
    }

    fn handle_pointer(&mut self, pointer: PointerEvent) -> bool {
        self.cursor.move_by(pointer.dx, pointer.dy);
        self.route_pointer(pointer);
        if pointer.released & 1 != 0 {
            self.dragging = None;
        }
        if pointer.buttons & 1 != 0
            && self.dragging.is_some()
            && (pointer.dx != 0 || pointer.dy != 0)
        {
            self.drag_active(pointer.dx, pointer.dy);
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
        if y >= TASKBAR_Y {
            if (START_X..START_X + 42).contains(&x) {
                self.toggle_launcher();
                return true;
            }
            let mut running_index = 0_i16;
            for app in AppKind::ALL {
                if !self.app_open[app.index()] {
                    continue;
                }
                let left = TASK_ICON_X + running_index * TASK_ICON_STEP;
                if (left..left + 38).contains(&x) {
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
            if (18..518).contains(&x) && (260..700).contains(&y) {
                for (index, app) in AppKind::ALL.iter().copied().enumerate() {
                    let column = index % 2;
                    let row = index / 2;
                    let left = 42 + column as i16 * 226;
                    let top = 342 + row as i16 * 67;
                    if (left..left + 208).contains(&x) && (top..top + 54).contains(&y) {
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
        let max_y = (TASKBAR_Y as i32 - rect.height as i32).max(0);
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
        let rect = if self.fullscreen {
            Rect::new(
                10,
                10,
                framebuffer::WIDTH as u16 - 20,
                TASKBAR_Y as u16 - 20,
            )
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
        let max_y = (TASKBAR_Y as i32 - rect.height as i32).max(8);
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
            self.document = document;
            let _ = self.server.damage(
                BROWSER_FIN,
                self.app_surfaces[AppKind::Browser.index()],
                Rect::new(0, 0, APP_WIDTH, APP_HEIGHT),
            );
            let _ = self
                .server
                .commit(BROWSER_FIN, self.app_surfaces[AppKind::Browser.index()]);
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
        if (98..130).contains(&local_y) {
            let width = (rect.width as i16 - 40) / 4;
            let index = ((local_x - 20).max(0) / width).min(3);
            match index {
                0 => self.navigate("hexa://home", HOME),
                1 => self.navigate("hexa://about", ABOUT),
                2 => self.navigate("hexa://packages", BROWSER_PACKAGES),
                _ => self.navigate("hexa://system", BROWSER_SYSTEM),
            }
            self.browser_editing = false;
            return true;
        }
        false
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
                if address.eq_ignore_ascii_case("hexa://home") || address == "home" {
                    self.navigate("hexa://home", HOME);
                } else if address.eq_ignore_ascii_case("hexa://about") || address == "about" {
                    self.navigate("hexa://about", ABOUT);
                } else if address.eq_ignore_ascii_case("hexa://packages") || address == "packages" {
                    self.navigate("hexa://packages", BROWSER_PACKAGES);
                } else if address.eq_ignore_ascii_case("hexa://system") || address == "system" {
                    self.navigate("hexa://system", BROWSER_SYSTEM);
                } else {
                    self.navigate("hexa://blocked", NETWORK_BLOCKED);
                }
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
    run_session(
        input,
        if start_browser {
            Some(AppKind::Browser)
        } else {
            None
        },
        session,
    );
}

pub fn run_games(input: &mut Input, session: crate::session::Session) {
    run_session(input, Some(AppKind::Games), session);
}

fn run_session(input: &mut Input, start_app: Option<AppKind>, session: crate::session::Session) {
    let mouse_ready = input.enable_mouse();
    if !framebuffer::enter() {
        crate::println!("HexaDisplay unavailable: no Bochs/QEMU VBE framebuffer.");
        slog!("HEXA_DISPLAY_UNAVAILABLE\r\n");
        return;
    }

    let mut desktop = DesktopState::new(start_app, session);
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
        TerminalAction::Message("COMMANDS: HELP STATUS CLEAR CLOSE EXIT")
    } else if trimmed.eq_ignore_ascii_case(b"status") {
        TerminalAction::Message("EXPOS ONLINE. LOCAL GRAPHICS AND SESSION ARE READY.")
    } else if trimmed.eq_ignore_ascii_case(b"clear") {
        TerminalAction::Clear
    } else if trimmed.eq_ignore_ascii_case(b"close") {
        TerminalAction::Close
    } else if trimmed.eq_ignore_ascii_case(b"exit") || trimmed.eq_ignore_ascii_case(b"shell") {
        TerminalAction::Exit
    } else {
        TerminalAction::Message("UNKNOWN COMMAND. TYPE HELP.")
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
    Rect::new(64 + offset * 22, 76 + offset * 8, APP_WIDTH, APP_HEIGHT)
}

fn render(desktop: &mut DesktopState) {
    desktop.cursor.invalidate();
    framebuffer::vertical_gradient(0, 0, 1024, 712, 0x0004_0710, 0x0010_1320);
    framebuffer::alpha_rect(570, 72, 360, 500, 0x005B_35B5, 52);
    framebuffer::alpha_rect(650, 130, 220, 390, 0x0000_A9C6, 30);
    for offset in 0..9 {
        let x = 560 + offset * 48;
        framebuffer::line(x, 76, x - 210, 636, 0x0025_2944);
    }
    framebuffer::line(338, 636, 950, 325, 0x003D_3464);
    framebuffer::rounded_rect(36, 34, 52, 52, 14, 0x0014_1924);
    framebuffer::rounded_rect(47, 45, 30, 30, 9, color::PURPLE);
    framebuffer::text(58, 56, "E", color::WHITE, 1);
    framebuffer::text(104, 44, "EXPOS PRISM", color::INK, 2);
    framebuffer::text(104, 69, "FORM-NATIVE DESKTOP", color::MUTED, 1);
    if !AppKind::ALL.iter().any(|app| desktop.app_open[app.index()]) {
        framebuffer::text(42, 622, "NO APPS RUNNING", color::MUTED, 1);
        framebuffer::text(42, 644, "OPEN START TO BEGIN", color::INK, 1);
    }
    draw_panel(desktop);

    for app in AppKind::ALL {
        if app != desktop.active && desktop.app_is_visible(app) {
            draw_app(desktop, app, false);
        }
    }
    if desktop.app_is_visible(desktop.active) {
        draw_app(desktop, desktop.active, true);
    }
    draw_dock(desktop);
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
    draw_window(rect, app.title(), focused, desktop.app_handles[app.index()]);
    let responsive_full = matches!(app, AppKind::Browser | AppKind::Terminal)
        && rect.width >= 480
        && rect.height >= 430;
    if responsive_full || (rect.width >= 620 && rect.height >= 430) {
        match app {
            AppKind::Browser => draw_browser(rect, desktop),
            AppKind::Terminal => draw_terminal(rect, desktop),
            AppKind::Forms => draw_forms(rect),
            AppKind::Packages => draw_packages(rect),
            AppKind::Settings => draw_settings(rect),
            AppKind::System => draw_system(rect, desktop),
            AppKind::Games => desktop.games.render(rect),
            AppKind::Notes => draw_notes(rect, desktop),
        }
    } else {
        draw_compact_app(rect, app, desktop, focused);
    }
}

fn draw_panel(desktop: &DesktopState) {
    framebuffer::rounded_rect(788, 18, 206, 42, 11, 0x0010_1520);
    framebuffer::outline(788, 18, 206, 42, color::BORDER);
    framebuffer::rect(804, 30, 7, 7, color::GREEN);
    framebuffer::text(821, 29, desktop.session.name(), color::INK, 1);
    framebuffer::text(821, 45, desktop.session.authority_name(), color::MUTED, 1);
    framebuffer::text(940, 35, "LOCAL", color::CYAN, 1);
}

fn draw_window(rect: Rect, title: &str, focused: bool, handle_id: u32) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rounded_rect(x - 6, y + 7, width + 12, height + 7, 8, 0x0002_0306);
    framebuffer::rect(x, y, width, height, color::WINDOW);
    framebuffer::outline(
        x,
        y,
        width,
        height,
        if focused {
            color::PURPLE
        } else {
            color::BORDER
        },
    );
    if focused {
        framebuffer::outline(x - 1, y - 1, width + 2, height + 2, 0x005E_42A6);
    }
    framebuffer::rect(x, y, width, 36, 0x0015_1922);
    framebuffer::rect(x + 12, y + 10, 16, 16, color::PURPLE);
    framebuffer::text(x + 17, y + 15, "E", color::WHITE, 1);
    framebuffer::text(x + 38, y + 13, title, color::INK, 1);
    if width >= 430 {
        framebuffer::text(x + width - 188, y + 13, "HANDLE", color::MUTED, 1);
        draw_number(x + width - 137, y + 13, handle_id as u64, color::CYAN);
    }
    framebuffer::rect(x + width - 126, y, 42, 36, 0x0015_1922);
    framebuffer::text(x + width - 110, y + 13, "_", color::MUTED, 1);
    framebuffer::rect(x + width - 84, y, 42, 36, 0x0015_1922);
    framebuffer::outline(x + width - 69, y + 11, 12, 10, color::MUTED);
    framebuffer::rect(
        x + width - 42,
        y,
        42,
        36,
        if focused { 0x0022_1823 } else { 0x0015_1922 },
    );
    framebuffer::text(x + width - 25, y + 13, "X", color::RED, 1);
}

fn draw_compact_app(rect: Rect, app: AppKind, desktop: &DesktopState, focused: bool) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(x + 12, y + 48, width - 24, height - 63, 0x0008_0B12);
    framebuffer::rect(x + 25, y + 66, 6, 54, app.accent());
    framebuffer::text(
        x + 44,
        y + 68,
        app.title(),
        app.accent(),
        if width >= 380 { 2 } else { 1 },
    );
    if width >= 330 {
        framebuffer::text(x + 44, y + 98, app.summary(), color::INK, 1);
    }
    framebuffer::text(
        x + 28,
        y + 145,
        if focused { "FOCUSED" } else { "VISIBLE" },
        if focused { color::GREEN } else { color::MUTED },
        1,
    );
    framebuffer::text(x + 28, y + 168, "RUNNING", color::MUTED, 1);
    framebuffer::text(x + 112, y + 168, "LOCAL", color::GREEN, 1);
    framebuffer::text(x + 28, y + 191, "CAPABILITY", color::MUTED, 1);
    draw_number(
        x + 91,
        y + 191,
        desktop.app_handles[app.index()] as u64,
        color::CYAN,
    );
    if height >= 330 {
        framebuffer::text(
            x + 28,
            y + height - 84,
            "SUPER+F FULLSCREEN",
            color::MUTED,
            1,
        );
        framebuffer::text(
            x + 28,
            y + height - 62,
            "DRAG TITLE BAR TO MOVE",
            color::MUTED,
            1,
        );
    }
}

fn draw_browser(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let bottom = y + rect.height as i32;
    framebuffer::rounded_rect(x + 18, y + 50, 40, 36, 8, 0x0018_1D28);
    framebuffer::text(x + 34, y + 63, "<", color::INK, 1);
    framebuffer::rounded_rect(x + 66, y + 50, width - 86, 36, 8, 0x0018_1D28);
    framebuffer::outline(x + 66, y + 50, width - 86, 36, color::BORDER);
    framebuffer::rect(x + 79, y + 62, 8, 8, color::GREEN);
    let address = if desktop.browser_editing {
        core::str::from_utf8(&desktop.browser_line[..desktop.browser_len]).unwrap_or("")
    } else {
        desktop.document.url()
    };
    framebuffer::text(x + 98, y + 63, address, color::INK, 1);
    if desktop.browser_editing {
        framebuffer::rect(
            x + 98 + address.len() as i32 * 6,
            y + 60,
            2,
            13,
            color::PURPLE,
        );
    }
    let tab_width = (width - 40) / 4;
    for (index, label) in ["HOME", "ABOUT", "PACKAGES", "SYSTEM"].iter().enumerate() {
        let tab_x = x + 20 + index as i32 * tab_width;
        framebuffer::rect(tab_x, y + 98, tab_width - 4, 30, 0x0012_1720);
        framebuffer::text(tab_x + 14, y + 109, label, color::MUTED, 1);
    }
    framebuffer::rect(x + 20, y + 137, width - 40, 1, color::BORDER);

    let mut content_y = y + 154;
    for node in desktop.document.nodes() {
        if content_y > bottom - 62 {
            break;
        }
        match node.kind {
            NodeKind::Title => {}
            NodeKind::Heading => {
                framebuffer::text(x + 34, content_y, node.text.as_str(), color::PURPLE, 2);
                content_y += 31;
            }
            NodeKind::Paragraph => {
                content_y = wrapped_text(
                    x + 34,
                    content_y,
                    width - 80,
                    node.text.as_str(),
                    color::INK,
                    2,
                ) + 10;
            }
            NodeKind::Link => {
                framebuffer::text(x + 38, content_y, ">", color::CYAN, 2);
                content_y = wrapped_text(
                    x + 58,
                    content_y,
                    width - 105,
                    node.text.as_str(),
                    color::CYAN,
                    2,
                ) + 7;
            }
            NodeKind::ListItem => {
                framebuffer::rect(x + 40, content_y + 5, 6, 6, color::GREEN);
                content_y = wrapped_text(
                    x + 58,
                    content_y,
                    width - 105,
                    node.text.as_str(),
                    color::INK,
                    2,
                ) + 7;
            }
        }
    }
    app_footer(
        rect,
        "LOCAL DOCUMENT MODE  //  INTERNET NEEDS TCP + TLS  //  CLICK ADDRESS TO TYPE",
    );
}

fn draw_terminal(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(x + 10, y + 42, width - 20, height - 54, 0x0000_0000);
    framebuffer::text(x + 28, y + 61, "ExpOS terminal", color::MUTED, 1);
    framebuffer::text(x + 28, y + 86, "Type help for commands.", color::INK, 1);
    if desktop.terminal_message_len > 0 {
        let message =
            core::str::from_utf8(&desktop.terminal_message[..desktop.terminal_message_len])
                .unwrap_or("INVALID TERMINAL OUTPUT");
        wrapped_text(x + 28, y + 116, width - 56, message, color::INK, 1);
    }
    if height >= 360 {
        framebuffer::text(x + 28, y + 178, "History", color::MUTED, 1);
        for reverse_index in (0..3).rev() {
            if let Some(command) = desktop.terminal_history_entry(reverse_index) {
                let row = y + 202 + (2 - reverse_index) as i32 * 20;
                framebuffer::text(x + 28, row, ">", color::GREEN, 1);
                framebuffer::text(x + 42, row, command, color::MUTED, 1);
            }
        }
    }
    framebuffer::text(
        x + 34,
        y + height - 64,
        desktop.session.name(),
        color::GREEN,
        2,
    );
    framebuffer::text(
        x + 34 + desktop.session.name().len() as i32 * 12,
        y + height - 64,
        "@expos $",
        color::GREEN,
        2,
    );
    if let Ok(line) = core::str::from_utf8(&desktop.terminal_line[..desktop.terminal_len]) {
        let input_x = x + 34 + (desktop.session.name().len() as i32 + 8) * 12;
        framebuffer::text(input_x, y + height - 64, line, color::WHITE, 2);
        framebuffer::rect(
            input_x + line.len() as i32 * 12,
            y + height - 65,
            10,
            17,
            color::PURPLE,
        );
    }
    app_footer(rect, "ENTER RUN  //  UP DOWN HISTORY  //  ESC RETURNS");
}

fn draw_notes(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(x + 12, y + 44, width - 24, 42, 0x000A_0D13);
    framebuffer::text(x + 28, y + 59, "Untitled note", color::INK, 1);
    framebuffer::text(x + width - 170, y + 59, "MEMORY ONLY", color::MUTED, 1);
    framebuffer::rect(x + 12, y + 88, width - 24, height - 122, 0x0000_0000);
    framebuffer::outline(x + 12, y + 88, width - 24, height - 122, color::BORDER);

    if desktop.notes_len == 0 {
        framebuffer::text(x + 32, y + 112, "Start typing...", 0x0056_5E70, 2);
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
    app_footer(
        rect,
        "TYPE TO EDIT  //  BACKSPACE DELETE  //  NOTES RESET AFTER REBOOT",
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
    framebuffer::text(x + 28, y + 58, "FORM REGISTRY", color::PURPLE, 2);
    framebuffer::text(
        x + 28,
        y + 86,
        "STABLE IDENTITIES AND ACTIVE RELATIONSHIPS",
        color::MUTED,
        1,
    );
    let rows = [
        ("ROOT", "DIMENSION", "ACTIVE", color::GREEN),
        ("AYO", "PACKAGE", "BOUND", color::CYAN),
        ("HEXADISPLAY", "SERVICE", "ACTIVE", color::GREEN),
        ("BROWSER", "INTERFACE", "FOCUSED", color::PURPLE),
        ("GO ABI V1", "INTERFACE", "READY", color::CYAN),
        ("HEXAFS", "STORAGE", "JOURNALED", color::GREEN),
    ];
    for (index, (name, kind, status, status_color)) in rows.iter().enumerate() {
        let row_y = y + 116 + index as i32 * 43;
        framebuffer::rect(x + 28, row_y, rect.width as i32 - 56, 34, 0x0015_1922);
        framebuffer::rect(x + 28, row_y, 5, 34, *status_color);
        framebuffer::text(x + 46, row_y + 12, name, color::INK, 1);
        framebuffer::text(x + 260, row_y + 12, kind, color::MUTED, 1);
        framebuffer::text(x + 494, row_y + 12, status, *status_color, 1);
    }
    app_footer(
        rect,
        "EACH WINDOW IS OWNED BY A FORM  TAB NEXT  ARROWS MOVE",
    );
}

fn draw_packages(rect: Rect) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let content_width = rect.width as i32 - 56;
    framebuffer::text(x + 28, y + 58, "AYO PACKAGE CENTER", color::PURPLE, 2);
    framebuffer::text(
        x + 28,
        y + 88,
        "CATALOG SNAPSHOT // POLICY CHECKED",
        color::MUTED,
        1,
    );
    package_card(
        x + 28,
        y + 116,
        content_width,
        "CORETOOLS",
        "INSTALLED",
        "DIAGNOSTICS AND REPAIR",
    );
    package_card(
        x + 28,
        y + 174,
        content_width,
        "HEXADISPLAY + RENDERKIT",
        "INSTALLED",
        "DISPLAY FORM + GRAPHICS PRIMITIVES",
    );
    package_card(
        x + 28,
        y + 232,
        content_width,
        "GAMEHUB + SNAKE + PONG",
        "AVAILABLE",
        "NATIVE PLAYABLE GAME FORMS",
    );
    package_card(
        x + 28,
        y + 290,
        content_width,
        "PRISM + SESSION",
        "AVAILABLE",
        "DARK DE MOUSE AND IDENTITY",
    );
    framebuffer::text(
        x + 34,
        y + 362,
        "AYO V3 DOWNLOADS, VERIFIES AND MATERIALIZES PACKAGE ARTIFACTS.",
        color::INK,
        1,
    );
    framebuffer::text(
        x + 34,
        y + 380,
        "REMOTE HTTPS REGISTRIES AND ROLLBACK RUN IN THE HOST TUI.",
        color::MUTED,
        1,
    );
    app_footer(
        rect,
        "AYO V3 PACKAGE MANAGER  //  VERIFIED ARTIFACTS  //  DEPENDENCY PLANS",
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
        if status == "INSTALLED" {
            color::GREEN
        } else {
            color::CYAN
        },
        1,
    );
}

fn draw_settings(rect: Rect) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let content_width = rect.width as i32 - 56;
    framebuffer::text(x + 28, y + 58, "DESKTOP SETTINGS", color::PURPLE, 2);
    framebuffer::text(x + 28, y + 91, "SESSION", color::MUTED, 1);
    setting_row(
        x + 28,
        y + 112,
        content_width,
        "RENDERER",
        "HEXADISPLAY V1",
        true,
    );
    setting_row(
        x + 28,
        y + 157,
        content_width,
        "RESOLUTION",
        "1024 X 768 XRGB",
        true,
    );
    setting_row(x + 28, y + 202, content_width, "THEME", "PRISM DARK", true);
    setting_row(
        x + 28,
        y + 247,
        content_width,
        "INPUT",
        "PS2 PLUS SERIAL",
        true,
    );
    setting_row(
        x + 28,
        y + 292,
        content_width,
        "USERS",
        "CLI USER MANAGER",
        true,
    );
    setting_row(
        x + 28,
        y + 337,
        content_width,
        "POLICY",
        "DIESE ENFORCED",
        true,
    );
    framebuffer::text(
        x + rect.width as i32 - 180,
        y + 305,
        "SLOTS",
        color::MUTED,
        1,
    );
    draw_number(
        x + rect.width as i32 - 92,
        y + 305,
        crate::session::account_count() as u64,
        color::CYAN,
    );
    app_footer(
        rect,
        "SETTINGS ARE SESSION LOCAL  TAB NEXT  ARROWS MOVE  Q SHELL",
    );
}

fn setting_row(x: i32, y: i32, width: i32, label: &str, value: &str, enabled: bool) {
    framebuffer::rect(x, y, width, 35, 0x0015_1922);
    framebuffer::text(x + 15, y + 13, label, color::INK, 1);
    framebuffer::text(x + 235, y + 13, value, color::MUTED, 1);
    framebuffer::rect(
        x + width - 48,
        y + 11,
        27,
        13,
        if enabled { color::GREEN } else { color::RED },
    );
    framebuffer::rect(
        x + width - if enabled { 32 } else { 46 },
        y + 13,
        9,
        9,
        color::WHITE,
    );
}

fn draw_system(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let metric_width = (rect.width as i32 - 68) / 2;
    let right_x = x + 40 + metric_width;
    framebuffer::text(x + 28, y + 58, "SYSTEM SCOPE", color::PURPLE, 2);
    framebuffer::text(x + 28, y + 88, "LIVE SESSION TELEMETRY", color::MUTED, 1);
    metric(
        x + 28,
        y + 120,
        metric_width,
        "DISPLAY PROTOCOL",
        "VERSION 1",
        color::GREEN,
    );
    metric(
        right_x,
        y + 120,
        metric_width,
        "FRAMEBUFFER",
        "1024 X 768",
        color::CYAN,
    );
    metric(
        x + 28,
        y + 196,
        metric_width,
        "SURFACES",
        "11 FORM OWNED",
        color::PURPLE,
    );
    metric(
        right_x,
        y + 196,
        metric_width,
        "GO ABI",
        "VERSION 1",
        color::GREEN,
    );
    metric(
        x + 28,
        y + 272,
        metric_width,
        "INPUT",
        "PS2 MOUSE + KEYS",
        color::CYAN,
    );
    metric(
        right_x,
        y + 272,
        metric_width,
        "NETWORK",
        "RTL8139 + ICMP",
        color::GREEN,
    );
    framebuffer::text(x + 34, y + 365, "ATOMIC COMMITS", color::MUTED, 1);
    draw_number(
        x + 180,
        y + 365,
        desktop.server.commit_sequence(),
        color::GREEN,
    );
    framebuffer::text(x + 352, y + 365, "ACTIVE FORM", color::MUTED, 1);
    framebuffer::text(x + 478, y + 365, desktop.active.title(), color::CYAN, 1);
    app_footer(
        rect,
        "SYSTEM IS LIVE  TAB NEXT  ARROWS MOVE  \x60 LAUNCHER  Q SHELL",
    );
}

fn metric(x: i32, y: i32, width: i32, label: &str, value: &str, accent: u32) {
    framebuffer::rect(x, y, width, 60, 0x0015_1922);
    framebuffer::rect(x, y, 5, 60, accent);
    framebuffer::text(x + 18, y + 13, label, color::MUTED, 1);
    framebuffer::text(x + 18, y + 34, value, accent, 1);
}

fn app_footer(rect: Rect, text: &str) {
    let x = rect.x as i32;
    let y = rect.y as i32 + rect.height as i32 - 28;
    framebuffer::rect(x + 1, y, rect.width as i32 - 2, 27, 0x0013_1720);
    framebuffer::text(x + 16, y + 10, text, color::MUTED, 1);
}

fn draw_dock(desktop: &DesktopState) {
    let y = TASKBAR_Y as i32;
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
    framebuffer::rounded_rect(START_X as i32, y + 7, 42, 42, 10, start_color);
    framebuffer::rounded_rect(25, y + 18, 20, 20, 6, color::PURPLE);
    framebuffer::text(32, y + 25, "E", color::WHITE, 1);
    let mut running_index = 0_i32;
    for app in AppKind::ALL {
        if !desktop.app_open[app.index()] {
            continue;
        }
        let x = TASK_ICON_X as i32 + running_index * TASK_ICON_STEP as i32;
        if app == desktop.active && desktop.app_is_visible(app) {
            framebuffer::rounded_rect(x, y + 7, 40, 42, 9, 0x0028_213C);
        }
        framebuffer::rounded_rect(x + 9, y + 15, 22, 22, 6, app.accent());
        framebuffer::text(x + 17, y + 23, app.shortcut(), color::WHITE, 1);
        framebuffer::rect(
            x + 9,
            y + 45,
            22,
            2,
            if desktop.app_minimized[app.index()] {
                color::MUTED
            } else {
                color::PURPLE
            },
        );
        running_index += 1;
    }
    framebuffer::rect(838, y + 20, 7, 7, color::GREEN);
    framebuffer::text(852, y + 19, "SYSTEM", color::MUTED, 1);
    framebuffer::text(916, y + 15, desktop.session.name(), color::INK, 1);
    framebuffer::text(
        916,
        y + 32,
        desktop.session.authority_name(),
        color::MUTED,
        1,
    );
}

fn draw_launcher(desktop: &DesktopState) {
    framebuffer::rounded_rect(27, 269, 500, 440, 14, 0x0001_0204);
    framebuffer::rounded_rect(18, 260, 500, 440, 14, 0x0010_141D);
    framebuffer::outline(18, 260, 500, 440, color::BORDER);
    framebuffer::text(42, 282, "APPLICATIONS", color::INK, 2);
    framebuffer::text(42, 310, "OPEN AN APP", color::MUTED, 1);
    framebuffer::rect(42, 327, 452, 1, color::BORDER);
    for (index, app) in AppKind::ALL.iter().copied().enumerate() {
        let column = index % 2;
        let row = index / 2;
        let x = 42 + column as i32 * 226;
        let y = 342 + row as i32 * 67;
        framebuffer::rounded_rect(
            x,
            y,
            208,
            54,
            8,
            if app == desktop.active && desktop.app_open[app.index()] {
                0x0030_2550
            } else {
                0x0018_1D28
            },
        );
        framebuffer::rounded_rect(x + 10, y + 10, 32, 32, 8, app.accent());
        framebuffer::text(x + 22, y + 22, app.shortcut(), color::WHITE, 1);
        framebuffer::text(x + 54, y + 13, app.title(), color::INK, 1);
        framebuffer::text(
            x + 54,
            y + 32,
            if desktop.app_open[app.index()] {
                "RUNNING"
            } else {
                "APP"
            },
            if desktop.app_open[app.index()] {
                color::GREEN
            } else {
                color::MUTED
            },
            1,
        );
    }
    framebuffer::rect(18, 620, 500, 80, 0x0009_0C12);
    framebuffer::rounded_rect(42, 643, 34, 34, 9, color::PURPLE);
    framebuffer::text(54, 655, "U", color::WHITE, 1);
    framebuffer::text(90, 645, desktop.session.name(), color::INK, 1);
    framebuffer::text(90, 663, desktop.session.authority_name(), color::MUTED, 1);
    framebuffer::text(427, 654, "ESC", color::MUTED, 1);
}

fn wrapped_text(mut x: i32, mut y: i32, width: i32, value: &str, color: u32, scale: i32) -> i32 {
    let left = x;
    let advance = 6 * scale;
    for word in value.split_ascii_whitespace() {
        let word_width = word.len() as i32 * advance;
        if x != left && x + word_width > left + width {
            x = left;
            y += 9 * scale;
        }
        framebuffer::text(x, y, word, color, scale);
        x += word_width + advance;
    }
    y + 7 * scale
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
