//! x86_64 application-processor startup and bounded CPU topology.

use core::{
    arch::asm,
    ptr,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::interrupts::Platform;

pub const MAX_CPUS: usize = 8;
const AP_TRAMPOLINE: usize = 0x8000;
const AP_MAILBOX_CR3: *mut u64 = 0x8F00 as *mut u64;
const AP_MAILBOX_STACK: *mut u64 = 0x8F08 as *mut u64;
const AP_MAILBOX_ENTRY: *mut u64 = 0x8F10 as *mut u64;
const AP_MAILBOX_SLOT: *mut u64 = 0x8F18 as *mut u64;
const AP_STACK_BYTES: usize = 64 * 1024;

#[repr(C, align(16))]
struct ApStack([u8; AP_STACK_BYTES]);

static mut AP_STACKS: [ApStack; MAX_CPUS - 1] =
    [const { ApStack([0; AP_STACK_BYTES]) }; MAX_CPUS - 1];
static ONLINE: AtomicU64 = AtomicU64::new(1);

unsafe extern "C" {
    static expos_ap_trampoline_start: u8;
    static expos_ap_trampoline_end: u8;
}

pub fn online_count() -> usize {
    ONLINE.load(Ordering::Acquire).count_ones() as usize
}

pub fn initialize(platform: Platform) -> usize {
    if !platform.local_apic {
        return 1;
    }
    let mut ids = [0_u8; MAX_CPUS];
    let count = discover_apic_ids(&mut ids, platform.bootstrap_id);
    if count <= 1 {
        crate::slog!(
            "EXPOS_SMP_READY discovered={} online=1 reason=no-aps\r\n",
            count
        );
        return 1;
    }
    let trampoline_size = unsafe {
        ptr::addr_of!(expos_ap_trampoline_end).offset_from(ptr::addr_of!(expos_ap_trampoline_start))
            as usize
    };
    if trampoline_size == 0 || trampoline_size > 0xF00 {
        crate::slog!(
            "EXPOS_SMP_READY discovered={} online=1 reason=trampoline-size\r\n",
            count
        );
        return 1;
    }
    unsafe {
        ptr::copy_nonoverlapping(
            ptr::addr_of!(expos_ap_trampoline_start),
            AP_TRAMPOLINE as *mut u8,
            trampoline_size,
        );
    }
    let cr3 = read_cr3();
    for (slot, apic_id) in ids[..count].iter().copied().enumerate() {
        if apic_id == platform.bootstrap_id || slot == 0 {
            continue;
        }
        let stack_index = slot.saturating_sub(1).min(MAX_CPUS - 2);
        let stack_top = unsafe {
            ptr::addr_of_mut!(AP_STACKS[stack_index].0)
                .cast::<u8>()
                .add(AP_STACK_BYTES) as u64
        };
        unsafe {
            ptr::write_volatile(AP_MAILBOX_CR3, cr3);
            ptr::write_volatile(AP_MAILBOX_STACK, stack_top);
            ptr::write_volatile(AP_MAILBOX_ENTRY, ap_entry as *const () as usize as u64);
            ptr::write_volatile(AP_MAILBOX_SLOT, slot as u64);
        }
        start_ap(platform.base, apic_id);
        let mask = 1_u64 << slot;
        let deadline = crate::hardware::timestamp().saturating_add(50_000_000);
        while ONLINE.load(Ordering::Acquire) & mask == 0 && crate::hardware::timestamp() < deadline
        {
            core::hint::spin_loop();
        }
    }
    let online = online_count();
    crate::slog!(
        "EXPOS_SMP_READY discovered={} online={} bsp={}\r\n",
        count,
        online,
        platform.bootstrap_id
    );
    online
}

extern "C" fn ap_entry(slot: u64) -> ! {
    let slot = slot as usize;
    crate::form_runtime::initialize_ap(slot);
    ONLINE.fetch_or(1_u64 << slot, Ordering::Release);
    crate::slog!(
        "EXPOS_SMP_CPU_ONLINE slot={} apic={}\r\n",
        slot,
        current_apic_id()
    );
    loop {
        crate::form_runtime::run_ap_work(slot);
        unsafe { asm!("pause", options(nomem, nostack, preserves_flags)) };
    }
}

fn discover_apic_ids(output: &mut [u8; MAX_CPUS], bsp: u8) -> usize {
    output[0] = bsp;
    let Some(rsdp) = crate::boot::acpi_rsdp() else {
        return 1;
    };
    let Some(madt) = find_madt(rsdp) else {
        return 1;
    };
    let length = unsafe { read_u32(madt + 4) as usize };
    let mut count = 1;
    let mut cursor = madt + 44;
    let end = madt.saturating_add(length);
    while cursor + 2 <= end && count < MAX_CPUS {
        let kind = unsafe { ptr::read_volatile(cursor as *const u8) };
        let entry_length = unsafe { ptr::read_volatile((cursor + 1) as *const u8) } as usize;
        if entry_length < 2 || cursor + entry_length > end {
            break;
        }
        if kind == 0 && entry_length >= 8 {
            let id = unsafe { ptr::read_volatile((cursor + 3) as *const u8) };
            let flags = unsafe { read_u32(cursor + 4) };
            if flags & 3 != 0 && id != bsp && !output[..count].contains(&id) {
                output[count] = id;
                count += 1;
            }
        }
        cursor += entry_length;
    }
    count
}

fn find_madt(rsdp: u64) -> Option<usize> {
    let rsdp = usize::try_from(rsdp).ok()?;
    if unsafe { bytes(rsdp, 8) } != b"RSD PTR " || !checksum(rsdp, 20) {
        return None;
    }
    let revision = unsafe { ptr::read_volatile((rsdp + 15) as *const u8) };
    let (root, entry_bytes) = if revision >= 2 {
        let length = unsafe { read_u32(rsdp + 20) as usize };
        if length < 36 || !checksum(rsdp, length) {
            return None;
        }
        (usize::try_from(unsafe { read_u64(rsdp + 24) }).ok()?, 8)
    } else {
        (unsafe { read_u32(rsdp + 16) as usize }, 4)
    };
    if !(0x1000..0x1_0000_0000).contains(&root) {
        return None;
    }
    let length = unsafe { read_u32(root + 4) as usize };
    if !(36..=1024 * 1024).contains(&length) || !checksum(root, length) {
        return None;
    }
    let entries = (length - 36) / entry_bytes;
    for index in 0..entries {
        let address = unsafe {
            if entry_bytes == 8 {
                read_u64(root + 36 + index * 8)
            } else {
                read_u32(root + 36 + index * 4) as u64
            }
        };
        let table = usize::try_from(address).ok()?;
        if table < 0x1000 || table + 44 >= 0x1_0000_0000 {
            continue;
        }
        if unsafe { bytes(table, 4) } == b"APIC" {
            let table_length = unsafe { read_u32(table + 4) as usize };
            if (44..=1024 * 1024).contains(&table_length) && checksum(table, table_length) {
                return Some(table);
            }
        }
    }
    None
}

fn checksum(address: usize, length: usize) -> bool {
    (0..length).fold(0_u8, |sum, offset| unsafe {
        sum.wrapping_add(ptr::read_volatile((address + offset) as *const u8))
    }) == 0
}

unsafe fn bytes(address: usize, length: usize) -> &'static [u8] {
    core::slice::from_raw_parts(address as *const u8, length)
}

