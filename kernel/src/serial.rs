use crate::port;
use core::fmt;

const DATA: u16 = 0; // THR (write) / RBR (read)
const IER: u16 = 1; // interrupt enable / divisor high when DLAB=1
const FCR: u16 = 2; // FIFO control
const LCR: u16 = 3; // line control / divisor low when DLAB=1
const MCR: u16 = 4; // modem control
const LSR: u16 = 5; // line status

const LSR_THR_EMPTY: u8 = 0x20;

/// 16550 UART. COM1 lives at 0x3F8 on every QEMU x86_64 machine.
pub struct SerialPort {
    base: u16,
}

impl SerialPort {
    pub const fn new(base: u16) -> Self {
        SerialPort { base }
    }

    /// 38400 baud, 8N1, FIFO enabled, RTS/DSR asserted.
    pub fn init(&mut self) {
        unsafe {
            port::outb(self.base + IER, 0x00); // disable interrupts
            port::outb(self.base + LCR, 0x80); // DLAB on
            port::outb(self.base + DATA, 0x03); // divisor low: 3 -> 38400 baud
            port::outb(self.base + IER, 0x00); // divisor high
            port::outb(self.base + LCR, 0x03); // 8 bits, no parity, 1 stop
            port::outb(self.base + FCR, 0xC7); // FIFO on, clear, 14-byte threshold
            port::outb(self.base + MCR, 0x0B); // DTR | RTS | OUT2
        }
    }

    fn transmit(&self, byte: u8) {
        unsafe {
            while port::inb(self.base + LSR) & LSR_THR_EMPTY == 0 {
                core::hint::spin_loop();
            }
            port::outb(self.base + DATA, byte);
        }
    }

    pub fn write_str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            if b == b'\n' {
                self.transmit(b'\r');
            }
            self.transmit(b);
        }
    }

    pub fn write_fmt_args(&mut self, args: fmt::Arguments<'_>) {
        let _ = fmt::write(self, args);
    }
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_str(s);
        Ok(())
    }
}

pub static COM1: crate::sync::SpinMutex<SerialPort> =
    crate::sync::SpinMutex::new(SerialPort::new(0x3F8));
