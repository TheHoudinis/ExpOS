use crate::{port, serial};
use core::arch::x86_64::_rdtsc;

const PS2_STATUS: u16 = 0x64;
const PS2_DATA: u16 = 0x60;

pub const KEY_UP: u8 = 0x80;
pub const KEY_DOWN: u8 = 0x81;
pub const KEY_LEFT: u8 = 0x82;
pub const KEY_RIGHT: u8 = 0x83;
pub const KEY_SUPER_LAUNCHER: u8 = 0x90;
pub const KEY_SUPER_TERMINAL: u8 = 0x91;
pub const KEY_SUPER_BROWSER: u8 = 0x92;
pub const KEY_SUPER_CLOSE: u8 = 0x93;
pub const KEY_SUPER_CYCLE: u8 = 0x94;
pub const KEY_SUPER_FULLSCREEN: u8 = 0x95;
pub const KEY_SUPER_FLOAT: u8 = 0x96;
pub const KEY_SUPER_OVERVIEW: u8 = 0x97;
pub const KEY_SUPER_WORKSPACE_1: u8 = 0x98;
pub const KEY_SUPER_WORKSPACE_2: u8 = 0x99;
pub const KEY_SUPER_WORKSPACE_3: u8 = 0x9A;
pub const KEY_SUPER_WORKSPACE_4: u8 = 0x9F;
pub const KEY_SUPER_LEFT: u8 = 0x9B;
pub const KEY_SUPER_RIGHT: u8 = 0x9C;
pub const KEY_SUPER_UP: u8 = 0x9D;
pub const KEY_SUPER_DOWN: u8 = 0x9E;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerEvent {
    pub dx: i16,
    pub dy: i16,
    pub buttons: u8,
    pub pressed: u8,
    pub released: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Key(u8),
    Pointer(PointerEvent),
}

pub struct Input {
    shift: bool,
    caps_lock: bool,
    extended: bool,
    super_key: bool,
    serial_escape: u8,
    serial_escape_started: u64,
    mouse_packet: [u8; 3],
    mouse_index: usize,
    mouse_buttons: u8,
    mouse_enabled: bool,
}

impl Input {
    pub const fn new() -> Self {
        Self {
            shift: false,
            caps_lock: false,
            extended: false,
            super_key: false,
            serial_escape: 0,
            serial_escape_started: 0,
            mouse_packet: [0; 3],
            mouse_index: 0,
            mouse_buttons: 0,
            mouse_enabled: false,
        }
    }

    pub fn enable_mouse(&mut self) -> bool {
        if self.mouse_enabled {
            return true;
        }
        if !controller_write_command(0xA8) || !controller_write_command(0x20) {
            return false;
        }
        let Some(mut command_byte) = controller_read(false) else {
            return false;
        };
        command_byte &= !0x22;
        if !controller_write_command(0x60) || !controller_write_data(command_byte) {
            return false;
        }
        if !mouse_command(0xF6) || !mouse_command(0xF4) {
            return false;
        }
        self.mouse_index = 0;
        self.mouse_buttons = 0;
        self.mouse_enabled = true;
        true
    }

    /// Poll serial first, then the PS/2 controller used by the QEMU window.
    pub fn poll(&mut self) -> Option<u8> {
        match self.poll_event()? {
            InputEvent::Key(key) => Some(key),
            InputEvent::Pointer(_) => None,
        }
    }

    pub fn poll_event(&mut self) -> Option<InputEvent> {
        if let Some(byte) = serial::COM1.lock().try_read() {
            return self.decode_serial(byte).map(InputEvent::Key);
        }
        if self.serial_escape != 0
            && unsafe { _rdtsc() }.wrapping_sub(self.serial_escape_started) > 5_000_000
        {
            self.serial_escape = 0;
            return Some(InputEvent::Key(0x1B));
        }
        let status = unsafe { port::inb(PS2_STATUS) };
        if status & 0x01 == 0 {
            return None;
        }
        let data = unsafe { port::inb(PS2_DATA) };
        if status & 0x20 != 0 {
            self.decode_mouse(data).map(InputEvent::Pointer)
        } else {
            self.decode_scancode(data).map(InputEvent::Key)
        }
    }

