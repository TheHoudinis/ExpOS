use crate::{
    framebuffer,
    input::{Input, InputEvent},
    print, println, slog,
};
use framebuffer::color;
use hexa_core::Authority;

const FIELD_CAPACITY: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    name: &'static str,
    authority: Authority,
}

impl Session {
    pub const fn name(self) -> &'static str {
        self.name
    }

    pub const fn authority(self) -> Authority {
        self.authority
    }

    pub const fn authority_name(self) -> &'static str {
        match self.authority {
            Authority::Operator => "Operator",
            Authority::Power => "Power",
            Authority::Guest => "Guest",
        }
    }
}

pub const ACCOUNTS: [Session; 3] = [
    Session {
        name: "operator",
        authority: Authority::Operator,
    },
    Session {
        name: "developer",
        authority: Authority::Power,
    },
    Session {
        name: "guest",
        authority: Authority::Guest,
    },
];

pub fn login(input: &mut Input) -> Session {
    let _ = input.enable_mouse();
    let graphical = framebuffer::enter();
    let mut username = [0_u8; FIELD_CAPACITY];
    let mut username_len = 0;
    let mut password = [0_u8; FIELD_CAPACITY];
    let mut password_len = 0;
    let mut password_field = false;
    let mut denied = false;
    let mut pointer_x = 400_i16;
    let mut pointer_y = 300_i16;
    slog!("HEXA_LOGIN_READY\r\n");
    if graphical {
        render_login(
            &username,
            username_len,
            password_len,
            password_field,
            denied,
        );
        draw_login_cursor(pointer_x, pointer_y);
    } else {
        println!("ExpOS login");
        print!("user: ");
    }

    loop {
        let Some(event) = input.poll_event() else {
            core::hint::spin_loop();
            continue;
        };
        let key = match event {
            InputEvent::Key(key) => Some(key),
            InputEvent::Pointer(pointer) if graphical => {
                pointer_x = pointer_x
                    .saturating_add(pointer.dx)
                    .clamp(0, framebuffer::WIDTH as i16 - 1);
                pointer_y = pointer_y
                    .saturating_add(pointer.dy)
                    .clamp(0, framebuffer::HEIGHT as i16 - 1);
                if pointer.pressed & 1 != 0 {
                    if (474..690).contains(&pointer_x) && (288..348).contains(&pointer_y) {
                        password_field = false;
                    } else if (474..690).contains(&pointer_x) && (348..408).contains(&pointer_y) {
                        password_field = true;
                    } else if (474..690).contains(&pointer_x) && (418..460).contains(&pointer_y) {
                        if let Some(session) =
                            authenticate(&username[..username_len], &password[..password_len])
                        {
                            return complete_login(session, true);
                        }
                        denied = true;
                        username_len = 0;
                        password_len = 0;
                        password_field = false;
                    }
                }
                None
            }
            InputEvent::Pointer(_) => None,
        };
        let Some(key) = key else {
            if graphical {
                render_login(
                    &username,
                    username_len,
                    password_len,
                    password_field,
                    denied,
                );
                draw_login_cursor(pointer_x, pointer_y);
            }
            continue;
        };
        match key {
            b'\t' => password_field = !password_field,
            0x08 => {
                if password_field {
                    password_len = password_len.saturating_sub(1);
                } else {
                    username_len = username_len.saturating_sub(1);
                }
            }
            b'\n' if !password_field => {
                password_field = true;
                if !graphical {
                    println!();
                    print!("password: ");
                }
            }
            b'\n' => {
                if let Some(session) =
                    authenticate(&username[..username_len], &password[..password_len])
                {
                    return complete_login(session, graphical);
                }
                denied = true;
                username_len = 0;
                password_len = 0;
                password_field = false;
                if !graphical {
                    println!("\nLogin denied. Try operator / expos.");
                    print!("user: ");
                }
            }
            byte @ 0x20..=0x7E => {
                denied = false;
                if password_field && password_len < FIELD_CAPACITY {
                    password[password_len] = byte;
                    password_len += 1;
                    if !graphical {
                        print!("*");
                    }
                } else if !password_field && username_len < FIELD_CAPACITY {
                    username[username_len] = byte.to_ascii_lowercase();
                    username_len += 1;
                    if !graphical {
                        print!("{}", byte as char);
                    }
                }
            }
            _ => {}
        }
        if graphical {
            render_login(
                &username,
                username_len,
                password_len,
                password_field,
                denied,
            );
            draw_login_cursor(pointer_x, pointer_y);
        }
    }
}