unsafe fn read_u32(address: usize) -> u32 {
    ptr::read_unaligned(address as *const u32)
}

unsafe fn read_u64(address: usize) -> u64 {
    ptr::read_unaligned(address as *const u64)
}

fn start_ap(apic_base: u64, apic_id: u8) {
    let base = apic_base as usize;
    unsafe {
        lapic_write(base, 0x310, (apic_id as u32) << 24);
        lapic_write(base, 0x300, 0x0000_C500);
        ipi_delay();
        lapic_write(base, 0x310, (apic_id as u32) << 24);
        lapic_write(base, 0x300, 0x0000_8500);
        ipi_delay();
        for _ in 0..2 {
            lapic_write(base, 0x310, (apic_id as u32) << 24);
            lapic_write(base, 0x300, 0x0000_0600 | (AP_TRAMPOLINE as u32 >> 12));
            ipi_delay();
        }
    }
}

unsafe fn lapic_write(base: usize, register: usize, value: u32) {
    while ptr::read_volatile((base + 0x300) as *const u32) & (1 << 12) != 0 {
        core::hint::spin_loop();
    }
    ptr::write_volatile((base + register) as *mut u32, value);
}

unsafe fn ipi_delay() {
    for _ in 0..100_000 {
        asm!("pause", options(nomem, nostack, preserves_flags));
    }
}

fn read_cr3() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

fn current_apic_id() -> u8 {
    (core::arch::x86_64::__cpuid(1).ebx >> 24) as u8
}
