//! ExpOS bootstrap kernel.
//!
//! Entered through native UEFI or the legacy Multiboot2 compatibility path.
//! `boot/boot.asm` installs kernel mappings and calls `kernel_main`.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(feature = "genesis-installer", allow(dead_code))]

extern crate alloc;

mod allocator;
mod apps;
pub(crate) mod boot;
mod compat;
mod crypto;
mod desktop;
pub mod display_timing;
mod expfs_store;
mod framebuffer;
mod games;
mod genesis;
mod hardware;
mod input;
mod kernel_controls;
mod network;
mod port;
mod python;
mod radio;
mod serial;
mod session;
mod shell;
pub(crate) mod state;
mod storage;
mod sync;
mod tls;
mod vga;
mod volatile;

use core::fmt;
#[cfg(not(test))]
use core::panic::PanicInfo;

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

#[cfg(not(test))]
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
    allocator::initialize();

    slog!("[ExpOS] kernel entered; serial online\r\n");
    vga::WRITER.lock().clear();

    println!("============================================");
    println!(
        " ExpOS v{} - Form-native bootstrap",
        env!("CARGO_PKG_VERSION")
    );
    println!("============================================");

    unsafe { boot::initialize(magic, mbi_phys) };
    println!("[ok] boot handoff @ {:#x}", mbi_phys);

    let mut writer = vga::WRITER.lock();
    writer.set_color(vga::Color::Black, vga::Color::LightGreen);
    let _ = fmt::Write::write_fmt(&mut *writer, format_args!(" [ok] x86_64 "));
    writer.set_color(vga::Color::Yellow, vga::Color::Blue);
    let _ = fmt::Write::write_fmt(&mut *writer, format_args!(" [ok] VGA + serial "));
    writer.set_color(vga::Color::White, vga::Color::Black);
    drop(writer);
    println!();

    println!();

    #[cfg(feature = "genesis-installer")]
    genesis::run();

    #[cfg(not(feature = "genesis-installer"))]
    {
        let genesis_config = genesis::load();
        let report = match genesis_config {
            Some(config) => expos_core::bootstrap_with_identity(
                config.cfc_fin,
                config.cfc_name,
                config.primary_fin,
                config.primary_name,
            ),
            None => expos_core::bootstrap_demo(),
        };
        let report = match report {
            Ok(report) => {
                println!("[ok] Root Form FIN {}", report.root_fin);
                println!("[ok] Stable Dimension FIN {}", report.stable_fin);
                println!("[ok] PIMP accepted; DIESE resolved policy");
                println!("[ok] scoped Form Handle #{}", report.handle_id);
                println!("[ok] ExpFS journal commit #{}", report.journal_sequence);
                slog!("EXPOS_BOOT_OK form-native bootstrap complete\r\n");
                report
            }
            Err(error) => panic!("Form-native bootstrap failed: {:?}", error),
        };
        expfs_store::initialize(report.cfc_fin, report.cfc_name, report.stable_fin);
        state::initialize();
        let preferences = state::preferences();
        if let Some(mode) = framebuffer::DisplayMode::from_persisted(preferences.display_mode) {
            let _ = framebuffer::request_mode(mode);
        }
        session::initialize_with_seed(genesis_config.map(|config| config.operator.0));
        println!();
        println!("Core architecture online. Starting session manager.");
        let mut input = input::Input::new();
        let requested_mode = session::choose_boot_mode(&mut input);
        boot::set_single_user(requested_mode == session::BootMode::SingleUser);
        if boot::single_user() {
            slog!("EXPOS_SERVICE_MODE single-user network=disabled desktop=disabled operator-only=true\r\n");
            radio::initialize(false, false);
        } else {
            let ethernet_ready = network::initialize();
            radio::initialize(ethernet_ready, network::link_up());
            radio::restore_persisted_policy(
                preferences.flags & state::PREF_NETWORK_ENABLED != 0,
                preferences.flags & state::PREF_WIFI_ENABLED != 0,
                preferences.flags & state::PREF_BLUETOOTH_ENABLED != 0,
            );
            slog!("EXPOS_SERVICE_MODE multi-user\r\n");
        }
        let login = session::login(&mut input, requested_mode);
        if login.mode == session::BootMode::Graphical {
            desktop::run(&mut input, false, login.session, report.cfc_fin);
        }
        shell::run(report, input, login.session)
    }
}
