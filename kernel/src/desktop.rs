use crate::{
    framebuffer,
    input::{
        Input, InputEvent, PointerEvent, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_SUPER_BROWSER,
        KEY_SUPER_CLOSE, KEY_SUPER_CYCLE, KEY_SUPER_DOWN, KEY_SUPER_FLOAT, KEY_SUPER_FULLSCREEN,
        KEY_SUPER_LAUNCHER, KEY_SUPER_LEFT, KEY_SUPER_OVERVIEW, KEY_SUPER_RIGHT,
        KEY_SUPER_TERMINAL, KEY_SUPER_UP, KEY_SUPER_WORKSPACE_1, KEY_SUPER_WORKSPACE_2,
        KEY_SUPER_WORKSPACE_3, KEY_SUPER_WORKSPACE_4, KEY_UP,
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

const APP_COUNT: usize = 7;
const APP_WIDTH: u16 = 704;
const APP_HEIGHT: u16 = 466;
const BUFFER_WIDTH: u16 = 800;
const BUFFER_HEIGHT: u16 = 550;
const TERMINAL_HISTORY: usize = 12;
const CURSOR_WIDTH: usize = 14;
const CURSOR_HEIGHT: usize = 20;
const TASKBAR_Y: i16 = 550;
const START_X: i16 = 204;
const TASK_ICON_X: i16 = 252;
const TASK_ICON_STEP: i16 = 44;
const STABLE_FIN: Fin = Fin::from_u128(0x4449_4D00_0000_0000_0000_0000_0000_0001);

const HOME: &str = "<title>Hexa Home</title><h1>Welcome to ExpOS</h1><p>A Form native desktop where identity capability state and relationships are first class.</p><h2>Explore locally</h2><a href='hexa://about'>1 About this browser</a><a href='hexa://packages'>2 Package Forms</a><a href='hexa://system'>3 System status</a><p>Use 1 2 3 or H inside Browser. External navigation is safely restricted.</p>";
const ABOUT: &str = "<title>About</title><h1>Hexa Browser</h1><p>This native Interface Form parses bounded local HTML and renders it through HexaDisplay.</p><p>Surface state is owner scoped and becomes visible only after an atomic commit.</p><p>HTTPS CSS JavaScript and media are intentionally not claimed before networking isolation and a complete web engine exist.</p><a href='hexa://home'>H Home</a>";
const BROWSER_PACKAGES: &str = "<title>Packages</title><h1>Prism Package Forms</h1><p>Ayo activates software as Forms rather than copying archives into Unix paths.</p><li>PrismDE desktop environment</li><li>MouseKit and SessionManager</li><li>GameHub Snake and Pong</li><li>DeveloperKit Go workspace</li><li>TextLab VirtioBlock and AudioKit</li><p>The built-in catalog contains 20 packages. Use the host ayo TUI to search and install complete dependency plans.</p><a href='hexa://home'>H Home</a>";
const BROWSER_SYSTEM: &str = "<title>System</title><h1>System Scope</h1><p>HexaDisplay protocol version 1 is active.</p><li>800 by 600 XRGB framebuffer</li><li>Atomic attach damage and commit</li><li>Focus z order keyboard and pointer routing</li><li>PS2 mouse keyboard and serial input</li><p>Network Driver Form is not bound. External navigation remains restricted.</p><a href='hexa://home'>H Home</a>";
const NETWORK_BLOCKED: &str = "<title>Network restricted</title><h1>Navigation blocked</h1><p>DIESE denied external navigation because no v8 Network Driver Form is bound and PIMP network policy is restricted.</p><p>This is a safe fallback not a fake internet connection.</p><a href='hexa://home'>H Home</a>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppKind {
    Browser,
    Terminal,
    Forms,
    Packages,
    Settings,
    System,
    Games,
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
        }
    }
}

struct DesktopState {
    server: DisplayServer,
    broker: CapabilityBroker,
    app_surfaces: [u32; APP_COUNT],
    app_handles: [u32; APP_COUNT],
    app_workspaces: [u8; APP_COUNT],
    app_open: [bool; APP_COUNT],
    app_floating: [bool; APP_COUNT],
    launcher_surface: u32,
    active: AppKind,
    workspace: u8,
    launcher_open: bool,
    overview_open: bool,
    fullscreen: bool,
    document: Document,
    terminal_line: [u8; 64],
    terminal_len: usize,
    terminal_message: [u8; 128],
    terminal_message_len: usize,
    terminal_history: [[u8; 64]; TERMINAL_HISTORY],
    terminal_history_len: [u8; TERMINAL_HISTORY],
    terminal_history_next: usize,
    terminal_history_count: usize,
    terminal_history_cursor: Option<usize>,
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
            x: 400,
            y: 300,
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
    fn new(start_browser: bool, session: crate::session::Session) -> Self {
        let active = if start_browser {
            AppKind::Browser
        } else {
            AppKind::Terminal
        };
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
                Rect::new(0, 0, 800, 600),
            )
            .expect("desktop background surface");
        let panel = server
            .create_surface(
                DISPLAY_FIN,
                "Prism Taskbar",
                SurfaceRole::Panel,
                Rect::new(0, TASKBAR_Y, 800, 50),
            )
            .expect("desktop panel surface");

