//! HexaOS bootstrap kernel.
//!
//! Booted by GRUB via Multiboot2. `boot/boot.asm` enters long mode and
//! calls `kernel_main(magic, mbi_phys)`.

#![no_std]
#![no_main]

mod compat;
mod hardware;
mod input;
mod port;
mod serial;
mod shell;
mod sync;
mod vga;
mod volatile;

use core::fmt;
use core::panic::PanicInfo;

const MULTIBOOT2_BOOTLOADER_MAGIC: u32 = 0x36D76289;

// ---------------------------------------------------------------------------
// Printing macros
// ---------------------------------------------------------------------------

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {
        $crate::print!("{}\n", format_args!($($arg)*))
    };
}

/// Log to the serial console only (COM1). Use `\r\n` line endings.
#[macro_export]
macro_rules! slog {
    ($($arg:tt)*) => {
        $crate::_slog(format_args!($($arg)*))
    };
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    let mut writer = vga::WRITER.lock();
    let _ = fmt::Write::write_fmt(&mut *writer, args);
    writer.update_cursor();
    drop(writer);
    let mut serial = serial::COM1.lock();
    let _ = fmt::Write::write_fmt(&mut *serial, args);
}

pub fn clear_console() {
    vga::WRITER.lock().clear();
    serial::COM1.lock().write_str("\x1B[2J\x1B[H");
}

#[doc(hidden)]
pub fn _slog(args: fmt::Arguments<'_>) {
    let mut serial = serial::COM1.lock();
    let _ = fmt::Write::write_fmt(&mut *serial, args);
}

// ---------------------------------------------------------------------------
// Panic handling
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Serial first: it works even if VGA is broken or locked.
    serial::COM1.lock().write_str("[KERNEL PANIC] ");
    if let Some(loc) = info.location() {
        let msg = core::format_args!("{}:{}:{}\n", loc.file(), loc.line(), loc.column());
        serial::COM1.lock().write_fmt_args(msg);
    }
    let msg = core::format_args!("{}\n", info.message());
    serial::COM1.lock().write_fmt_args(msg);

    let mut writer = vga::WRITER.lock();
    writer.set_color(vga::Color::White, vga::Color::Red);
    let _ = fmt::Write::write_fmt(&mut *writer, format_args!("KERNEL PANIC\n{}", info));

    loop {
        port::halt();
    }
}

// ---------------------------------------------------------------------------
// Kernel entry point (called from boot/boot.asm in ring-0 long mode)
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn kernel_main(magic: u32, mbi_phys: u64) -> ! {
    serial::COM1.lock().init();

    slog!("[HexaOS] kernel entered; serial online\r\n");
    vga::WRITER.lock().clear();

    println!("============================================");
    println!(
        " HexaOS v{} - Form-native bootstrap",
        env!("CARGO_PKG_VERSION")
    );
    println!("============================================");

    if magic != MULTIBOOT2_BOOTLOADER_MAGIC {
        panic!(
            "bad Multiboot2 magic: expected {:#x}, got {:#x}",
            MULTIBOOT2_BOOTLOADER_MAGIC, magic
        );
    }
    println!("[ok] Multiboot2 magic {:#010x}", magic);
    println!("[ok] boot info structure @ {:#x}", mbi_phys);

    let mut writer = vga::WRITER.lock();
    writer.set_color(vga::Color::Black, vga::Color::LightGreen);
    let _ = fmt::Write::write_fmt(&mut *writer, format_args!(" [ok] x86_64 "));
    writer.set_color(vga::Color::Yellow, vga::Color::Blue);
    let _ = fmt::Write::write_fmt(&mut *writer, format_args!(" [ok] VGA + serial "));
    writer.set_color(vga::Color::LightGray, vga::Color::Black);
    drop(writer);
    println!();

    println!();

    let report = match hexa_core::bootstrap_demo() {
        Ok(report) => {
            println!("[ok] Root Form FIN {}", report.root_fin);
            println!("[ok] Stable Dimension FIN {}", report.stable_fin);
            println!("[ok] PIMP accepted; DIESE resolved policy");
            println!("[ok] scoped Form Handle #{}", report.handle_id);
            println!("[ok] HexaFS journal commit #{}", report.journal_sequence);
            slog!("HEXA_BOOT_OK form-native bootstrap complete\r\n");
            report
        }
        Err(error) => panic!("Form-native bootstrap failed: {:?}", error),
    };
    println!();
    println!("Core architecture online. Starting command environment.");
    shell::run(report)
}
