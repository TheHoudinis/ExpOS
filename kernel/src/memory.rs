//! Physical page-frame ownership below 4 GiB.
//!
//! Firmware RAM is free only after the complete kernel image, handoff records,
//! framebuffer and page zero have been reserved. Form address spaces lease
//! frames from this allocator and map them at independent high virtual ranges.

use crate::sync::SpinMutex;

pub const PAGE_BYTES: u64 = 4096;
const MAX_PHYSICAL_BYTES: u64 = 1_u64 << 32;
const FRAME_COUNT: usize = (MAX_PHYSICAL_BYTES / PAGE_BYTES) as usize;
const BITMAP_WORDS: usize = FRAME_COUNT / 64;

unsafe extern "C" {
    static __kernel_end: u8;
}

struct Allocator {
    used: [u64; BITMAP_WORDS],
    leased: [u64; BITMAP_WORDS],
    cursor: usize,
    free: usize,
    initialized: bool,
}

impl Allocator {
    const fn new() -> Self {
        Self {
            used: [u64::MAX; BITMAP_WORDS],
            leased: [0; BITMAP_WORDS],
            cursor: 0,
            free: 0,
            initialized: false,
        }
    }

    fn mark(&mut self, frame: usize, used: bool) {
        if frame >= FRAME_COUNT {
            return;
        }
        let (word, bit) = (frame / 64, 1_u64 << (frame % 64));
        let was_used = self.used[word] & bit != 0;
        if used {
            self.used[word] |= bit;
            if !was_used {
                self.free = self.free.saturating_sub(1);
            }
        } else {
            self.used[word] &= !bit;
            if was_used {
                self.free = self.free.saturating_add(1);
            }
        }
    }

    fn mark_range(&mut self, base: u64, bytes: u64, used: bool) {
        let start = if used {
            base / PAGE_BYTES
        } else {
            base.saturating_add(PAGE_BYTES - 1) / PAGE_BYTES
        };
        let end_bytes = base.saturating_add(bytes).min(MAX_PHYSICAL_BYTES);
        let end = if used {
            end_bytes.saturating_add(PAGE_BYTES - 1) / PAGE_BYTES
        } else {
            end_bytes / PAGE_BYTES
        };
        for frame in start as usize..end as usize {
            self.mark(frame, used);
        }
    }
}

static ALLOCATOR: SpinMutex<Allocator> = SpinMutex::new(Allocator::new());

pub fn initialize() {
    let mut allocator = ALLOCATOR.lock();
    if allocator.initialized {
        return;
    }
    crate::boot::visit_usable_memory(|base, bytes| allocator.mark_range(base, bytes, false));
    let kernel_end = core::ptr::addr_of!(__kernel_end) as u64;
    allocator.mark_range(0, kernel_end, true);
    crate::boot::visit_handoff_ranges(|base, bytes| allocator.mark_range(base, bytes, true));
    allocator.cursor = (kernel_end.div_ceil(PAGE_BYTES) as usize).min(FRAME_COUNT);
    allocator.initialized = true;
    crate::slog!(
        "EXPOS_FRAME_ALLOCATOR frames={} free={} kernel_end={:#x}\r\n",
        FRAME_COUNT,
        allocator.free,
        kernel_end
    );
}

pub fn allocate_frame() -> Option<u64> {
    let mut allocator = ALLOCATOR.lock();
    if !allocator.initialized || allocator.free == 0 {
        return None;
    }
    for offset in 0..FRAME_COUNT {
        let frame = (allocator.cursor + offset) % FRAME_COUNT;
        let bit = 1_u64 << (frame % 64);
        if allocator.used[frame / 64] & bit == 0 {
            allocator.mark(frame, true);
            allocator.leased[frame / 64] |= bit;
            allocator.cursor = (frame + 1) % FRAME_COUNT;
            let physical = frame as u64 * PAGE_BYTES;
            unsafe { core::ptr::write_bytes(physical as *mut u8, 0, PAGE_BYTES as usize) };
            return Some(physical);
        }
    }
    None
}

pub fn release_frame(physical: u64) -> bool {
    if physical == 0 || !physical.is_multiple_of(PAGE_BYTES) || physical >= MAX_PHYSICAL_BYTES {
        return false;
    }
    let mut allocator = ALLOCATOR.lock();
    let frame = (physical / PAGE_BYTES) as usize;
    let bit = 1_u64 << (frame % 64);
    // A reserved firmware/kernel frame is marked used too, but it was never
    // leased by this allocator. Refuse to turn such a frame into free memory
    // even if a faulty caller supplies its physical address.
    if allocator.used[frame / 64] & bit == 0 || allocator.leased[frame / 64] & bit == 0 {
        return false;
    }
    unsafe { core::ptr::write_bytes(physical as *mut u8, 0, PAGE_BYTES as usize) };
    allocator.leased[frame / 64] &= !bit;
    allocator.mark(frame, false);
    true
}
