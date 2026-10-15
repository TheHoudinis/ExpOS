//! Supervisor identity-map permissions, installed before any AP or Form runs.
//!
//! Keep huge pages for RAM/MMIO, splitting only text, rodata and low memory.
//! All non-code pages are NX when supported. No writable kernel code alias is
//! retained. The AP trampoline is copied while RW/NX, then sealed RX before SIPI.

use core::{arch::asm, ptr};

const PAGE: u64 = 4096;
const HUGE: u64 = 2 * 1024 * 1024;
const PRESENT: u64 = 1;
const WRITE: u64 = 2;
const LARGE: u64 = 1 << 7;
const NX: u64 = 1 << 63;
const ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;
const SPLIT_CAPACITY: usize = 64;
const DYNAMIC_SPLIT_CAPACITY: usize = 64;

#[repr(C, align(4096))]
struct Table([u64; 512]);

static mut TABLES: [Table; SPLIT_CAPACITY] = [const { Table([0; 512]) }; SPLIT_CAPACITY];
static mut DYNAMIC_TABLES: [Table; DYNAMIC_SPLIT_CAPACITY] =
    [const { Table([0; 512]) }; DYNAMIC_SPLIT_CAPACITY];
static DYNAMIC_USED: crate::sync::SpinMutex<usize> = crate::sync::SpinMutex::new(0);

unsafe extern "C" {
    static __kernel_text_start: u8;
    static __kernel_text_end: u8;
    static __kernel_rodata_start: u8;
    static __kernel_rodata_end: u8;
}

#[derive(Clone, Copy)]
struct Layout {
    text_start: u64,
    text_end: u64,
    rodata_start: u64,
    rodata_end: u64,
}

impl Layout {
    fn flags(self, address: u64, nx: u64) -> u64 {
        if address == 0 {
            0 // trap supervisor null dereferences too
        } else if (self.text_start..self.text_end).contains(&address) {
            PRESENT
        } else if (self.rodata_start..self.rodata_end).contains(&address) {
            PRESENT | nx
        } else {
            PRESENT | WRITE | nx
        }
    }

    fn split(self, base: u64) -> bool {
        base == 0
            || (base < self.text_end && base + HUGE > self.text_start)
            || (base < self.rodata_end && base + HUGE > self.rodata_start)
    }
}

pub fn initialize() {
    let layout = Layout {
        text_start: ptr::addr_of!(__kernel_text_start) as u64,
        text_end: ptr::addr_of!(__kernel_text_end) as u64,
        rodata_start: ptr::addr_of!(__kernel_rodata_start) as u64,
        rodata_end: ptr::addr_of!(__kernel_rodata_end) as u64,
    };
    assert!(layout.text_start > 0 && layout.text_start < layout.text_end);
    assert!(layout.text_end <= layout.rodata_start && layout.rodata_start < layout.rodata_end);
    assert!(layout.rodata_end < 1_u64 << 32);
    for boundary in [
        layout.text_start,
        layout.text_end,
        layout.rodata_start,
        layout.rodata_end,
    ] {
        assert_eq!(boundary % PAGE, 0);
    }
    let nx = crate::security::nx_page_flag();
    let mut used = 0;
    unsafe {
        let root = read_cr3();
        let pdpt = (ptr::read_volatile(root as *const u64) & ADDRESS) as *mut u64;
        for index in 0..2048 {
            let base = index as u64 * HUGE;
            let directory = (ptr::read_volatile(pdpt.add(index / 512)) & ADDRESS) as *mut u64;
            let entry = directory.add(index % 512);
            if layout.split(base) {
                assert!(
                    used < SPLIT_CAPACITY,
                    "kernel read-only sections exceed page-table pool"
                );
                let table = &mut *ptr::addr_of_mut!(TABLES[used]);
                for (page, value) in table.0.iter_mut().enumerate() {
                    let address = base + page as u64 * PAGE;
                    *value = address | layout.flags(address, nx);
                }
                ptr::write_volatile(entry, table as *mut Table as u64 | PRESENT | WRITE);
                used += 1;
            } else {
                ptr::write_volatile(entry, base | PRESENT | WRITE | LARGE | nx);
            }
        }
        // Bootstrap never sets PGE/global entries. Reload invalidates all old
        // huge-page translations, including the currently executing text.
        asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags));
    }
    crate::slog!(
        "EXPOS_KERNEL_MAPPINGS wx={} text=rx rodata={} data={} null=unmapped split_tables={}\r\n",
        nx == NX,
        if nx == NX { "r-nx" } else { "r" },
        if nx == NX { "rw-nx" } else { "rw" },
        used
    );
}