    fn decode_mouse(&mut self, byte: u8) -> Option<PointerEvent> {
        if self.mouse_index == 0 && byte & 0x08 == 0 {
            return None;
        }
        self.mouse_packet[self.mouse_index] = byte;
        self.mouse_index += 1;
        if self.mouse_index < self.mouse_packet.len() {
            return None;
        }
        self.mouse_index = 0;
        let header = self.mouse_packet[0];
        let buttons = header & 0x07;
        let previous = self.mouse_buttons;
        self.mouse_buttons = buttons;
        let overflow = header & 0xC0 != 0;
        Some(PointerEvent {
            dx: if overflow {
                0
            } else {
                self.mouse_packet[1] as i8 as i16
            },
            dy: if overflow {
                0
            } else {
                -(self.mouse_packet[2] as i8 as i16)
            },
            buttons,
            pressed: buttons & !previous,
            released: previous & !buttons,
        })
    }

    fn decode_scancode(&mut self, scancode: u8) -> Option<u8> {
        if scancode == 0xE0 {
            self.extended = true;
            return None;
        }
        if self.extended {
            self.extended = false;
            return match scancode {
                0x5B => {
                    self.super_key = true;
                    None
                }
                0xDB => {
                    self.super_key = false;
                    None
                }
                0x5C => {
                    self.super_key = true;
                    None
                }
                0xDC => {
                    self.super_key = false;
                    None
                }
                0x48 => Some(if self.super_key { KEY_SUPER_UP } else { KEY_UP }),
                0x50 => Some(if self.super_key {
                    KEY_SUPER_DOWN
                } else {
                    KEY_DOWN
                }),
                0x4B => Some(if self.super_key {
                    KEY_SUPER_LEFT
                } else {
                    KEY_LEFT
                }),
                0x4D => Some(if self.super_key {
                    KEY_SUPER_RIGHT
                } else {
                    KEY_RIGHT
                }),
                _ => None,
            };
        }
        match scancode {
            0x2A | 0x36 => {
                self.shift = true;
                return None;
            }
            0xAA | 0xB6 => {
                self.shift = false;
                return None;
            }
            0x3A => {
                self.caps_lock = !self.caps_lock;
                return None;
            }
            released if released & 0x80 != 0 => return None,
            _ => {}
        }

        let base = match scancode {
            0x01 => 0x1B,
            0x02 => b'1',
            0x03 => b'2',
            0x04 => b'3',
            0x05 => b'4',
            0x06 => b'5',
            0x07 => b'6',
            0x08 => b'7',
            0x09 => b'8',
            0x0A => b'9',
            0x0B => b'0',
            0x0C => b'-',
            0x0D => b'=',
            0x0E => 0x08,
            0x0F => b'\t',
            0x10 => b'q',
            0x11 => b'w',
            0x12 => b'e',
            0x13 => b'r',
            0x14 => b't',
            0x15 => b'y',
            0x16 => b'u',
            0x17 => b'i',
            0x18 => b'o',
            0x19 => b'p',
            0x1A => b'[',
            0x1B => b']',
            0x1C => b'\n',
            0x1E => b'a',
            0x1F => b's',
            0x20 => b'd',
            0x21 => b'f',
            0x22 => b'g',
            0x23 => b'h',
            0x24 => b'j',
            0x25 => b'k',
            0x26 => b'l',
            0x27 => b';',
            0x28 => b'\'',
            0x29 => b'`',
            0x2B => b'\\',
            0x2C => b'z',
            0x2D => b'x',
            0x2E => b'c',
            0x2F => b'v',
            0x30 => b'b',
            0x31 => b'n',
            0x32 => b'm',
            0x33 => b',',
            0x34 => b'.',
            0x35 => b'/',
            0x39 => b' ',
            _ => return None,
        };
        let key = apply_modifiers(base, self.shift, self.caps_lock);
        if self.super_key {
            super_binding(key)
        } else {
            Some(key)
        }
    }

