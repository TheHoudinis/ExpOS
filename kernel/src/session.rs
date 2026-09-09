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
                    if (center_x - 310..center_x - 10).contains(&pointer_x)
                        && (center_y - 65..center_y + 95).contains(&pointer_y)
                    {
                        selected = BootMode::Graphical;
                        accepted = true;
                    } else if (center_x + 10..center_x + 310).contains(&pointer_x)
                        && (center_y - 65..center_y + 95).contains(&pointer_y)
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
    framebuffer::vertical_gradient(0, 0, width, height, 0x0004_060B, 0x0014_0B20);
    framebuffer::text(
        center_x - 144,
        center_y - 180,
        "START EXPOS",
        color::WHITE,
        4,
    );
    framebuffer::text(
        center_x - 178,
        center_y - 132,
        "CHOOSE YOUR SESSION ENVIRONMENT",
        color::MUTED,
        2,
    );
    boot_mode_card(
        center_x - 310,
        center_y - 65,
        "1  PRISM",
        "GRAPHICAL DESKTOP",
        selected == BootMode::Graphical,
    );
    boot_mode_card(
        center_x + 10,
        center_y - 65,
        "2  CONSOLE",
        "DIRECT COMMAND SHELL",
        selected == BootMode::Console,
    );
    framebuffer::text(
        center_x - 166,
        center_y + 140,
        "ARROWS OR 1/2  ENTER TO CONTINUE",
        color::MUTED,
        1,
    );
}

fn boot_mode_card(x: i32, y: i32, title: &str, detail: &str, selected: bool) {
    framebuffer::rounded_rect(
        x,
        y,
        300,
        160,
        14,
        if selected { 0x0028_1C45 } else { 0x000D_1119 },
    );
    framebuffer::outline(
        x,
        y,
        300,
        160,
        if selected {
            color::PURPLE
        } else {
            color::BORDER
        },
    );
    framebuffer::text(x + 28, y + 38, title, color::INK, 3);
    framebuffer::text(x + 28, y + 88, detail, color::MUTED, 1);
    framebuffer::text(
        x + 28,
        y + 116,
        if selected { "SELECTED" } else { "AVAILABLE" },
        if selected { color::CYAN } else { color::MUTED },
        1,
    );
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
    let card_width = if width >= 1_000 { 380 } else { 344 };
    let card_height = 568;
    let card_x = width - card_width - 42;
    let card_y = ((height - card_height) / 2).max(16);
    LoginLayout {
        card_x,
        card_y,
        card_width,
        field_x: card_x + 42,
        field_width: card_width - 84,
        user_y: card_y + 216,
        password_y: card_y + 286,
        sign_in_y: card_y + 364,
        switch_y: card_y + 420,
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
    let left_center = layout.card_x / 2;
    framebuffer::vertical_gradient(0, 0, width, height, 0x0004_060B, 0x0011_0A1D);
    framebuffer::alpha_rect(42, 42, layout.card_x - 84, height - 84, 0x003E_176E, 72);
    framebuffer::line(68, height - 96, layout.card_x - 62, 92, 0x0044_2870);
    framebuffer::line(42, height / 2 + 10, layout.card_x - 42, 170, 0x0029_5E78);
    framebuffer::rounded_rect(
        left_center - 69,
        height / 2 - 160,
        138,
        138,
        28,
        0x0017_1B26,
    );
    framebuffer::rounded_rect(
        left_center - 35,
        height / 2 - 126,
        70,
        70,
        18,
        color::PURPLE,
    );
    framebuffer::text(left_center - 16, height / 2 - 103, "EX", color::WHITE, 3);
    framebuffer::text(
        left_center - 88,
        height / 2 + 20,
        "EXPOS PRISM",
        color::WHITE,
        3,
    );
    framebuffer::text(
        left_center - 96,
        height / 2 + 62,
        "FORM NATIVE SESSION",
        color::MUTED,
        1,
    );
    framebuffer::rounded_rect(
        layout.card_x,
        layout.card_y,
        layout.card_width,
        568,
        14,
        0x000C_0F16,
    );
    framebuffer::outline(
        layout.card_x,
        layout.card_y,
        layout.card_width,
        568,
        color::BORDER,
    );
    let badge_x = layout.card_x + layout.card_width / 2 - 40;
    framebuffer::rounded_rect(badge_x, layout.card_y + 36, 80, 80, 22, 0x001D_172C);
    framebuffer::text(badge_x + 28, layout.card_y + 62, "EX", color::PURPLE, 2);
    framebuffer::text(
        layout.card_x + 82,
        layout.card_y + 142,
        "WELCOME TO EXPOS",
        color::INK,
        2,
    );
    framebuffer::text(
        layout.card_x + 94,
        layout.card_y + 174,
        "PRISM SESSION LOGIN",
        color::MUTED,
        1,
    );
    login_field(
        layout.field_x,
        layout.user_y,
        layout.field_width,
        "USER",
        !password_field,
    );
    if let Ok(name) = core::str::from_utf8(&username[..username_len]) {
        framebuffer::text(layout.field_x + 14, layout.user_y + 27, name, color::INK, 1);
    }
    login_field(
        layout.field_x,
        layout.password_y,
        layout.field_width,
        "PASSWORD",
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
        color::PURPLE,
    );
    framebuffer::text(
        layout.field_x + layout.field_width / 2 - 36,
        layout.sign_in_y + 15,
        "SIGN IN",
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
        layout.field_x + layout.field_width / 2 - 72,
        layout.switch_y + 11,
        "USE CONSOLE LOGIN",
        color::INK,
        1,
    );
    framebuffer::text(
        layout.field_x + 30,
        layout.card_y + 472,
        "TAB SWITCHES FIELDS  ESC CHANGES MODE",
        color::MUTED,
        1,
    );
    if denied {
        framebuffer::text(
            layout.field_x + 20,
            layout.card_y + 518,
            "LOGIN DENIED - TRY AGAIN",
            color::RED,
            1,
        );
    } else {
        framebuffer::text(
            layout.field_x + 14,
            layout.card_y + 518,
            "OPERATOR DEFAULT: expos",
            color::MUTED,
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
    let center_x = framebuffer::WIDTH as i32 / 2;
    let center_y = framebuffer::HEIGHT as i32 / 2;
    framebuffer::text(center_x - 116, center_y - 48, "WELCOME", color::WHITE, 3);
    framebuffer::text(center_x - 42, center_y + 4, session.name(), 0x00B9_DCFF, 2);
}