fn complete_login(session: Session, graphical: bool) -> Session {
    if graphical {
        render_welcome(session);
        framebuffer::exit();
    }
    crate::clear_console();
    println!(
        "Signed in as {} with {} authority.",
        session.name(),
        session.authority_name()
    );
    slog!("HEXA_LOGIN_OK user={}\r\n", session.name());
    session
}

fn authenticate(username: &[u8], password: &[u8]) -> Option<Session> {
    match (username, password) {
        (b"operator", b"expos") => Some(ACCOUNTS[0]),
        (b"developer", b"prism") => Some(ACCOUNTS[1]),
        (b"guest", b"guest") => Some(ACCOUNTS[2]),
        _ => None,
    }
}

fn render_login(
    username: &[u8; FIELD_CAPACITY],
    username_len: usize,
    password_len: usize,
    password_field: bool,
    denied: bool,
) {
    framebuffer::clear(0x0009_3976);
    for row in 0..20 {
        framebuffer::rect(0, row * 30, 800, 30, 0x000A_3976 + row as u32 * 0x0000_0202);
    }
    framebuffer::rect(74, 82, 132, 174, 0x0021_73C8);
    framebuffer::rect(216, 60, 152, 196, 0x0032_8DE7);
    framebuffer::rect(74, 266, 132, 174, 0x0019_62B5);
    framebuffer::rect(216, 266, 152, 196, 0x0028_7CD7);
    framebuffer::rect(432, 76, 300, 446, 0x00F2_F7FC);
    framebuffer::outline(432, 76, 300, 446, 0x0096_BDE4);
    framebuffer::rect(542, 112, 80, 80, 0x0000_78D4);
    framebuffer::text(570, 138, "EX", color::WHITE, 2);
    framebuffer::text(515, 218, "WELCOME TO EXPOS", 0x0024_3B55, 2);
    framebuffer::text(520, 248, "PRISM SESSION LOGIN", 0x0064_7890, 1);
    login_field(474, 288, 216, "USER", !password_field);
    if let Ok(name) = core::str::from_utf8(&username[..username_len]) {
        framebuffer::text(488, 315, name, 0x0022_3347, 1);
    }
    login_field(474, 348, 216, "PASSWORD", password_field);
    for index in 0..password_len.min(16) {
        framebuffer::text(488 + index as i32 * 12, 375, "*", 0x0022_3347, 2);
    }
    framebuffer::rect(474, 418, 216, 42, 0x0000_78D4);
    framebuffer::text(539, 433, "SIGN IN", color::WHITE, 2);
    framebuffer::text(488, 478, "TAB SWITCHES FIELDS", 0x0064_7890, 1);
    if denied {
        framebuffer::text(481, 498, "LOGIN DENIED - TRY AGAIN", color::RED, 1);
    } else {
        framebuffer::text(473, 498, "OPERATOR DEFAULT: expos", 0x0064_7890, 1);
    }
}

fn login_field(x: i32, y: i32, width: i32, label: &str, active: bool) {
    framebuffer::text(x, y, label, 0x0058_6D84, 1);
    framebuffer::rect(x, y + 17, width, 34, color::WHITE);
    framebuffer::outline(
        x,
        y + 17,
        width,
        34,
        if active { 0x0000_78D4 } else { 0x00B6_C8DA },
    );
}

fn draw_login_cursor(x: i16, y: i16) {
    for row in 0..16 {
        let width = (row / 2 + 1).min(8);
        for column in 0..width {
            let edge = column == 0 || column == width - 1 || row == 0 || row == 15;
            framebuffer::pixel(
                x as i32 + column,
                y as i32 + row,
                if edge { 0x0012_1822 } else { color::WHITE },
            );
        }
    }
}

fn render_welcome(session: Session) {
    framebuffer::clear(0x000A_3976);
    framebuffer::text(284, 252, "WELCOME", color::WHITE, 3);
    framebuffer::text(334, 294, session.name(), 0x00B9_DCFF, 2);
}
