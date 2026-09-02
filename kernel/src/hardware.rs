use crate::{port, println};
use core::arch::x86_64::{__cpuid, _rdtsc};

pub fn print_date() {
    let second = read_cmos(0x00);
    let minute = read_cmos(0x02);
    let hour = read_cmos(0x04);
    let day = read_cmos(0x07);
    let month = read_cmos(0x08);
    let year = read_cmos(0x09);
    let status_b = read_cmos(0x0B);
    let convert = |value| {
        if status_b & 0x04 == 0 {
            bcd(value)
        } else {
            value
        }
    };
    println!(
        "20{:02}-{:02}-{:02} {:02}:{:02}:{:02} RTC",
        convert(year),
        convert(month),
        convert(day),
        convert(hour & 0x7F),
        convert(minute),
        convert(second)
    );
}

pub fn print_cpu_info() {
    let vendor_leaf = __cpuid(0);
    let mut vendor = [0_u8; 12];
    vendor[0..4].copy_from_slice(&vendor_leaf.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&vendor_leaf.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&vendor_leaf.ecx.to_le_bytes());
    let vendor = core::str::from_utf8(&vendor).unwrap_or("unknown");
    let features = __cpuid(1);
    let family = ((features.eax >> 8) & 0xF) + ((features.eax >> 20) & 0xFF);
    let model = ((features.eax >> 4) & 0xF) | ((features.eax >> 12) & 0xF0);
    println!("CPU vendor: {}", vendor);
    println!(
        "family={} model={} stepping={}",
        family,
        model,
        features.eax & 0xF
    );
    println!(
        "features: SSE={} SSE2={} APIC={} TSC={}",
        yes_no(features.edx & (1 << 25) != 0),
        yes_no(features.edx & (1 << 26) != 0),
        yes_no(features.edx & (1 << 9) != 0),
        yes_no(features.edx & (1 << 4) != 0)
    );
}

pub fn print_pci() {
    println!("BUS SLOT FUNC VENDOR DEVICE");
    let mut found = 0;
    for bus in 0_u16..8 {
        for slot in 0_u8..32 {
            let value = pci_read(bus as u8, slot, 0, 0);
            let vendor = value as u16;
            if vendor == 0xFFFF {
                continue;
            }
            println!(
                "{:02X}  {:02X}   00   {:04X}   {:04X}",
                bus,
                slot,
                vendor,
                value >> 16
            );
            found += 1;
            if found == 16 {
                return;
            }
        }
    }
    if found == 0 {
        println!("No PCI functions detected.");
    }
}

pub fn print_memory_architecture() {
    let cr0: u64;
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    }
    println!("paging={} long-mode=yes", yes_no(cr0 & (1 << 31) != 0));
    println!("bootstrap map=1 GiB huge-pages  CR3={:#018X}", cr3);
    println!("kernel stack=64 KiB  QEMU RAM default=256 MiB");
}

pub fn random_u32() -> u32 {
    let ticks = timestamp();
    let mut value = (ticks as u32) ^ ((ticks >> 32) as u32);
    value ^= value << 13;
    value ^= value >> 17;
    value ^ (value << 5)
}

pub fn timestamp() -> u64 {
    unsafe { _rdtsc() }
}

fn read_cmos(register: u8) -> u8 {
    unsafe {
        port::outb(0x70, register | 0x80);
        port::inb(0x71)
    }
}

fn pci_read(bus: u8, slot: u8, function: u8, offset: u8) -> u32 {
    let address = 0x8000_0000
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((function as u32) << 8)
        | (offset as u32 & 0xFC);
    unsafe {
        port::outl(0xCF8, address);
        port::inl(0xCFC)
    }
}

const fn bcd(value: u8) -> u8 {
    (value & 0x0F) + ((value >> 4) * 10)
}

const fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}
