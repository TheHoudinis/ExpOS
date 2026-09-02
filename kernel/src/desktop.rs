use crate::{
    framebuffer,
    input::{Input, KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_UP},
    slog,
};
use framebuffer::color;
use hexa_core::{
    BufferFormat, BufferHandle, DisplayServer, Document, Fin, NodeKind, Rect, SurfaceRole,
};

pub const DISPLAY_FIN: Fin = Fin::from_u128(0x4449_5350_4C41_5900_0000_0000_0000_0001);
pub const BROWSER_FIN: Fin = Fin::from_u128(0x4252_4F57_5345_5200_0000_0000_0000_0001);
pub const TERMINAL_FIN: Fin = Fin::from_u128(0x5445_524D_494E_414C_0000_0000_0000_0001);
pub const FORMS_FIN: Fin = Fin::from_u128(0x464F_524D_5300_0000_0000_0000_0000_0001);
pub const PACKAGES_FIN: Fin = Fin::from_u128(0x5041_434B_4147_4553_0000_0000_0000_0001);
pub const SETTINGS_FIN: Fin = Fin::from_u128(0x5345_5454_494E_4753_0000_0000_0000_0001);
pub const SYSTEM_FIN: Fin = Fin::from_u128(0x5359_5354_454D_0000_0000_0000_0000_0001);

const APP_COUNT: usize = 6;
const APP_WIDTH: u16 = 704;
const APP_HEIGHT: u16 = 466;

const HOME: &str = "<title>Hexa Home</title><h1>Welcome to ExpOS</h1><p>A Form native desktop where identity capability state and relationships are first class.</p><h2>Explore locally</h2><a href='hexa://about'>1 About this browser</a><a href='hexa://packages'>2 Package Forms</a><a href='hexa://system'>3 System status</a><p>Use 1 2 3 or H inside Browser. External navigation is safely restricted.</p>";
const ABOUT: &str = "<title>About</title><h1>Hexa Browser</h1><p>This native Interface Form parses bounded local HTML and renders it through HexaDisplay.</p><p>Surface state is owner scoped and becomes visible only after an atomic commit.</p><p>HTTPS CSS JavaScript and media are intentionally not claimed before networking isolation and a complete web engine exist.</p><a href='hexa://home'>H Home</a>";
const BROWSER_PACKAGES: &str = "<title>Packages</title><h1>Package Forms</h1><p>Ayo activates software as Forms rather than copying archives into Unix paths.</p><li>CoreTools diagnostics and repair</li><li>Network socket capability</li><li>Terminal command Interface</li><li>Browser document Interface</li><p>Use the Packages app for the native catalog and the host ayo TUI for repository downloads.</p><a href='hexa://home'>H Home</a>";
const BROWSER_SYSTEM: &str = "<title>System</title><h1>System Scope</h1><p>HexaDisplay protocol version 1 is active.</p><li>800 by 600 XRGB framebuffer</li><li>Atomic attach damage and commit</li><li>Focus z order and keyboard routing</li><li>PS2 and serial input</li><p>Network Driver Form is not bound. External navigation remains restricted.</p><a href='hexa://home'>H Home</a>";
const NETWORK_BLOCKED: &str = "<title>Network restricted</title><h1>Navigation blocked</h1><p>DIESE denied external navigation because no v8 Network Driver Form is bound and PIMP network policy is restricted.</p><p>This is a safe fallback not a fake internet connection.</p><a href='hexa://home'>H Home</a>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppKind {
    Browser,
    Terminal,
    Forms,
    Packages,
    Settings,
    System,
}

impl AppKind {
    const ALL: [Self; APP_COUNT] = [
        Self::Browser,
        Self::Terminal,
        Self::Forms,
        Self::Packages,
        Self::Settings,
        Self::System,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Browser => 0,
            Self::Terminal => 1,
            Self::Forms => 2,
            Self::Packages => 3,
            Self::Settings => 4,
            Self::System => 5,
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
        }
    }
}

