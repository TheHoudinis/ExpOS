//! Small general-purpose heap for bounded kernel services that require `alloc`.
//!
//! Most ExpOS subsystems remain fixed-capacity. The heap exists for audited
//! cryptographic dependencies (notably RSA certificate-chain validation), and
//! uses a reclaiming free-list rather than a one-way bump allocator so repeated
//! HTTPS requests do not leak memory.

#[cfg(not(test))]
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(test))]
const HEAP_BYTES: usize = 8 * 1024 * 1024;

#[cfg(not(test))]
#[repr(C, align(4096))]
struct HeapMemory([u8; HEAP_BYTES]);

#[cfg(not(test))]
#[global_allocator]
static ALLOCATOR: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();

#[cfg(not(test))]
static INITIALIZED: AtomicBool = AtomicBool::new(false);

#[cfg(not(test))]
static mut HEAP_MEMORY: HeapMemory = HeapMemory([0; HEAP_BYTES]);

#[cfg(not(test))]
pub fn initialize() {
    if INITIALIZED.swap(true, Ordering::AcqRel) {
        return;
    }
    let start = core::ptr::addr_of_mut!(HEAP_MEMORY).cast::<u8>();
    // SAFETY: `HEAP_MEMORY` is a page-aligned, static, uniquely owned region.
    // Initialization is guarded by `INITIALIZED`, and the allocator protects
    // all subsequent access with its own spin lock.
    unsafe {
        ALLOCATOR.lock().init(start, HEAP_BYTES);
    }
    crate::slog!("HEXA_HEAP_READY bytes={}\r\n", HEAP_BYTES);
}

#[cfg(test)]
pub fn initialize() {}