    fn decode_serial(&mut self, byte: u8) -> Option<u8> {
        match (self.serial_escape, byte) {
            (0, 0x1B) => {
                self.serial_escape = 1;
                self.serial_escape_started = unsafe { _rdtsc() };
                None
            }
            (1, b'[') => {
                self.serial_escape = 2;
                None
            }
            (1, _) => {
                self.serial_escape = 0;
                Some(0x1B)
            }
            (2, b'A') => {
                self.serial_escape = 0;
                Some(KEY_UP)
            }
            (2, b'B') => {
                self.serial_escape = 0;
                Some(KEY_DOWN)
            }
            (2, b'C') => {
                self.serial_escape = 0;
                Some(KEY_RIGHT)
            }
            (2, b'D') => {
                self.serial_escape = 0;
                Some(KEY_LEFT)
            }
            (2, _) => {
                self.serial_escape = 0;
                None
            }
            (_, b'\r') => Some(b'\n'),
            (_, 0x7F) => Some(0x08),
            (_, value) => Some(value),
        }
    }
}

fn controller_write_command(command: u8) -> bool {
    if !controller_wait_write() {
        return false;
    }
    unsafe { port::outb(PS2_STATUS, command) };
    true
}

fn controller_write_data(value: u8) -> bool {
    if !controller_wait_write() {
        return false;
    }
    unsafe { port::outb(PS2_DATA, value) };
    true
}

fn controller_wait_write() -> bool {
    for _ in 0..100_000 {
        if unsafe { port::inb(PS2_STATUS) } & 0x02 == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn controller_read(auxiliary: bool) -> Option<u8> {
    for _ in 0..100_000 {
        let status = unsafe { port::inb(PS2_STATUS) };
        if status & 0x01 != 0 {
            let value = unsafe { port::inb(PS2_DATA) };
            if !auxiliary || status & 0x20 != 0 {
                return Some(value);
            }
        }
        core::hint::spin_loop();
    }
    None
}

fn mouse_command(command: u8) -> bool {
    controller_write_command(0xD4)
        && controller_write_data(command)
        && controller_read(true) == Some(0xFA)
}

fn super_binding(key: u8) -> Option<u8> {
    match key.to_ascii_lowercase() {
        b' ' => Some(KEY_SUPER_LAUNCHER),
        b'\n' => Some(KEY_SUPER_TERMINAL),
        b'b' => Some(KEY_SUPER_BROWSER),
        b'q' => Some(KEY_SUPER_CLOSE),
        b'\t' => Some(KEY_SUPER_CYCLE),
        b'f' => Some(KEY_SUPER_FULLSCREEN),
        b'v' => Some(KEY_SUPER_FLOAT),
        b'o' => Some(KEY_SUPER_OVERVIEW),
        b'1' => Some(KEY_SUPER_WORKSPACE_1),
        b'2' => Some(KEY_SUPER_WORKSPACE_2),
        b'3' => Some(KEY_SUPER_WORKSPACE_3),
        b'4' => Some(KEY_SUPER_WORKSPACE_4),
        _ => None,
    }
}

fn apply_modifiers(byte: u8, shift: bool, caps_lock: bool) -> u8 {
    if byte.is_ascii_lowercase() {
        return if shift ^ caps_lock {
            byte.to_ascii_uppercase()
        } else {
            byte
        };
    }
    if !shift {
        return byte;
    }
    match byte {
        b'1' => b'!',
        b'2' => b'@',
        b'3' => b'#',
        b'4' => b'$',
        b'5' => b'%',
        b'6' => b'^',
        b'7' => b'&',
        b'8' => b'*',
        b'9' => b'(',
        b'0' => b')',
        b'-' => b'_',
        b'=' => b'+',
        b'[' => b'{',
        b']' => b'}',
        b';' => b':',
        b'\'' => b'"',
        b'`' => b'~',
        b'\\' => b'|',
        b',' => b'<',
        b'.' => b'>',
        b'/' => b'?',
        other => other,
    }
}
