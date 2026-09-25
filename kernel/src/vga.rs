use crate::port;
use crate::sync::SpinMutex;
use crate::volatile::Volatile;
use core::fmt;

const COLS: usize = 80;
const ROWS: usize = 25;
const VGA_BUFFER: usize = 0xB8000;
const CRT_PORT_INDEX: u16 = 0x3D4;
const CRT_PORT_DATA: u16 = 0x3D5;

type Buffer = [[Volatile<u16>; COLS]; ROWS];

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

pub const fn make_color(fg: Color, bg: Color) -> u8 {
    (bg as u8) << 4 | fg as u8
}

pub struct Writer {
    col: usize,
    row: usize,
    color_code: u8,
}

impl Writer {
    fn buffer(&mut self) -> &mut Buffer {
        // SAFETY: the VGA text buffer is memory-mapped at 0xB8000 on all
        // PC-compatible machines; we are the sole owner via WRITER lock.
        unsafe { &mut *(VGA_BUFFER as *mut Buffer) }
    }

    fn blank(&self) -> u16 {
        ((self.color_code as u16) << 8) | b' ' as u16
    }

    pub fn clear(&mut self) {
        let b = self.blank();
        let buf = self.buffer();
        for row in buf.iter() {
            for cell in row.iter() {
                cell.write(b);
            }
        }
        self.col = 0;
        self.row = 0;
        self.update_cursor();
    }

    pub fn set_color(&mut self, fg: Color, bg: Color) {
        self.color_code = make_color(fg, bg);
    }

    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            0x08 => {
                if self.col > 0 {
                    self.col -= 1;
                    let row = self.row;
                    let col = self.col;
                    let blank = self.blank();
                    let buf = self.buffer();
                    buf[row][col].write(blank);
                }
            }
            _ => {
                if self.col >= COLS {
                    self.new_line();
                }
                let attr = (self.color_code as u16) << 8;
                let row = self.row;
                let col = self.col;
                let buf = self.buffer();
                buf[row][col].write(attr | byte as u16);
                self.col += 1;
            }
        }
        self.update_cursor();
    }

    pub fn write_string(&mut self, s: &str) {
        for &byte in s.as_bytes() {
            match byte {
                b'\n' | 0x08 | 0x20..=0x7E => self.write_byte(byte),
                _ => self.write_byte(b'?'),
            }
        }
    }

    fn new_line(&mut self) {
        if self.row + 1 >= ROWS {
            self.scroll_up();
        } else {
            self.row += 1;
        }
        self.col = 0;
    }

    #[allow(clippy::needless_range_loop)] // adjacent MMIO rows must be copied in order
    fn scroll_up(&mut self) {
        let blank = self.blank();
        let buf = self.buffer();
        for row in 1..ROWS {
            for col in 0..COLS {
                let cell = buf[row][col].read();
                buf[row - 1][col].write(cell);
            }
        }
        for col in 0..COLS {
            buf[ROWS - 1][col].write(blank);
        }
        self.row = ROWS - 1;
    }

    /// Move the hardware blinking cursor to (row, col).
    pub fn update_cursor(&mut self) {
        let pos = self.row * COLS + self.col;
        unsafe {
            port::outb(CRT_PORT_INDEX, 0x0F);
            port::outb(CRT_PORT_DATA, (pos & 0xFF) as u8);
            port::outb(CRT_PORT_INDEX, 0x0E);
            port::outb(CRT_PORT_DATA, ((pos >> 8) & 0xFF) as u8);
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

pub static WRITER: SpinMutex<Writer> = SpinMutex::new(Writer {
    col: 0,
    row: 0,
    // Bright white is the command environment's neutral foreground. Accent
    // colors are selected explicitly and always return to this baseline.
    color_code: make_color(Color::White, Color::Black),
});
