//! ExpPython: bounded, embedded MicroPython, with no filesystem/network imports.
#[cfg(not(test))]
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(test))]
static ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(not(test))]
extern "C" {
    fn expos_python_run(source: *const u8, length: usize) -> i32;
}

#[cfg(not(test))]
pub fn execute(source: &str) -> bool {
    if !crate::security::python_allowed() {
        crate::println!("ExpPython is disabled by the immutable Genesis security policy.");
        crate::slog!("EXPOS_PYTHON_DENIED policy=genesis\r\n");
        return false;
    }
    if source.len() > 4096 {
        crate::println!("ExpPython: source exceeds the 4096-byte boundary.");
        return false;
    }
    if ACTIVE.swap(true, Ordering::AcqRel) {
        crate::println!("ExpPython is already executing.");
        return false;
    }
    // The interpreter keeps NLR exception/abort jumps entirely within C. The
    // caller retains its source throughout execution and no pointers escape.
    let result = unsafe { expos_python_run(source.as_ptr(), source.len()) };
    ACTIVE.store(false, Ordering::Release);
    match result {
        0 => crate::slog!("EXPOS_PYTHON_OK\r\n"),
        1 => crate::println!("ExpPython: execution budget exhausted; script stopped."),
        2 => crate::println!("ExpPython: output budget exhausted; script stopped."),
        3 => crate::println!("ExpPython: script raised an exception."),
        _ => crate::println!("ExpPython: script exceeds 4096 bytes."),
    }
    result == 0
}

#[cfg(test)]
pub fn execute(_source: &str) -> bool {
    // Real interpreter tests are in ports/python/test_runtime.py and QEMU.
    false
}

#[cfg(not(test))]
#[no_mangle]
extern "C" fn expos_python_output(data: *const u8, length: usize) {
    // Pointer and length are supplied synchronously by the trusted C port.
    // Reject impossible callback metadata before constructing a Rust slice;
    // the C runtime independently maintains the same 16 KiB output ceiling.
    if data.is_null() || length > 16 * 1024 {
        expos_python_fatal();
    }
    let bytes = unsafe { core::slice::from_raw_parts(data, length) };
    for &byte in bytes {
        crate::print!("{}", if byte.is_ascii() { byte as char } else { '?' });
    }
}

#[cfg(not(test))]
#[no_mangle]
extern "C" fn expos_python_fatal() -> ! {
    panic!("ExpPython runtime invariant failed")
}
