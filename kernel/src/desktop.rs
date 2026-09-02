use crate::{framebuffer, input::Input, slog};
use framebuffer::color;
use hexa_core::{
    BufferFormat, BufferHandle, DisplayServer, Document, Fin, NodeKind, Rect, SurfaceRole,
};

pub const DISPLAY_FIN: Fin = Fin::from_u128(0x4449_5350_4C41_5900_0000_0000_0000_0001);
pub const BROWSER_FIN: Fin = Fin::from_u128(0x4252_4F57_5345_5200_0000_0000_0000_0001);

const HOME: &str = "<title>Hexa Home</title><h1>Welcome to HexaOS</h1><p>The environment is built from Forms with stable identity capabilities and relationships.</p><h2>Start here</h2><a href='hexa://about'>1 About this browser</a><a href='hexa://packages'>2 Package Forms</a><a href='hexa://system'>3 System status</a><p>Press 1 2 or 3 to open a link. Press Q to return to the shell.</p>";
const ABOUT: &str = "<title>About</title><h1>Hexa Browser</h1><p>This native Interface Form parses bounded local HTML and renders it through HexaDisplay.</p><p>Surface state is owner scoped and becomes visible only after an atomic commit.</p><p>HTTPS CSS JavaScript and media are intentionally not claimed before networking isolation and a complete web engine exist.</p><a href='hexa://home'>H Home</a>";
const PACKAGES: &str = "<title>Packages</title><h1>Package Forms</h1><p>Ayo activates software as Forms rather than copying archives into Unix paths.</p><li>CoreTools diagnostics and repair</li><li>Network socket capability</li><li>Terminal command Interface</li><li>Browser document Interface</li><p>Use the host ayo TUI until native persistent HexaFS and Go execution contexts are connected.</p><a href='hexa://home'>H Home</a>";
const SYSTEM: &str = "<title>System</title><h1>System Scope</h1><p>HexaDisplay protocol version 1 is active.</p><li>800 by 600 XRGB framebuffer</li><li>Atomic attach damage and commit</li><li>Focus z order and hit testing</li><li>PS2 and serial keyboard routing</li><p>Network Driver Form is not bound. External navigation remains restricted.</p><a href='hexa://home'>H Home</a>";
const NETWORK_BLOCKED: &str = "<title>Network restricted</title><h1>Navigation blocked</h1><p>DIESE denied external navigation because no v8 Network Driver Form is bound and PIMP network policy is restricted.</p><p>This is a safe fallback not a fake internet connection.</p><a href='hexa://home'>H Home</a>";