struct DesktopState {
    server: DisplayServer,
    app_surfaces: [u32; APP_COUNT],
    launcher_surface: u32,
    active: AppKind,
    launcher_open: bool,
    document: Document,
    terminal_line: [u8; 64],
    terminal_len: usize,
    terminal_message: [u8; 128],
    terminal_message_len: usize,
    should_exit: bool,
}

impl DesktopState {
    fn new(start_browser: bool) -> Self {
        let active = if start_browser {
            AppKind::Browser
        } else {
            AppKind::Terminal
        };
        let mut server = DisplayServer::new();
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
                "Desktop Panel",
                SurfaceRole::Panel,
                Rect::new(0, 0, 800, 42),
            )
            .expect("desktop panel surface");

        let mut app_surfaces = [0; APP_COUNT];
        for (index, app) in AppKind::ALL.iter().copied().enumerate() {
            let x = 48 + ((index % 3) as i16 * 10);
            let y = 58 + ((index % 2) as i16 * 10);
            let surface = server
                .create_surface(
                    app.owner(),
                    app.title(),
                    SurfaceRole::Window,
                    Rect::new(x, y, APP_WIDTH, APP_HEIGHT),
                )
                .expect("built-in application surface");
            let _ = server.attach(
                app.owner(),
                surface,
                buffer((index + 3) as u32, app.owner(), APP_WIDTH, APP_HEIGHT),
            );
            let _ = server.set_visible(app.owner(), surface, app == active);
            app_surfaces[index] = surface;
        }

        let launcher_surface = server
            .create_surface(
                DISPLAY_FIN,
                "Launcher",
                SurfaceRole::Popup,
                Rect::new(18, 48, 280, 360),
            )
            .expect("desktop launcher surface");
        let _ = server.attach(DISPLAY_FIN, background, buffer(1, DISPLAY_FIN, 800, 600));
        let _ = server.attach(DISPLAY_FIN, panel, buffer(2, DISPLAY_FIN, 800, 42));
        let _ = server.attach(
            DISPLAY_FIN,
            launcher_surface,
            buffer(9, DISPLAY_FIN, 280, 360),
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
            app_surfaces,
            launcher_surface,
            active,
            launcher_open: false,
            document: Document::parse("hexa://home", HOME).expect("built-in home document"),
            terminal_line: [0; 64],
            terminal_len: 0,
            terminal_message: [0; 128],
            terminal_message_len: 0,
            should_exit: false,
        }
    }

    fn active_surface(&self) -> u32 {
        self.app_surfaces[self.active.index()]
    }

    fn switch_to(&mut self, next: AppKind) {
        if self.active != next {
            let previous = self.active;
            let _ = self.server.set_visible(
                previous.owner(),
                self.app_surfaces[previous.index()],
                false,
            );
            let _ = self
                .server
                .commit(previous.owner(), self.app_surfaces[previous.index()]);
            self.active = next;
            let _ = self
                .server
                .set_visible(next.owner(), self.app_surfaces[next.index()], true);
            let _ = self
                .server
                .commit(next.owner(), self.app_surfaces[next.index()]);
        }
        self.close_launcher();
        let _ = self.server.focus(self.active_surface());
    }

    fn cycle_app(&mut self) {
        self.switch_to(AppKind::ALL[(self.active.index() + 1) % APP_COUNT]);
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
        let id = self.active_surface();
        let owner = self.active.owner();
        let Some(rect) = self.server.surface(id).map(|surface| surface.current.rect) else {
            return;
        };
        let max_x = (framebuffer::WIDTH as i32 - rect.width as i32 - 8).max(8);
        let max_y = (568 - rect.height as i32).max(48);
        let x = (rect.x as i32 + dx as i32).clamp(8, max_x) as i16;
        let y = (rect.y as i32 + dy as i32).clamp(48, max_y) as i16;
        let _ = self.server.set_position(owner, id, x, y);
        let _ = self.server.commit(owner, id);
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
                let action = terminal_action(&self.terminal_line[..self.terminal_len]);
                self.terminal_len = 0;
                match action {
                    TerminalAction::Message(message) => self.set_terminal_message(message),
                    TerminalAction::Clear => self.terminal_message_len = 0,
                    TerminalAction::Switch(app) => self.switch_to(app),
                    TerminalAction::Exit => self.should_exit = true,
                }
            }
            0x08 => {
                self.terminal_len = self.terminal_len.saturating_sub(1);
            }
            byte if (byte.is_ascii_graphic() || byte == b' ')
                && self.terminal_len < self.terminal_line.len() =>
            {
                self.terminal_line[self.terminal_len] = byte;
                self.terminal_len += 1;
            }
            _ => {}
        }
    }
}