        let mut app_surfaces = [0; APP_COUNT];
        let mut app_handles = [0; APP_COUNT];
        for (index, app) in AppKind::ALL.iter().copied().enumerate() {
            let primary = if index < 2 {
                app == active
            } else {
                index % 2 == 0
            };
            let rect = if primary {
                Rect::new(24, 24, 510, 514)
            } else {
                Rect::new(546, 24, 230, 514)
            };
            let surface = server
                .create_surface(app.owner(), app.title(), SurfaceRole::Window, rect)
                .expect("built-in application surface");
            let _ = server.attach(
                app.owner(),
                surface,
                buffer((index + 3) as u32, app.owner(), BUFFER_WIDTH, BUFFER_HEIGHT),
            );
            let _ = server.set_visible(app.owner(), surface, index < 2);
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
                Rect::new(190, 112, 420, 428),
            )
            .expect("desktop launcher surface");
        let _ = server.attach(DISPLAY_FIN, background, buffer(1, DISPLAY_FIN, 800, 600));
        let _ = server.attach(DISPLAY_FIN, panel, buffer(2, DISPLAY_FIN, 800, 50));
        let _ = server.attach(
            DISPLAY_FIN,
            launcher_surface,
            buffer(10, DISPLAY_FIN, 420, 428),
        );
        let _ = server.set_visible(DISPLAY_FIN, launcher_surface, false);

        let _ = server.commit(DISPLAY_FIN, background);
        let _ = server.commit(DISPLAY_FIN, panel);
        for app in AppKind::ALL {
            let _ = server.commit(app.owner(), app_surfaces[app.index()]);
        }
        let _ = server.commit(DISPLAY_FIN, launcher_surface);
        let _ = server.focus(app_surfaces[active.index()]);