pub fn run(input: &mut Input, start_browser: bool) {
    if !framebuffer::enter() {
        crate::println!("HexaDisplay unavailable: no Bochs/QEMU VBE framebuffer.");
        slog!("HEXA_DISPLAY_UNAVAILABLE\r\n");
        return;
    }
    let mut server = DisplayServer::new();
    let background = server
        .create_surface(
            DISPLAY_FIN,
            "Root Canvas",
            SurfaceRole::Background,
            Rect::new(0, 0, 800, 600),
        )
        .expect("built-in background surface");
    let browser = server
        .create_surface(
            BROWSER_FIN,
            "Browser",
            SurfaceRole::Window,
            Rect::new(38, 58, 724, 492),
        )
        .expect("built-in browser surface");
    let scope = server
        .create_surface(
            DISPLAY_FIN,
            "System Scope",
            SurfaceRole::Panel,
            Rect::new(560, 430, 180, 92),
        )
        .expect("built-in scope surface");
    let _ = server.attach(DISPLAY_FIN, background, buffer(1, DISPLAY_FIN, 800, 600));
    let _ = server.attach(BROWSER_FIN, browser, buffer(2, BROWSER_FIN, 724, 492));
    let _ = server.attach(DISPLAY_FIN, scope, buffer(3, DISPLAY_FIN, 180, 92));
    let _ = server.commit(DISPLAY_FIN, background);
    let _ = server.commit(BROWSER_FIN, browser);
    let _ = server.commit(DISPLAY_FIN, scope);
    let _ = server.focus(if start_browser { browser } else { scope });

    let mut document = Document::parse("hexa://home", HOME).expect("built-in home document");
    render(&server, &document);
    slog!("HEXA_DISPLAY_READY surfaces=3 commit=3\r\n");

    loop {
        let Some(key) = input.poll() else {
            core::hint::spin_loop();
            continue;
        };
        let _ = server.route_key(key);
        let next = match key.to_ascii_lowercase() {
            b'q' | 0x1B => break,
            b'h' => Some(("hexa://home", HOME)),
            b'1' | b'a' => Some(("hexa://about", ABOUT)),
            b'2' | b'p' => Some(("hexa://packages", PACKAGES)),
            b'3' | b's' => Some(("hexa://system", SYSTEM)),
            b'n' => Some(("hexa://blocked", NETWORK_BLOCKED)),
            b'\t' => {
                let target = if server.focused() == Some(browser) {
                    scope
                } else {
                    browser
                };
                let _ = server.focus(target);
                render(&server, &document);
                None
            }
            _ => None,
        };
        if let Some((url, source)) = next {
            if let Ok(next_document) = Document::parse(url, source) {
                document = next_document;
                let _ = server.damage(BROWSER_FIN, browser, Rect::new(0, 0, 724, 492));
                let _ = server.commit(BROWSER_FIN, browser);
                render(&server, &document);
            }
        }
    }
    framebuffer::exit();
    crate::clear_console();
    crate::println!("HexaDisplay session closed; command environment restored.");
    slog!("HEXA_DISPLAY_CLOSED\r\n");
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

fn render(server: &DisplayServer, document: &Document) {
    framebuffer::clear(color::BACKGROUND);
    for row in 0..14 {
        framebuffer::rect(
            0,
            row * 40,
            800,
            40,
            0x0010_1320 + (row as u32 * 0x0001_0102),
        );
    }
    framebuffer::rect(0, 0, 800, 42, color::PANEL);
    framebuffer::text(20, 13, "HEXAOS", color::WHITE, 2);
    framebuffer::text(126, 14, "STABLE DIMENSION", color::MUTED, 1);
    framebuffer::text(655, 14, "OPERATOR", color::GREEN, 1);

    let browser_focused = server.focused().is_some_and(|id| {
        server
            .surface(id)
            .is_some_and(|surface| surface.owner == BROWSER_FIN)
    });
    framebuffer::rect(32, 52, 736, 504, 0x0008_0A12);
    framebuffer::rect(38, 58, 724, 492, color::WINDOW);
    framebuffer::outline(
        38,
        58,
        724,
        492,
        if browser_focused {
            color::PURPLE
        } else {
            color::BORDER
        },
    );
    framebuffer::rect(38, 58, 724, 38, color::PANEL);
    framebuffer::rect(54, 72, 10, 10, color::RED);
    framebuffer::rect(72, 72, 10, 10, 0x00F4_B942);
    framebuffer::rect(90, 72, 10, 10, color::GREEN);
    framebuffer::text(116, 70, "BROWSER FORM", color::WHITE, 1);

    framebuffer::rect(58, 108, 684, 34, 0x00E7_EAF2);
    framebuffer::outline(58, 108, 684, 34, color::BORDER);
    framebuffer::text(72, 119, document.url(), color::INK, 1);
    framebuffer::rect(58, 153, 684, 1, color::BORDER);

    let mut y = 170;
    for node in document.nodes() {
        if y > 480 {
            break;
        }
        match node.kind {
            NodeKind::Title => {}
            NodeKind::Heading => {
                framebuffer::text(72, y, node.text.as_str(), color::PURPLE, 2);
                y += 34;
            }
            NodeKind::Paragraph => {
                y = wrapped_text(72, y, 610, node.text.as_str(), color::INK, 2) + 12;
            }
            NodeKind::Link => {
                framebuffer::text(76, y, ">", color::CYAN, 2);
                y = wrapped_text(96, y, 580, node.text.as_str(), color::CYAN, 2) + 9;
            }
            NodeKind::ListItem => {
                framebuffer::rect(76, y + 5, 6, 6, color::GREEN);
                y = wrapped_text(94, y, 580, node.text.as_str(), color::INK, 2) + 8;
            }
        }
    }

    framebuffer::rect(58, 514, 684, 24, 0x00E7_EAF2);
    framebuffer::text(
        70,
        522,
        "1 ABOUT   2 PACKAGES   3 SYSTEM   N NETWORK   TAB FOCUS   Q CLOSE",
        color::MUTED,
        1,
    );

    framebuffer::rect(582, 438, 150, 72, color::PANEL);
    framebuffer::outline(
        582,
        438,
        150,
        72,
        if !browser_focused {
            color::GREEN
        } else {
            color::MUTED
        },
    );
    framebuffer::text(594, 450, "SYSTEM SCOPE", color::WHITE, 1);
    framebuffer::text(594, 468, "SURFACES 3", color::GREEN, 1);
    framebuffer::text(594, 484, "COMMITS", color::MUTED, 1);
    draw_number(672, 484, server.commit_sequence(), color::CYAN);

    framebuffer::rect(0, 570, 800, 30, color::PANEL);
    framebuffer::rect(18, 578, 14, 14, color::PURPLE);
    framebuffer::text(43, 581, "HEXA DISPLAY V1", color::WHITE, 1);
    framebuffer::text(648, 581, "NETWORK RESTRICTED", color::MUTED, 1);
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