/// BSP only, before AP startup. The mailbox must be on a separate RW/NX page.
pub fn seal_ap_trampoline() {
    unsafe {
        let pdpt = (ptr::read_volatile(read_cr3() as *const u64) & ADDRESS) as *const u64;
        let pd = (ptr::read_volatile(pdpt) & ADDRESS) as *const u64;
        let pt = (ptr::read_volatile(pd) & ADDRESS) as *mut u64;
        ptr::write_volatile(pt.add(8), 0x8000 | PRESENT);
        asm!("invlpg [rax]", in("rax") 0x8000_u64, options(nostack, preserves_flags));
    }
}

/// Remove every writable executable alias of a Form code frame. The user
/// mapping remains read-only/executable; its supervisor identity alias becomes
/// read-only/NX until the frame is reclaimed.
pub fn seal_form_code_frame(physical: u64) -> bool {
    set_identity_frame(physical, PRESENT | crate::security::nx_page_flag())
}

/// Restore a reclaimed frame to the normal supervisor RW/NX pool.
pub fn unseal_form_frame(physical: u64) -> bool {
    set_identity_frame(physical, PRESENT | WRITE | crate::security::nx_page_flag())
}

fn set_identity_frame(physical: u64, flags: u64) -> bool {
    if physical == 0 || physical >= 1_u64 << 32 || !physical.is_multiple_of(PAGE) {
        return false;
    }
    unsafe {
        let root = read_cr3();
        let pdpt = (ptr::read_volatile(root as *const u64) & ADDRESS) as *mut u64;
        let pdpt_index = (physical / (512 * HUGE)) as usize;
        let directory_entry = ptr::read_volatile(pdpt.add(pdpt_index));
        if directory_entry & PRESENT == 0 {
            return false;
        }
        let directory = (directory_entry & ADDRESS) as *mut u64;
        let pd_index = ((physical / HUGE) % 512) as usize;
        let entry = directory.add(pd_index);
        let value = ptr::read_volatile(entry);
        let table = if value & LARGE != 0 {
            let mut used = DYNAMIC_USED.lock();
            if *used >= DYNAMIC_SPLIT_CAPACITY {
                return false;
            }
            let table = &mut *ptr::addr_of_mut!(DYNAMIC_TABLES[*used]);
            let base = value & ADDRESS;
            let inherited = value & (PRESENT | WRITE | NX);
            for (page, target) in table.0.iter_mut().enumerate() {
                *target = (base + page as u64 * PAGE) | inherited;
            }
            *used += 1;
            ptr::write_volatile(entry, table as *mut Table as u64 | PRESENT | WRITE);
            table as *mut Table as *mut u64
        } else {
            (value & ADDRESS) as *mut u64
        };
        let page_index = ((physical / PAGE) % 512) as usize;
        ptr::write_volatile(table.add(page_index), physical | flags);
        asm!("invlpg [rax]", in("rax") physical, options(nostack, preserves_flags));
    }
    true
}

fn read_cr3() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value & ADDRESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_boundaries_never_produce_writable_executable_pages() {
        let layout = Layout {
            text_start: 0x101000,
            text_end: 0x203000,
            rodata_start: 0x203000,
            rodata_end: 0x405000,
        };
        for address in (0..0x800000).step_by(PAGE as usize) {
            let flags = layout.flags(address, NX);
            assert!(flags & PRESENT == 0 || flags & WRITE == 0 || flags & NX != 0);
            if !layout.split(address / HUGE * HUGE) {
                assert_eq!(flags, PRESENT | WRITE | NX);
            }
        }
        assert_eq!(layout.flags(0, NX), 0);
        assert_eq!(layout.flags(layout.text_end - PAGE, NX), PRESENT);
        assert_eq!(layout.flags(layout.rodata_start, NX), PRESENT | NX);
        assert_eq!(layout.flags(layout.rodata_end, NX), PRESENT | WRITE | NX);
    }
}
