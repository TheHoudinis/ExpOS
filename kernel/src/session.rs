use crate::{
    framebuffer,
    input::{Input, InputEvent},
    print, println, slog,
};
use framebuffer::color;
use hexa_core::Authority;

const FIELD_CAPACITY: usize = 24;
const MAX_ACCOUNTS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Field {
    bytes: [u8; FIELD_CAPACITY],
    len: u8,
}

impl Field {
    const EMPTY: Self = Self {
        bytes: [0; FIELD_CAPACITY],
        len: 0,
    };

    const fn from_static(value: &[u8]) -> Self {
        let mut field = Self::EMPTY;
        let mut index = 0;
        while index < value.len() && index < FIELD_CAPACITY {
            field.bytes[index] = value[index];
            index += 1;
        }
        field.len = index as u8;
        field
    }

    fn from_input(value: &[u8], lowercase: bool) -> Self {
        let mut field = Self::EMPTY;
        let length = value.len().min(FIELD_CAPACITY);
        for (index, byte) in value[..length].iter().copied().enumerate() {
            field.bytes[index] = if lowercase {
                byte.to_ascii_lowercase()
            } else {
                byte
            };
        }
        field.len = length as u8;
        field
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("invalid")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    name: Field,
    authority: Authority,
}

impl Session {
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub const fn authority(&self) -> Authority {
        self.authority
    }

    pub const fn authority_name(&self) -> &'static str {
        match self.authority {
            Authority::Operator => "Operator",
            Authority::Power => "Power",
            Authority::Guest => "Guest",
        }
    }
}

#[derive(Clone, Copy)]
struct Account {
    name: Field,
    password: Field,
    authority: Authority,
    occupied: bool,
}

impl Account {
    const EMPTY: Self = Self {
        name: Field::EMPTY,
        password: Field::EMPTY,
        authority: Authority::Guest,
        occupied: false,
    };

    const fn builtin(name: &[u8], password: &[u8], authority: Authority) -> Self {
        Self {
            name: Field::from_static(name),
            password: Field::from_static(password),
            authority,
            occupied: true,
        }
    }

    const fn session(self) -> Session {
        Session {
            name: self.name,
            authority: self.authority,
        }
    }
}

struct AccountStore {
    accounts: [Account; MAX_ACCOUNTS],
}

impl AccountStore {
    const fn new() -> Self {
        let mut accounts = [Account::EMPTY; MAX_ACCOUNTS];
        accounts[0] = Account::builtin(b"operator", b"expos", Authority::Operator);
        accounts[1] = Account::builtin(b"developer", b"prism", Authority::Power);
        accounts[2] = Account::builtin(b"guest", b"guest", Authority::Guest);
        Self { accounts }
    }
}

static ACCOUNTS: crate::sync::SpinMutex<AccountStore> =
    crate::sync::SpinMutex::new(AccountStore::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountError {
    InvalidName,
    InvalidPassword,
    Duplicate,
    Full,
    Missing,
    Protected,
    Active,
}

impl AccountError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidName => "name must be 2-16 lowercase letters, digits, - or _",
            Self::InvalidPassword => "password must contain 4-23 printable characters",
            Self::Duplicate => "that user already exists",
            Self::Full => "the alpha account registry is full",
            Self::Missing => "user not found",
            Self::Protected => "the built-in operator account is protected",
            Self::Active => "cannot delete the active user",
        }
    }
}

pub fn visit_accounts(mut visitor: impl FnMut(&str, Authority)) {
    let store = ACCOUNTS.lock();
    for account in store.accounts.iter().filter(|account| account.occupied) {
        visitor(account.name.as_str(), account.authority);
    }
}

pub fn account_count() -> usize {
    ACCOUNTS
        .lock()
        .accounts
        .iter()
        .filter(|account| account.occupied)
        .count()
}

pub fn add_account(name: &str, password: &str, authority: Authority) -> Result<(), AccountError> {
    if !valid_name(name.as_bytes()) {
        return Err(AccountError::InvalidName);
    }
    if !valid_password(password.as_bytes()) {
        return Err(AccountError::InvalidPassword);
    }
    let normalized = Field::from_input(name.as_bytes(), true);
    let mut store = ACCOUNTS.lock();
    if store
        .accounts
        .iter()
        .any(|account| account.occupied && account.name.as_bytes() == normalized.as_bytes())
    {
        return Err(AccountError::Duplicate);
    }
    let Some(slot) = store.accounts.iter_mut().find(|account| !account.occupied) else {
        return Err(AccountError::Full);
    };
    *slot = Account {
        name: normalized,
        password: Field::from_input(password.as_bytes(), false),
        authority,
        occupied: true,
    };
    Ok(())
}

