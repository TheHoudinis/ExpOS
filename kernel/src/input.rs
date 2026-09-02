use crate::{port, serial};

const PS2_STATUS: u16 = 0x64;
const PS2_DATA: u16 = 0x60;

pub struct Input {
    shift: bool,
    caps_lock: bool,
    extended: bool,
}

impl Input {
    pub const fn new() -> Self {
        Self {
            shift: false,
            caps_lock: false,
            extended: false,
        }
    }

    /// Poll serial first, then the PS/2 controller used by the QEMU window.
    pub fn poll(&mut self) -> Option<u8> {
        if let Some(byte) = serial::COM1.lock().try_read() {
            return normalize_serial(byte);
        }
        let status = unsafe { port::inb(PS2_STATUS) };
        if status & 0x01 == 0 {
            return None;
        }
        let scancode = unsafe { port::inb(PS2_DATA) };
        self.decode_scancode(scancode)
    }

    fn decode_scancode(&mut self, scancode: u8) -> Option<u8> {
        if scancode == 0xE0 {
            self.extended = true;
            return None;
        }
        if self.extended {
            self.extended = false;
            return None;
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
            0x0F => b' ',
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
        Some(apply_modifiers(base, self.shift, self.caps_lock))
    }
}

fn normalize_serial(byte: u8) -> Option<u8> {
    match byte {
        b'\r' => Some(b'\n'),
        0x7F => Some(0x08),
        value => Some(value),
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
