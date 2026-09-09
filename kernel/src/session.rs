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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootMode {
    Graphical,
    Console,
}

impl BootMode {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Graphical => "graphical",
            Self::Console => "console",
        }
    }

    const fn alternate(self) -> Self {
        match self {
            Self::Graphical => Self::Console,
            Self::Console => Self::Graphical,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoginResult {
    pub session: Session,
    pub mode: BootMode,
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

pub fn choose_boot_mode(input: &mut Input) -> BootMode {
    let _ = input.enable_mouse();
    if !framebuffer::enter() {
        slog!("HEXA_BOOT_MODE console fallback=true\r\n");
        return BootMode::Console;
    }
    let mut selected = BootMode::Graphical;
    let mut pointer_x = (framebuffer::WIDTH / 2) as i16;
    let mut pointer_y = (framebuffer::HEIGHT / 2) as i16;
    render_boot_mode(selected);
    draw_login_cursor(pointer_x, pointer_y);
    slog!("HEXA_BOOT_MODE_READY\r\n");
    loop {
        let Some(event) = input.poll_event() else {
            core::hint::spin_loop();
            continue;
        };
        let mut accepted = false;
        match event {
            InputEvent::Key(b'1' | b'g' | b'G') | InputEvent::Key(crate::input::KEY_UP) => {
                selected = BootMode::Graphical;
            }
            InputEvent::Key(b'2' | b'c' | b'C') | InputEvent::Key(crate::input::KEY_DOWN) => {
                selected = BootMode::Console;
            }
            InputEvent::Key(b'\n') => accepted = true,
            InputEvent::Pointer(pointer) => {
                pointer_x = pointer_x
                    .saturating_add(pointer.dx)
                    .clamp(0, framebuffer::WIDTH as i16 - 1);
                pointer_y = pointer_y
                    .saturating_add(pointer.dy)
                    .clamp(0, framebuffer::HEIGHT as i16 - 1);
                let center_x = framebuffer::WIDTH as i16 / 2;
                let center_y = framebuffer::HEIGHT as i16 / 2;
                if pointer.pressed & 1 != 0 {
                    if (center_x - 250..center_x - 10).contains(&pointer_x)
                        && (center_y - 35..center_y + 45).contains(&pointer_y)
                    {
                        selected = BootMode::Graphical;
                        accepted = true;
                    } else if (center_x + 10..center_x + 250).contains(&pointer_x)
                        && (center_y - 35..center_y + 45).contains(&pointer_y)
                    {
                        selected = BootMode::Console;
                        accepted = true;
                    }
                }
            }
            _ => {}
        }
        if accepted {
            framebuffer::exit();
            crate::clear_console();
            slog!("HEXA_BOOT_MODE {}\r\n", selected.name());
            return selected;
        }
        render_boot_mode(selected);
        draw_login_cursor(pointer_x, pointer_y);
    }
}

pub fn login(input: &mut Input, initial_mode: BootMode) -> LoginResult {
    let mut mode = initial_mode;
    loop {
        match login_once(input, mode) {
            LoginAttempt::Authenticated(session) => return LoginResult { session, mode },
            LoginAttempt::SwitchEnvironment => {
                mode = mode.alternate();
                slog!("HEXA_LOGIN_ENVIRONMENT {}\r\n", mode.name());
            }
        }
    }
}

enum LoginAttempt {
    Authenticated(Session),
    SwitchEnvironment,
}

fn login_once(input: &mut Input, mode: BootMode) -> LoginAttempt {
    let _ = input.enable_mouse();
    let graphical = mode == BootMode::Graphical && framebuffer::enter();
    if mode == BootMode::Graphical && !graphical {
        return LoginAttempt::SwitchEnvironment;
    }
    let mut username = [0_u8; FIELD_CAPACITY];
    let mut username_len = 0;
    let mut password = [0_u8; FIELD_CAPACITY];
    let mut password_len = 0;
    let mut password_field = false;
    let mut denied = false;
    let mut pointer_x = (framebuffer::WIDTH / 2) as i16;
    let mut pointer_y = (framebuffer::HEIGHT / 2) as i16;
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
        println!("Press Esc to use the graphical login.");
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
                    let layout = login_layout();
                    if (layout.field_x..layout.field_x + layout.field_width)
                        .contains(&(pointer_x as i32))
                        && (layout.user_y..layout.user_y + 60).contains(&(pointer_y as i32))
                    {
                        password_field = false;
                    } else if (layout.field_x..layout.field_x + layout.field_width)
                        .contains(&(pointer_x as i32))
                        && (layout.password_y..layout.password_y + 60).contains(&(pointer_y as i32))
                    {
                        password_field = true;
                    } else if (layout.field_x..layout.field_x + layout.field_width)
                        .contains(&(pointer_x as i32))
                        && (layout.sign_in_y..layout.sign_in_y + 42).contains(&(pointer_y as i32))
                    {
                        if let Some(session) =
                            authenticate(&username[..username_len], &password[..password_len])
                        {
                            return LoginAttempt::Authenticated(complete_login(session, true));
                        }
                        denied = true;
                        username_len = 0;
                        password_len = 0;
                        password_field = false;
                    } else if (layout.field_x..layout.field_x + layout.field_width)
                        .contains(&(pointer_x as i32))
                        && (layout.switch_y..layout.switch_y + 34).contains(&(pointer_y as i32))
                    {
                        framebuffer::exit();
                        crate::clear_console();
                        return LoginAttempt::SwitchEnvironment;
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
            0x1B => {
                if graphical {
                    framebuffer::exit();
                }
                crate::clear_console();
                return LoginAttempt::SwitchEnvironment;
            }
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
                    return LoginAttempt::Authenticated(complete_login(session, graphical));
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

fn render_boot_mode(selected: BootMode) {
    let width = framebuffer::WIDTH as i32;
    let height = framebuffer::HEIGHT as i32;
    let center_x = width / 2;
    let center_y = height / 2;
    framebuffer::clear(0x000B_0D10);
    framebuffer::text(center_x - 22, center_y - 120, "ExpOS", color::WHITE, 2);
    boot_mode_card(
        center_x - 250,
        center_y - 35,
        "1  Desktop",
        selected == BootMode::Graphical,
    );
    boot_mode_card(
        center_x + 10,
        center_y - 35,
        "2  Console",
        selected == BootMode::Console,
    );
    framebuffer::text(center_x - 20, center_y + 86, "Enter", color::MUTED, 1);
}

fn boot_mode_card(x: i32, y: i32, title: &str, selected: bool) {
    framebuffer::rounded_rect(
        x,
        y,
        240,
        80,
        8,
        if selected { 0x0024_292F } else { 0x0013_161A },
    );
    framebuffer::outline(
        x,
        y,
        240,
        80,
        if selected {
            color::WHITE
        } else {
            color::BORDER
        },
    );
    framebuffer::text(x + 30, y + 29, title, color::INK, 2);
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

#[derive(Clone, Copy)]
struct LoginLayout {
    card_x: i32,
    card_y: i32,
    card_width: i32,
    field_x: i32,
    field_width: i32,
    user_y: i32,
    password_y: i32,
    sign_in_y: i32,
    switch_y: i32,
}

fn login_layout() -> LoginLayout {
    let width = framebuffer::WIDTH as i32;
    let height = framebuffer::HEIGHT as i32;
    let card_width = if width >= 1_000 { 400 } else { 344 };
    let card_height = 430;
    let card_x = (width - card_width) / 2;
    let card_y = ((height - card_height) / 2).max(16);
    LoginLayout {
        card_x,
        card_y,
        card_width,
        field_x: card_x + 40,
        field_width: card_width - 80,
        user_y: card_y + 110,
        password_y: card_y + 185,
        sign_in_y: card_y + 270,
        switch_y: card_y + 326,
    }
}

fn render_login(
    username: &[u8; FIELD_CAPACITY],
    username_len: usize,
    password_len: usize,
    password_field: bool,
    denied: bool,
) {
    let width = framebuffer::WIDTH as i32;
    let height = framebuffer::HEIGHT as i32;
    let layout = login_layout();
    framebuffer::rect(0, 0, width, height, 0x000B_0D10);
    framebuffer::rounded_rect(
        layout.card_x,
        layout.card_y,
        layout.card_width,
        430,
        10,
        0x0013_161A,
    );
    framebuffer::outline(
        layout.card_x,
        layout.card_y,
        layout.card_width,
        430,
        color::BORDER,
    );
    framebuffer::text(
        layout.card_x + 40,
        layout.card_y + 45,
        "ExpOS",
        color::INK,
        2,
    );
    login_field(
        layout.field_x,
        layout.user_y,
        layout.field_width,
        "Username",
        !password_field,
    );
    if let Ok(name) = core::str::from_utf8(&username[..username_len]) {
        framebuffer::text(layout.field_x + 14, layout.user_y + 27, name, color::INK, 1);
    }
    login_field(
        layout.field_x,
        layout.password_y,
        layout.field_width,
        "Password",
        password_field,
    );
    for index in 0..password_len.min(20) {
        framebuffer::text(
            layout.field_x + 14 + index as i32 * 12,
            layout.password_y + 27,
            "*",
            color::INK,
            2,
        );
    }
    framebuffer::rounded_rect(
        layout.field_x,
        layout.sign_in_y,
        layout.field_width,
        42,
        8,
        color::GREEN,
    );
    framebuffer::text(
        layout.field_x + layout.field_width / 2 - 36,
        layout.sign_in_y + 15,
        "Sign in",
        color::WHITE,
        2,
    );
    framebuffer::rounded_rect(
        layout.field_x,
        layout.switch_y,
        layout.field_width,
        34,
        7,
        0x0018_1D27,
    );
    framebuffer::outline(
        layout.field_x,
        layout.switch_y,
        layout.field_width,
        34,
        color::BORDER,
    );
    framebuffer::text(
        layout.field_x + layout.field_width / 2 - 52,
        layout.switch_y + 11,
        "Console login",
        color::INK,
        1,
    );
    if denied {
        framebuffer::text(
            layout.field_x,
            layout.card_y + 390,
            "Login denied",
            color::RED,
            1,
        );
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
        if active { color::WHITE } else { color::BORDER },
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
    let center_x = framebuffer::WIDTH as i32 / 2;
    let center_y = framebuffer::HEIGHT as i32 / 2;
    let name_width = session.name().len() as i32 * framebuffer::text_advance(2);
    framebuffer::text(
        center_x - name_width / 2,
        center_y - 20,
        session.name(),
        color::WHITE,
        2,
    );
}