pub fn remove_account(name: &str, active_name: &str) -> Result<(), AccountError> {
    let normalized = Field::from_input(name.as_bytes(), true);
    if normalized.as_bytes() == b"operator" {
        return Err(AccountError::Protected);
    }
    if normalized.as_bytes() == active_name.as_bytes() {
        return Err(AccountError::Active);
    }
    let mut store = ACCOUNTS.lock();
    let Some(account) = store
        .accounts
        .iter_mut()
        .find(|account| account.occupied && account.name.as_bytes() == normalized.as_bytes())
    else {
        return Err(AccountError::Missing);
    };
    *account = Account::EMPTY;
    Ok(())
}

pub fn change_password(name: &str, password: &str) -> Result<(), AccountError> {
    if !valid_password(password.as_bytes()) {
        return Err(AccountError::InvalidPassword);
    }
    let normalized = Field::from_input(name.as_bytes(), true);
    let mut store = ACCOUNTS.lock();
    let Some(account) = store
        .accounts
        .iter_mut()
        .find(|account| account.occupied && account.name.as_bytes() == normalized.as_bytes())
    else {
        return Err(AccountError::Missing);
    };
    account.password = Field::from_input(password.as_bytes(), false);
    Ok(())
}

fn valid_name(name: &[u8]) -> bool {
    (2..=16).contains(&name.len())
        && name
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(byte))
}

fn valid_password(password: &[u8]) -> bool {
    (4..FIELD_CAPACITY).contains(&password.len())
        && password.iter().all(|byte| byte.is_ascii_graphic())
}

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
    let normalized = Field::from_input(username, true);
    ACCOUNTS
        .lock()
        .accounts
        .iter()
        .find(|account| {
            account.occupied
                && account.name.as_bytes() == normalized.as_bytes()
                && account.password.as_bytes() == password
        })
        .copied()
        .map(Account::session)
}

fn render_login(
    username: &[u8; FIELD_CAPACITY],
    username_len: usize,
    password_len: usize,
    password_field: bool,
    denied: bool,
) {
    framebuffer::vertical_gradient(0, 0, 800, 600, 0x0004_060B, 0x0011_0A1D);
    framebuffer::alpha_rect(42, 54, 340, 492, 0x003E_176E, 72);
    framebuffer::line(68, 470, 352, 92, 0x0044_2870);
    framebuffer::line(42, 310, 382, 170, 0x0029_5E78);
    framebuffer::rounded_rect(98, 150, 138, 138, 28, 0x0017_1B26);
    framebuffer::rounded_rect(132, 184, 70, 70, 18, color::PURPLE);
    framebuffer::text(151, 207, "EX", color::WHITE, 3);
    framebuffer::text(91, 330, "EXPOS PRISM", color::WHITE, 3);
    framebuffer::text(92, 372, "FORM NATIVE SESSION", color::MUTED, 1);
    framebuffer::rounded_rect(432, 76, 300, 446, 14, 0x000C_0F16);
    framebuffer::outline(432, 76, 300, 446, color::BORDER);
    framebuffer::rounded_rect(542, 112, 80, 80, 22, 0x001D_172C);
    framebuffer::text(570, 138, "EX", color::PURPLE, 2);
    framebuffer::text(515, 218, "WELCOME TO EXPOS", color::INK, 2);
    framebuffer::text(520, 248, "PRISM SESSION LOGIN", color::MUTED, 1);
    login_field(474, 288, 216, "USER", !password_field);
    if let Ok(name) = core::str::from_utf8(&username[..username_len]) {
        framebuffer::text(488, 315, name, color::INK, 1);
    }
    login_field(474, 348, 216, "PASSWORD", password_field);
    for index in 0..password_len.min(16) {
        framebuffer::text(488 + index as i32 * 12, 375, "*", color::INK, 2);
    }
    framebuffer::rounded_rect(474, 418, 216, 42, 8, color::PURPLE);
    framebuffer::text(539, 433, "SIGN IN", color::WHITE, 2);
    framebuffer::text(488, 478, "TAB SWITCHES FIELDS", color::MUTED, 1);
    if denied {
        framebuffer::text(481, 498, "LOGIN DENIED - TRY AGAIN", color::RED, 1);
    } else {
        framebuffer::text(473, 498, "OPERATOR DEFAULT: expos", color::MUTED, 1);
    }
}

fn login_field(x: i32, y: i32, width: i32, label: &str, active: bool) {
    framebuffer::text(x, y, label, color::MUTED, 1);
    framebuffer::rounded_rect(x, y + 17, width, 34, 6, 0x0018_1D27);
    framebuffer::outline(
        x,
        y + 17,
        width,
        34,
        if active { color::PURPLE } else { color::BORDER },
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
    framebuffer::clear(color::BACKGROUND);
    framebuffer::text(284, 252, "WELCOME", color::WHITE, 3);
    framebuffer::text(334, 294, session.name(), 0x00B9_DCFF, 2);
}
