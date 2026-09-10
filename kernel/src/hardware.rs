use crate::{port, println};
use core::arch::x86_64::{__cpuid, _rdtsc};

/// Allocation-free PCI identity used by early kernel services.  Keeping this
/// scanner in the hardware layer lets drivers report detected-but-unsupported
/// devices without each service inventing a second PCI walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PciFunction {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub programming_interface: u8,
}

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

pub fn print_kernel_features() {
    let basic = __cpuid(1);
    let maximum_extended = __cpuid(0x8000_0000).eax;
    let extended = if maximum_extended >= 0x8000_0001 {
        Some(__cpuid(0x8000_0001))
    } else {
        None
    };
    let cr0: u64;
    let cr4: u64;
    let efer: u64;
    unsafe {
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
        let low: u32;
        let high: u32;
        core::arch::asm!(
            "rdmsr",
            in("ecx") 0xC000_0080_u32,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
        efer = ((high as u64) << 32) | low as u64;
    }
    let extended_edx = extended.map(|leaf| leaf.edx).unwrap_or(0);
    println!("KERNEL FEATURE MATRIX");
    println!(
        "cpu: long-mode={} NX={} syscall={} SSE2={} APIC={}",
        yes_no(extended_edx & (1 << 29) != 0),
        yes_no(extended_edx & (1 << 20) != 0),
        yes_no(extended_edx & (1 << 11) != 0),
        yes_no(basic.edx & (1 << 26) != 0),
        yes_no(basic.edx & (1 << 9) != 0)
    );
    println!(
        "state: paging={} write-protect={} PAE={} global-pages={} NX-active={}",
        yes_no(cr0 & (1 << 31) != 0),
        yes_no(cr0 & (1 << 16) != 0),
        yes_no(cr4 & (1 << 5) != 0),
        yes_no(cr4 & (1 << 7) != 0),
        yes_no(efer & (1 << 11) != 0)
    );
    println!("kernel: 64-bit paging, PCI scan, RTC, COM1, PS/2 input, VBE, RTL8139, ATA PIO");
    println!("services: Forms, FIN resolution, persistent state journal, PIMP/DIESE, Handles");
    println!(
        "desktop: empty-start taskbar shell, window controls, pointer hit-testing, routed input"
    );
    println!("network: Ethernet, ARP, IPv4, ICMP, UDP, DNS, TCP, HTTP and verified TLS 1.3");
    println!("radio: capability policy plus PCI Wi-Fi/Bluetooth class discovery");
    println!(
        "pending: interrupts, general filesystem/audio, DHCP, IPv6, Wi-Fi drivers and USB/Bluetooth"
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

/// Return the first PCI function with the requested class and subclass.
///
/// The scan includes secondary buses and multifunction devices.  It only
/// reads PCI configuration space and does not claim or configure the device.
pub fn find_pci_class(class_code: u8, subclass: u8) -> Option<PciFunction> {
    find_pci(|function| function.class_code == class_code && function.subclass == subclass)
}

/// Return the first PCI function with an exact vendor/device identity.
pub fn find_pci_device(vendor_id: u16, device_id: u16) -> Option<PciFunction> {
    find_pci(|function| function.vendor_id == vendor_id && function.device_id == device_id)
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
    println!("kernel stack=512 KiB  QEMU RAM default=256 MiB");
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

fn find_pci(mut matches: impl FnMut(PciFunction) -> bool) -> Option<PciFunction> {
    for bus in 0_u16..=255 {
        for slot in 0_u8..32 {
            let Some(primary) = read_pci_function(bus as u8, slot, 0) else {
                continue;
            };
            if matches(primary) {
                return Some(primary);
            }

            // Header Type is byte 0x0E.  Bit seven advertises a
            // multifunction slot, so only then are functions 1..7 valid to
            // probe.
            let header = pci_read(bus as u8, slot, 0, 0x0C);
            if header & (1 << 23) == 0 {
                continue;
            }
            for function in 1_u8..8 {
                let Some(candidate) = read_pci_function(bus as u8, slot, function) else {
                    continue;
                };
                if matches(candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn read_pci_function(bus: u8, slot: u8, function: u8) -> Option<PciFunction> {
    let identity = pci_read(bus, slot, function, 0);
    let vendor_id = identity as u16;
    if vendor_id == 0xFFFF {
        return None;
    }
    let class = pci_read(bus, slot, function, 0x08);
    Some(PciFunction {
        bus,
        slot,
        function,
        vendor_id,
        device_id: (identity >> 16) as u16,
        class_code: (class >> 24) as u8,
        subclass: (class >> 16) as u8,
        programming_interface: (class >> 8) as u8,
    })
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