        Self {
            server,
            broker,
            app_surfaces,
            app_handles,
            app_workspaces: [1, 1, 2, 2, 3, 3, 4],
            app_open: [true; APP_COUNT],
            app_floating: [false; APP_COUNT],
            launcher_surface,
            active,
            workspace: 1,
            launcher_open: false,
            overview_open: false,
            fullscreen: false,
            document: Document::parse("hexa://home", HOME).expect("built-in home document"),
            terminal_line: [0; 64],
            terminal_len: 0,
            terminal_message: [0; 128],
            terminal_message_len: 0,
            terminal_history: [[0; 64]; TERMINAL_HISTORY],
            terminal_history_len: [0; TERMINAL_HISTORY],
            terminal_history_next: 0,
            terminal_history_count: 0,
            terminal_history_cursor: None,
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
        if self.authorized(self.active, Operations::INPUT) {
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
            if (START_X..START_X + 38).contains(&x) {
                self.toggle_launcher();
                return true;
            }
            for (index, app) in AppKind::ALL.iter().copied().enumerate() {
                let left = TASK_ICON_X + index as i16 * TASK_ICON_STEP;
                if (left..left + 38).contains(&x) {
                    self.focus_existing(app);
                    return true;
                }
            }
            return false;
        }
        if self.launcher_open {
            if (190..610).contains(&x) && (112..540).contains(&y) {
                for (index, app) in AppKind::ALL.iter().copied().enumerate() {
                    let column = index % 2;
                    let row = index / 2;
                    let left = 214 + column as i16 * 190;
                    let top = 190 + row as i16 * 63;
                    if (left..left + 170).contains(&x) && (top..top + 50).contains(&y) {
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
                self.close_active();
            } else {
                if !self.app_floating[app.index()] {
                    self.toggle_floating();
                }
                self.dragging = Some(app);
            }
            return true;
        }
        if app == AppKind::Games && self.games.handle_click(x, y, rect) {
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
            && self.app_workspaces[app.index()] == self.workspace
            && (!self.fullscreen || app == self.active)
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
        self.app_open[next.index()] = true;
        self.app_workspaces[next.index()] = self.workspace;
        self.active = next;
        self.fullscreen = false;
        self.close_launcher();
        self.overview_open = false;
        self.sync_visibility();
        self.arrange_windows();
        let _ = self.server.focus(self.active_surface());
    }

    fn focus_existing(&mut self, next: AppKind) {
        if !self.app_open[next.index()] {
            self.switch_to(next);
            return;
        }
        self.workspace = self.app_workspaces[next.index()];
        self.active = next;
        self.fullscreen = false;
        self.close_launcher();
        self.overview_open = false;
        self.sync_visibility();
        self.arrange_windows();
        let _ = self.server.focus(self.active_surface());
    }

    fn cycle_app(&mut self) {
        for offset in 1..=APP_COUNT {
            let next = AppKind::ALL[(self.active.index() + offset) % APP_COUNT];
            if self.app_open[next.index()] && self.app_workspaces[next.index()] == self.workspace {
                self.active = next;
                self.arrange_windows();
                let _ = self.server.focus(self.active_surface());
                return;
            }
        }
    }

    fn switch_workspace(&mut self, workspace: u8) {
        if !(1..=4).contains(&workspace) || workspace == self.workspace {
            return;
        }
        self.workspace = workspace;
        self.fullscreen = false;
        self.overview_open = false;
        let next = AppKind::ALL
            .iter()
            .copied()
            .find(|app| self.app_open[app.index()] && self.app_workspaces[app.index()] == workspace)
            .unwrap_or(AppKind::Terminal);
        self.app_open[next.index()] = true;
        self.app_workspaces[next.index()] = workspace;
        self.active = next;
        self.sync_visibility();
        self.arrange_windows();
        let _ = self.server.focus(self.active_surface());
    }

    fn close_active(&mut self) {
        self.app_open[self.active.index()] = false;
        self.fullscreen = false;
        let replacement = AppKind::ALL.iter().copied().find(|app| {
            self.app_open[app.index()] && self.app_workspaces[app.index()] == self.workspace
        });
        let next = replacement.unwrap_or(AppKind::Terminal);
        if replacement.is_none() {
            self.app_open[next.index()] = true;
            self.app_workspaces[next.index()] = self.workspace;
        }
        self.active = next;
        self.sync_visibility();
        self.arrange_windows();
        let _ = self.server.focus(self.active_surface());
    }

    fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        self.sync_visibility();
        self.arrange_windows();
        let _ = self.server.focus(self.active_surface());
    }

    fn toggle_floating(&mut self) {
        self.fullscreen = false;
        let index = self.active.index();
        self.app_floating[index] = !self.app_floating[index];
        if self.app_floating[index] && self.authorized(self.active, Operations::DISPLAY) {
            let _ = self.server.set_geometry(
                self.active.owner(),
                self.active_surface(),
                Rect::new(76, 48, 650, 472),
            );
            let _ = self
                .server
                .commit(self.active.owner(), self.active_surface());
        }
        self.sync_visibility();
        self.arrange_windows();
        let _ = self.server.focus(self.active_surface());
    }

    fn toggle_launcher(&mut self) {
        self.launcher_open = !self.launcher_open;
        let _ = self
            .server
            .set_visible(DISPLAY_FIN, self.launcher_surface, self.launcher_open);
        let _ = self.server.commit(DISPLAY_FIN, self.launcher_surface);
        if self.launcher_open {
            let _ = self.server.focus(self.launcher_surface);
        } else {
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
        if !self.app_floating[self.active.index()] {
            self.toggle_floating();
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

    fn arrange_windows(&mut self) {
        if self.fullscreen {
            self.set_app_geometry(self.active, Rect::new(8, 8, 784, 534));
            return;
        }

        let tiled_count = AppKind::ALL
            .iter()
            .filter(|app| {
                self.app_open[app.index()]
                    && self.app_workspaces[app.index()] == self.workspace
                    && !self.app_floating[app.index()]
            })
            .count();
        let master = if !self.app_floating[self.active.index()] {
            Some(self.active)
        } else {
            AppKind::ALL.iter().copied().find(|app| {
                self.app_open[app.index()]
                    && self.app_workspaces[app.index()] == self.workspace
                    && !self.app_floating[app.index()]
            })
        };

        if let Some(master) = master {
            let rect = if tiled_count == 1 {
                Rect::new(24, 20, 752, 518)
            } else {
                Rect::new(20, 20, 510, 518)
            };
            self.set_app_geometry(master, rect);
        }
        if tiled_count > 1 {
            let stack_count = tiled_count - 1;
            let stack_height = (518 - (stack_count.saturating_sub(1) * 8)) / stack_count;
            let mut stack_index = 0;
            for app in AppKind::ALL {
                if Some(app) == master
                    || !self.app_open[app.index()]
                    || self.app_workspaces[app.index()] != self.workspace
                    || self.app_floating[app.index()]
                {
                    continue;
                }
                let y = 20 + stack_index as i16 * (stack_height as i16 + 8);
                self.set_app_geometry(app, Rect::new(538, y, 242, stack_height as u16));
                stack_index += 1;
            }
        }
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
                    TerminalAction::Switch(app) => self.switch_to(app),
                    TerminalAction::Workspace(workspace) => self.switch_workspace(workspace),
                    TerminalAction::Float => self.toggle_floating(),
                    TerminalAction::Fullscreen => self.toggle_fullscreen(),
                    TerminalAction::Overview => self.overview_open = true,
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
    Switch(AppKind),
    Workspace(u8),
    Float,
    Fullscreen,
    Overview,
    Exit,
}

pub fn run(input: &mut Input, start_browser: bool, session: crate::session::Session) {
    run_session(
        input,
        if start_browser {
            AppKind::Browser
        } else {
            AppKind::Terminal
        },
        session,
    );
}

pub fn run_games(input: &mut Input, session: crate::session::Session) {
    run_session(input, AppKind::Games, session);
}

fn run_session(input: &mut Input, start_app: AppKind, session: crate::session::Session) {
    let mouse_ready = input.enable_mouse();
    if !framebuffer::enter() {
        crate::println!("HexaDisplay unavailable: no Bochs/QEMU VBE framebuffer.");
        slog!("HEXA_DISPLAY_UNAVAILABLE\r\n");
        return;
    }

    let mut desktop = DesktopState::new(start_app == AppKind::Browser, session);
    if start_app == AppKind::Games {
        desktop.switch_workspace(4);
    }
    render(&mut desktop);
    slog!("HEXA_DISPLAY_READY surfaces=10 commit=10\r\n");
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
            if desktop.launcher_open {
                desktop.close_launcher();
                let _ = desktop.server.focus(desktop.active_surface());
                render(&mut desktop);
                continue;
            }
            if desktop.overview_open {
                desktop.overview_open = false;
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
        if desktop.active == AppKind::Games && desktop.games.handle_key(key) {
            render(&mut desktop);
            continue;
        }
        match key {
            KEY_SUPER_TERMINAL => desktop.switch_to(AppKind::Terminal),
            KEY_SUPER_BROWSER => desktop.switch_to(AppKind::Browser),
            KEY_SUPER_CLOSE => desktop.close_active(),
            KEY_SUPER_CYCLE | KEY_SUPER_LEFT | KEY_SUPER_RIGHT => desktop.cycle_app(),
            KEY_SUPER_UP => {
                let workspace = if desktop.workspace == 1 {
                    4
                } else {
                    desktop.workspace - 1
                };
                desktop.switch_workspace(workspace);
            }
            KEY_SUPER_DOWN => {
                let workspace = if desktop.workspace == 4 {
                    1
                } else {
                    desktop.workspace + 1
                };
                desktop.switch_workspace(workspace);
            }
            KEY_SUPER_FULLSCREEN => desktop.toggle_fullscreen(),
            KEY_SUPER_FLOAT => desktop.toggle_floating(),
            KEY_SUPER_OVERVIEW => desktop.overview_open = !desktop.overview_open,
            KEY_SUPER_WORKSPACE_1 => desktop.switch_workspace(1),
            KEY_SUPER_WORKSPACE_2 => desktop.switch_workspace(2),
            KEY_SUPER_WORKSPACE_3 => desktop.switch_workspace(3),
            KEY_SUPER_WORKSPACE_4 => desktop.switch_workspace(4),
            b'\t' => desktop.cycle_app(),
            KEY_UP if desktop.active == AppKind::Terminal => desktop.recall_terminal_history(true),
            KEY_DOWN if desktop.active == AppKind::Terminal => {
                desktop.recall_terminal_history(false)
            }
            KEY_LEFT => desktop.move_active(-12, 0),
            KEY_RIGHT => desktop.move_active(12, 0),
            KEY_UP => desktop.move_active(0, -12),
            KEY_DOWN => desktop.move_active(0, 12),
            _ if desktop.active == AppKind::Terminal => desktop.handle_terminal_key(key),
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
        _ => None,
    }
}

fn terminal_action(command: &[u8]) -> TerminalAction {
    let trimmed = trim_ascii(command);
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(b"help") {
        TerminalAction::Message(
            "COMMANDS: STATUS FORMS PACKAGES BROWSER GAMES SETTINGS SYSTEM WS1 WS2 WS3 WS4 FLOAT FULL OVERVIEW CLEAR EXIT",
        )
    } else if trimmed.eq_ignore_ascii_case(b"status") {
        TerminalAction::Message("EXPOS ONLINE. HEXADISPLAY V1. NETWORK RESTRICTED.")
    } else if trimmed.eq_ignore_ascii_case(b"forms") {
        TerminalAction::Switch(AppKind::Forms)
    } else if trimmed.eq_ignore_ascii_case(b"packages") || trimmed.eq_ignore_ascii_case(b"ayo") {
        TerminalAction::Switch(AppKind::Packages)
    } else if trimmed.eq_ignore_ascii_case(b"browser") {
        TerminalAction::Switch(AppKind::Browser)
    } else if trimmed.eq_ignore_ascii_case(b"settings") {
        TerminalAction::Switch(AppKind::Settings)
    } else if trimmed.eq_ignore_ascii_case(b"system") {
        TerminalAction::Switch(AppKind::System)
    } else if trimmed.eq_ignore_ascii_case(b"games") || trimmed.eq_ignore_ascii_case(b"arcade") {
        TerminalAction::Switch(AppKind::Games)
    } else if trimmed.eq_ignore_ascii_case(b"ws1") {
        TerminalAction::Workspace(1)
    } else if trimmed.eq_ignore_ascii_case(b"ws2") {
        TerminalAction::Workspace(2)
    } else if trimmed.eq_ignore_ascii_case(b"ws3") {
        TerminalAction::Workspace(3)
    } else if trimmed.eq_ignore_ascii_case(b"ws4") {
        TerminalAction::Workspace(4)
    } else if trimmed.eq_ignore_ascii_case(b"float") {
        TerminalAction::Float
    } else if trimmed.eq_ignore_ascii_case(b"full") || trimmed.eq_ignore_ascii_case(b"fullscreen") {
        TerminalAction::Fullscreen
    } else if trimmed.eq_ignore_ascii_case(b"overview") {
        TerminalAction::Overview
    } else if trimmed.eq_ignore_ascii_case(b"clear") {
        TerminalAction::Clear
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

fn render(desktop: &mut DesktopState) {
    desktop.cursor.invalidate();
    framebuffer::clear(0x0010_376F);
    for row in 0..22 {
        framebuffer::rect(
            0,
            row * 25,
            800,
            25,
            0x000A_316C + (row as u32 * 0x0000_0203),
        );
    }
    framebuffer::rect(505, 88, 112, 150, 0x0027_75D3);
    framebuffer::rect(625, 70, 130, 168, 0x0034_8BE8);
    framebuffer::rect(505, 246, 112, 150, 0x001E_66BF);
    framebuffer::rect(625, 246, 130, 168, 0x0028_7DD8);
    framebuffer::rect(497, 80, 4, 340, 0x0067_B8FF);
    framebuffer::rect(20, 22, 74, 58, 0x0018_4F98);
    framebuffer::text(31, 42, "FORMS", color::WHITE, 1);
    framebuffer::text(25, 88, "SYSTEM FORMS", 0x00DB_EBFF, 1);
    draw_panel(desktop);

    if desktop.overview_open {
        draw_overview(desktop);
    } else {
        for app in AppKind::ALL {
            if app != desktop.active && desktop.app_is_visible(app) {
                draw_app(desktop, app, false);
            }
        }
        if desktop.app_is_visible(desktop.active) {
            draw_app(desktop, desktop.active, true);
        }
    }
    draw_dock(desktop);
    if desktop.launcher_open {
        draw_launcher(desktop);
    }
    desktop.cursor.draw();
}

fn render_active_window(desktop: &mut DesktopState) {
    desktop.cursor.restore();
    if desktop.app_is_visible(desktop.active) {
        draw_app(desktop, desktop.active, true);
    }
    desktop.cursor.draw();
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
            AppKind::Browser => draw_browser(rect, &desktop.document),
            AppKind::Terminal => draw_terminal(rect, desktop),
            AppKind::Forms => draw_forms(rect),
            AppKind::Packages => draw_packages(rect),
            AppKind::Settings => draw_settings(rect),
            AppKind::System => draw_system(rect, desktop),
            AppKind::Games => desktop.games.render(rect),
        }
    } else {
        draw_compact_app(rect, app, desktop, focused);
    }
}

fn draw_panel(desktop: &DesktopState) {
    framebuffer::rect(608, 14, 174, 34, 0x00E7_EFF9);
    framebuffer::outline(608, 14, 174, 34, 0x00A7_C9EE);
    framebuffer::text(621, 27, "DESKTOP", 0x0029_4463, 1);
    for workspace in 1..=4 {
        let x = 681 + (workspace - 1) * 23;
        framebuffer::rect(
            x,
            21,
            18,
            18,
            if desktop.workspace == workspace as u8 {
                0x0000_78D4
            } else {
                0x00C5_D7EA
            },
        );
        draw_number(x + 7, 27, workspace as u64, color::WHITE);
    }
}

fn draw_window(rect: Rect, title: &str, focused: bool, handle_id: u32) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(x - 6, y + 7, width + 12, height + 7, 0x0010_2A50);
    framebuffer::rect(x, y, width, height, color::WINDOW);
    framebuffer::outline(
        x,
        y,
        width,
        height,
        if focused { 0x0000_78D4 } else { 0x0097_A8BA },
    );
    if focused {
        framebuffer::outline(x - 1, y - 1, width + 2, height + 2, 0x0066_ADE8);
    }
    framebuffer::rect(x, y, width, 36, 0x00F3_F6FA);
    framebuffer::rect(x + 12, y + 10, 16, 16, 0x0000_78D4);
    framebuffer::text(x + 17, y + 15, "E", color::WHITE, 1);
    framebuffer::text(x + 38, y + 13, title, 0x0021_2A35, 1);
    if width >= 430 {
        framebuffer::text(x + width - 188, y + 13, "HANDLE", 0x0071_7E8C, 1);
        draw_number(x + width - 137, y + 13, handle_id as u64, 0x0000_78D4);
    }
    framebuffer::rect(x + width - 126, y, 42, 36, 0x00F3_F6FA);
    framebuffer::text(x + width - 110, y + 13, "_", 0x0048_535F, 1);
    framebuffer::rect(x + width - 84, y, 42, 36, 0x00F3_F6FA);
    framebuffer::outline(x + width - 69, y + 11, 12, 10, 0x0048_535F);
    framebuffer::rect(
        x + width - 42,
        y,
        42,
        36,
        if focused { 0x00E8_F0F9 } else { 0x00F3_F6FA },
    );
    framebuffer::text(x + width - 25, y + 13, "X", 0x0050_5A66, 1);
}

fn draw_compact_app(rect: Rect, app: AppKind, desktop: &DesktopState, focused: bool) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    let dark = app == AppKind::Terminal;
    framebuffer::rect(
        x + 12,
        y + 48,
        width - 24,
        height - 63,
        if dark { 0x0008_0B12 } else { 0x00EA_EDF4 },
    );
    framebuffer::rect(x + 25, y + 66, 6, 54, app.accent());
    framebuffer::text(
        x + 44,
        y + 68,
        app.title(),
        app.accent(),
        if width >= 380 { 2 } else { 1 },
    );
    if width >= 330 {
        framebuffer::text(
            x + 44,
            y + 98,
            app.summary(),
            if dark { color::MUTED } else { color::INK },
            1,
        );
    }
    framebuffer::text(
        x + 28,
        y + 145,
        if focused { "FOCUSED" } else { "VISIBLE" },
        if focused { color::GREEN } else { color::MUTED },
        1,
    );
    framebuffer::text(x + 28, y + 168, "WORKSPACE", color::MUTED, 1);
    draw_number(
        x + 112,
        y + 168,
        desktop.app_workspaces[app.index()] as u64,
        color::WHITE,
    );
    framebuffer::text(x + 28, y + 191, "HANDLE", color::MUTED, 1);
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
        framebuffer::text(x + 28, y + height - 62, "SUPER+V FLOAT", color::MUTED, 1);
    }
}

fn draw_browser(rect: Rect, document: &Document) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let bottom = y + rect.height as i32;
    framebuffer::rect(x + 20, y + 52, width - 40, 32, 0x00E7_EAF2);
    framebuffer::outline(x + 20, y + 52, width - 40, 32, color::BORDER);
    framebuffer::text(x + 34, y + 63, document.url(), color::INK, 1);
    framebuffer::rect(x + 20, y + 94, width - 40, 1, color::BORDER);

    let mut content_y = y + 109;
    for node in document.nodes() {
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
        "1 ABOUT  2 PACKAGES  3 SYSTEM  H HOME  N NETWORK  Q SHELL",
    );
}

fn draw_terminal(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(x + 14, y + 50, width - 28, height - 78, 0x0008_0B12);
    framebuffer::text(x + 34, y + 72, "EXPOS GRAPHICAL TERMINAL", color::GREEN, 2);
    framebuffer::text(
        x + 34,
        y + 103,
        "COMMAND INTERFACE FORM // SESSION AUTHORITY",
        color::MUTED,
        1,
    );
    framebuffer::rect(x + 34, y + 126, width - 68, 1, 0x0025_2B3D);
    framebuffer::text(x + 34, y + 150, "TYPE HELP FOR COMMANDS.", color::WHITE, 1);
    if desktop.terminal_message_len > 0 {
        let message =
            core::str::from_utf8(&desktop.terminal_message[..desktop.terminal_message_len])
                .unwrap_or("INVALID TERMINAL OUTPUT");
        wrapped_text(x + 34, y + 181, width - 76, message, color::CYAN, 2);
    }
    if height >= 430 {
        framebuffer::text(
            x + 34,
            y + 242,
            "RECENT // UP DOWN TO RECALL",
            color::MUTED,
            1,
        );
        for reverse_index in (0..3).rev() {
            if let Some(command) = desktop.terminal_history_entry(reverse_index) {
                let row = y + 266 + (2 - reverse_index) as i32 * 20;
                framebuffer::text(x + 40, row, ">", color::PURPLE, 1);
                framebuffer::text(x + 54, row, command, color::MUTED, 1);
            }
        }
    }
    framebuffer::text(
        x + 34,
        y + height - 83,
        desktop.session.name(),
        color::GREEN,
        2,
    );
    framebuffer::text(
        x + 34 + desktop.session.name().len() as i32 * 12,
        y + height - 83,
        "@stable>",
        color::GREEN,
        2,
    );
    if let Ok(line) = core::str::from_utf8(&desktop.terminal_line[..desktop.terminal_len]) {
        let input_x = x + 34 + (desktop.session.name().len() as i32 + 9) * 12;
        framebuffer::text(input_x, y + height - 83, line, color::WHITE, 2);
        framebuffer::rect(
            input_x + line.len() as i32 * 12,
            y + height - 84,
            10,
            17,
            color::PURPLE,
        );
    }
    app_footer(
        rect,
        "ENTER RUN  UP DOWN HISTORY  TAB NEXT APP  \x60 LAUNCHER  ESC SHELL",
    );
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
        framebuffer::rect(x + 28, row_y, rect.width as i32 - 56, 34, 0x00EA_EDF4);
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
        "CORETOOLS",
        "INSTALLED",
        "DIAGNOSTICS AND REPAIR",
    );
    package_card(
        x + 28,
        y + 174,
        "HEXADISPLAY",
        "INSTALLED",
        "FORM NATIVE DISPLAY SERVICE",
    );
    package_card(
        x + 28,
        y + 232,
        "GAMEHUB + SNAKE + PONG",
        "AVAILABLE",
        "NATIVE PLAYABLE GAME FORMS",
    );
    package_card(
        x + 28,
        y + 290,
        "PRISMDE + SESSION + MOUSEKIT",
        "AVAILABLE",
        "DESKTOP IDENTITY AND POINTER",
    );
    framebuffer::text(
        x + 34,
        y + 362,
        "DOWNLOAD AND SIGNATURE OPERATIONS RUN IN THE HOST AYO TUI.",
        color::INK,
        1,
    );
    framebuffer::text(
        x + 34,
        y + 380,
        "NATIVE PERSISTENT REPOSITORY TRANSPORT IS THE NEXT BOUND DRIVER.",
        color::MUTED,
        1,
    );
    app_footer(
        rect,
        "AYO V2 PACKAGE FORM  TAB NEXT  \x60 LAUNCHER  Q SHELL",
    );
}

fn package_card(x: i32, y: i32, name: &str, status: &str, detail: &str) {
    framebuffer::rect(x, y, 648, 46, 0x00EA_EDF4);
    framebuffer::outline(x, y, 648, 46, color::BORDER);
    framebuffer::text(x + 14, y + 11, name, color::INK, 1);
    framebuffer::text(x + 178, y + 11, detail, color::MUTED, 1);
    framebuffer::text(
        x + 540,
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
    framebuffer::text(x + 28, y + 58, "DESKTOP SETTINGS", color::PURPLE, 2);
    framebuffer::text(x + 28, y + 91, "SESSION", color::MUTED, 1);
    setting_row(x + 28, y + 112, "RENDERER", "HEXADISPLAY V1", true);
    setting_row(x + 28, y + 157, "RESOLUTION", "800 X 600 XRGB", true);
    setting_row(x + 28, y + 202, "THEME", "STABLE DIMENSION", true);
    setting_row(x + 28, y + 247, "INPUT", "PS2 PLUS SERIAL", true);
    setting_row(x + 28, y + 292, "NETWORK", "RESTRICTED", false);
    setting_row(x + 28, y + 337, "POLICY", "DIESE ENFORCED", true);
    app_footer(
        rect,
        "SETTINGS ARE SESSION LOCAL  TAB NEXT  ARROWS MOVE  Q SHELL",
    );
}

fn setting_row(x: i32, y: i32, label: &str, value: &str, enabled: bool) {
    framebuffer::rect(x, y, 648, 35, 0x00EA_EDF4);
    framebuffer::text(x + 15, y + 13, label, color::INK, 1);
    framebuffer::text(x + 235, y + 13, value, color::MUTED, 1);
    framebuffer::rect(
        x + 600,
        y + 11,
        27,
        13,
        if enabled { color::GREEN } else { color::RED },
    );
    framebuffer::rect(
        x + if enabled { 616 } else { 602 },
        y + 13,
        9,
        9,
        color::WHITE,
    );
}

fn draw_system(rect: Rect, desktop: &DesktopState) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    framebuffer::text(x + 28, y + 58, "SYSTEM SCOPE", color::PURPLE, 2);
    framebuffer::text(x + 28, y + 88, "LIVE SESSION TELEMETRY", color::MUTED, 1);
    metric(
        x + 28,
        y + 120,
        "DISPLAY PROTOCOL",
        "VERSION 1",
        color::GREEN,
    );
    metric(x + 352, y + 120, "FRAMEBUFFER", "800 X 600", color::CYAN);
    metric(x + 28, y + 196, "SURFACES", "10 FORM OWNED", color::PURPLE);
    metric(x + 352, y + 196, "GO ABI", "VERSION 1", color::GREEN);
    metric(x + 28, y + 272, "INPUT", "PS2 MOUSE + KEYS", color::CYAN);
    metric(x + 352, y + 272, "NETWORK", "RESTRICTED", color::RED);
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

fn metric(x: i32, y: i32, label: &str, value: &str, accent: u32) {
    framebuffer::rect(x, y, 306, 60, 0x00EA_EDF4);
    framebuffer::rect(x, y, 5, 60, accent);
    framebuffer::text(x + 18, y + 13, label, color::MUTED, 1);
    framebuffer::text(x + 18, y + 34, value, accent, 1);
}

fn app_footer(rect: Rect, text: &str) {
    let x = rect.x as i32;
    let y = rect.y as i32 + rect.height as i32 - 28;
    framebuffer::rect(x + 1, y, rect.width as i32 - 2, 27, 0x00E7_EAF2);
    framebuffer::text(x + 16, y + 10, text, color::MUTED, 1);
}

fn draw_dock(desktop: &DesktopState) {
    framebuffer::rect(0, TASKBAR_Y as i32, 800, 50, 0x00E9_F1FA);
    framebuffer::rect(0, TASKBAR_Y as i32, 800, 1, 0x00B6_D1EC);
    let start_color = if desktop.launcher_open {
        0x00D2_E7FA
    } else {
        0x00E9_F1FA
    };
    framebuffer::rect(START_X as i32, 556, 38, 38, start_color);
    for (x, y) in [(213, 564), (224, 564), (213, 575), (224, 575)] {
        framebuffer::rect(x, y, 9, 9, 0x0000_78D4);
    }
    for app in AppKind::ALL {
        let x = TASK_ICON_X as i32 + app.index() as i32 * TASK_ICON_STEP as i32;
        if app == desktop.active {
            framebuffer::rect(x, 556, 38, 38, 0x00D2_E7FA);
        }
        framebuffer::rect(x + 10, 564, 18, 18, app.accent());
        framebuffer::text(x + 16, 570, app.shortcut(), color::WHITE, 1);
        if desktop.app_open[app.index()] {
            framebuffer::rect(x + 10, 591, 18, 2, 0x0000_78D4);
        }
    }
    framebuffer::text(606, 568, "NET --", 0x003E_5268, 1);
    framebuffer::text(658, 568, "VOL", 0x003E_5268, 1);
    framebuffer::text(704, 564, desktop.session.name(), 0x0029_4463, 1);
    framebuffer::text(704, 580, desktop.session.authority_name(), 0x005F_7489, 1);
}

fn draw_overview(desktop: &DesktopState) {
    framebuffer::rect(28, 58, 744, 488, 0x0012_1728);
    framebuffer::outline(28, 58, 744, 488, color::PURPLE);
    framebuffer::text(50, 80, "FORM OVERVIEW", color::WHITE, 2);
    framebuffer::text(
        50,
        108,
        "SUPER+O CLOSE  SUPER+1..4 WORKSPACE  SUPER+Q CLOSE WINDOW",
        color::MUTED,
        1,
    );
    for (index, app) in AppKind::ALL.iter().copied().enumerate() {
        let column = index % 4;
        let row = index / 4;
        let x = 50 + column as i32 * 178;
        let y = 140 + row as i32 * 176;
        framebuffer::rect(
            x,
            y,
            162,
            150,
            if app == desktop.active {
                0x0035_285E
            } else {
                0x0022_293B
            },
        );
        framebuffer::outline(
            x,
            y,
            162,
            150,
            if app == desktop.active {
                color::PURPLE
            } else {
                color::BORDER
            },
        );
        framebuffer::rect(x + 14, y + 15, 8, 38, app.accent());
        framebuffer::text(x + 34, y + 17, app.title(), color::WHITE, 1);
        framebuffer::text(x + 14, y + 82, "WORKSPACE", color::MUTED, 1);
        draw_number(
            x + 98,
            y + 82,
            desktop.app_workspaces[app.index()] as u64,
            color::CYAN,
        );
        framebuffer::text(
            x + 14,
            y + 108,
            if desktop.app_open[app.index()] {
                "OPEN + CAP GRANTED"
            } else {
                "CLOSED"
            },
            if desktop.app_open[app.index()] {
                color::GREEN
            } else {
                color::RED
            },
            1,
        );
    }
}

fn draw_launcher(desktop: &DesktopState) {
    framebuffer::rect(198, 120, 420, 428, 0x0010_2A50);
    framebuffer::rect(190, 112, 420, 428, 0x00F3_F7FC);
    framebuffer::outline(190, 112, 420, 428, 0x009D_C0E3);
    framebuffer::text(214, 132, "EXPOS START", 0x0022_3347, 2);
    framebuffer::rect(214, 158, 372, 25, color::WHITE);
    framebuffer::outline(214, 158, 372, 25, 0x00B8_CBDD);
    framebuffer::text(226, 168, "SEARCH FORMS AND CAPABILITIES", 0x0070_8091, 1);
    for (index, app) in AppKind::ALL.iter().copied().enumerate() {
        let column = index % 2;
        let row = index / 2;
        let x = 214 + column as i32 * 190;
        let y = 190 + row as i32 * 63;
        framebuffer::rect(
            x,
            y,
            170,
            50,
            if app == desktop.active {
                0x00D9_ECFB
            } else {
                0x00E9_F1F9
            },
        );
        framebuffer::rect(x + 10, y + 9, 30, 30, app.accent());
        framebuffer::text(x + 22, y + 20, app.shortcut(), color::WHITE, 1);
        framebuffer::text(x + 50, y + 14, app.title(), 0x0029_394B, 1);
        framebuffer::text(x + 50, y + 31, "FORM APP", 0x0070_8091, 1);
    }
    framebuffer::rect(190, 470, 420, 70, 0x00E3_ECF6);
    framebuffer::rect(214, 489, 30, 30, 0x0000_78D4);
    framebuffer::text(225, 500, "O", color::WHITE, 1);
    framebuffer::text(256, 491, desktop.session.name(), 0x0029_394B, 1);
    framebuffer::text(256, 508, desktop.session.authority_name(), 0x006D_7E90, 1);
    framebuffer::text(493, 500, "ESC CLOSE", 0x006D_7E90, 1);
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