enum TerminalAction {
    Message(&'static str),
    Clear,
    Switch(AppKind),
    Exit,
}

pub fn run(input: &mut Input, start_browser: bool) {
    if !framebuffer::enter() {
        crate::println!("HexaDisplay unavailable: no Bochs/QEMU VBE framebuffer.");
        slog!("HEXA_DISPLAY_UNAVAILABLE\r\n");
        return;
    }

    let mut desktop = DesktopState::new(start_browser);
    render(&desktop);
    slog!("HEXA_DISPLAY_READY surfaces=9 commit=9\r\n");

    while !desktop.should_exit {
        let Some(key) = input.poll() else {
            core::hint::spin_loop();
            continue;
        };
        let _ = desktop.server.route_key(key);

        if key == 0x1B {
            if desktop.launcher_open {
                desktop.close_launcher();
                let _ = desktop.server.focus(desktop.active_surface());
                render(&desktop);
                continue;
            }
            break;
        }
        if key == b'\x60' || key == b'~' {
            desktop.toggle_launcher();
            render(&desktop);
            continue;
        }
        if desktop.launcher_open {
            if let Some(app) = launcher_shortcut(key) {
                desktop.switch_to(app);
            }
            render(&desktop);
            continue;
        }
        match key {
            b'\t' => desktop.cycle_app(),
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
        render(&desktop);
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
        _ => None,
    }
}

fn terminal_action(command: &[u8]) -> TerminalAction {
    let trimmed = trim_ascii(command);
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(b"help") {
        TerminalAction::Message(
            "COMMANDS: STATUS FORMS PACKAGES BROWSER SETTINGS SYSTEM CLEAR EXIT",
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

fn render(desktop: &DesktopState) {
    framebuffer::clear(color::BACKGROUND);
    for row in 0..15 {
        framebuffer::rect(
            0,
            row * 40,
            800,
            40,
            0x000C_1020 + (row as u32 * 0x0001_0102),
        );
    }
    draw_panel(desktop);

    let rect = desktop
        .server
        .surface(desktop.active_surface())
        .map(|surface| surface.current.rect)
        .unwrap_or(Rect::new(48, 58, APP_WIDTH, APP_HEIGHT));
    draw_window(rect, desktop.active.title());
    match desktop.active {
        AppKind::Browser => draw_browser(rect, &desktop.document),
        AppKind::Terminal => draw_terminal(rect, desktop),
        AppKind::Forms => draw_forms(rect),
        AppKind::Packages => draw_packages(rect),
        AppKind::Settings => draw_settings(rect),
        AppKind::System => draw_system(rect, desktop),
    }
    draw_dock(desktop);
    if desktop.launcher_open {
        draw_launcher(desktop);
    }
}

fn draw_panel(desktop: &DesktopState) {
    framebuffer::rect(0, 0, 800, 42, color::PANEL);
    framebuffer::rect(0, 41, 800, 1, color::PURPLE);
    framebuffer::rect(16, 10, 22, 22, color::PURPLE);
    framebuffer::text(22, 17, "H", color::WHITE, 1);
    framebuffer::text(50, 12, "EXPOS", color::WHITE, 2);
    framebuffer::text(120, 14, desktop.active.title(), color::CYAN, 1);
    framebuffer::text(602, 14, "FORM NATIVE", color::MUTED, 1);
    framebuffer::rect(730, 14, 8, 8, color::GREEN);
    framebuffer::text(746, 14, "ONLINE", color::WHITE, 1);
}

fn draw_window(rect: Rect, title: &str) {
    let x = rect.x as i32;
    let y = rect.y as i32;
    let width = rect.width as i32;
    let height = rect.height as i32;
    framebuffer::rect(x - 7, y + 7, width + 14, height + 7, 0x0005_0710);
    framebuffer::rect(x, y, width, height, color::WINDOW);
    framebuffer::outline(x, y, width, height, color::PURPLE);
    framebuffer::rect(x, y, width, 38, color::PANEL);
    framebuffer::rect(x + 15, y + 14, 10, 10, color::RED);
    framebuffer::rect(x + 33, y + 14, 10, 10, 0x00F4_B942);
    framebuffer::rect(x + 51, y + 14, 10, 10, color::GREEN);
    framebuffer::text(x + 78, y + 13, title, color::WHITE, 1);
    framebuffer::text(x + width - 147, y + 13, "FORM WINDOW", color::MUTED, 1);
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
        "COMMAND INTERFACE FORM // OPERATOR AUTHORITY",
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
    framebuffer::text(x + 34, y + height - 83, "operator@stable>", color::GREEN, 2);
    if let Ok(line) = core::str::from_utf8(&desktop.terminal_line[..desktop.terminal_len]) {
        framebuffer::text(x + 226, y + height - 83, line, color::WHITE, 2);
        framebuffer::rect(
            x + 226 + line.len() as i32 * 12,
            y + height - 84,
            10,
            17,
            color::PURPLE,
        );
    }
    app_footer(
        rect,
        "ENTER RUN  BACKSPACE EDIT  TAB NEXT APP  \x60 LAUNCHER  ESC SHELL",
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
        "GOSDK",
        "AVAILABLE",
        "GO ABI V1 DEVELOPMENT KIT",
    );
    package_card(
        x + 28,
        y + 290,
        "DEMOBROWSER",
        "AVAILABLE",
        "LOCAL DOCUMENT INTERFACE",
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
    metric(x + 28, y + 196, "SURFACES", "9 FORM OWNED", color::PURPLE);
    metric(x + 352, y + 196, "GO ABI", "VERSION 1", color::GREEN);
    metric(x + 28, y + 272, "INPUT", "PS2 SERIAL", color::CYAN);
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
    framebuffer::rect(0, 568, 800, 32, color::PANEL);
    framebuffer::rect(0, 568, 800, 1, 0x0030_374C);
    framebuffer::text(18, 580, "\x60 APPS", color::WHITE, 1);
    let mut x = 105;
    for app in AppKind::ALL {
        if app == desktop.active {
            framebuffer::rect(x - 7, 575, 82, 19, color::PURPLE);
        }
        framebuffer::text(
            x,
            581,
            app.shortcut(),
            if app == desktop.active {
                color::WHITE
            } else {
                color::MUTED
            },
            1,
        );
        framebuffer::text(
            x + 12,
            581,
            app.title(),
            if app == desktop.active {
                color::WHITE
            } else {
                color::MUTED
            },
            1,
        );
        x += 108;
    }
}

fn draw_launcher(desktop: &DesktopState) {
    framebuffer::rect(24, 54, 280, 360, 0x0005_0710);
    framebuffer::rect(18, 48, 280, 360, color::PANEL);
    framebuffer::outline(18, 48, 280, 360, color::PURPLE);
    framebuffer::text(38, 70, "FORM LAUNCHER", color::WHITE, 2);
    framebuffer::text(38, 96, "SELECT AN INTERFACE", color::MUTED, 1);
    for (index, app) in AppKind::ALL.iter().copied().enumerate() {
        let y = 122 + index as i32 * 42;
        framebuffer::rect(
            34,
            y,
            248,
            33,
            if app == desktop.active {
                0x0035_285E
            } else {
                0x0025_2B3D
            },
        );
        framebuffer::rect(
            45,
            y + 9,
            16,
            16,
            if app == desktop.active {
                color::PURPLE
            } else {
                color::CYAN
            },
        );
        framebuffer::text(49, y + 14, app.shortcut(), color::WHITE, 1);
        framebuffer::text(77, y + 13, app.title(), color::WHITE, 1);
    }
    framebuffer::text(38, 386, "\x60 OR ESC CLOSE", color::MUTED, 1);
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
