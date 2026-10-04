//! APIC and PCI message-signalled interrupt foundations.
//!
//! Device drivers still poll in this milestone, but MSI/MSI-X discovery,
//! bounded vector allocation and auditable message construction are centralized
//! here.  No driver is allowed to rewrite PCI capability lists itself.

use crate::pci;
use core::arch::x86_64::__cpuid;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const IA32_APIC_BASE: u32 = 0x1B;
const FIRST_DEVICE_VECTOR: u8 = 0x40;
const LAST_DEVICE_VECTOR: u8 = 0x7F;
const MSI_CAPABILITY: u8 = 0x05;
const MSIX_CAPABILITY: u8 = 0x11;
static NEXT_VECTOR: AtomicU8 = AtomicU8::new(FIRST_DEVICE_VECTOR);
static APIC_AVAILABLE: AtomicBool = AtomicBool::new(false);
static APIC_BASE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageKind {
    Msi,
    MsiX,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Binding {
    pub vector: u8,
    pub kind: MessageKind,
    pub destination_apic: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Platform {
    pub local_apic: bool,
    pub x2apic: bool,
    pub base: u64,
    pub bootstrap_id: u8,
}

pub fn initialize() -> Platform {
    let leaf = __cpuid(1);
    let local_apic = leaf.edx & (1 << 9) != 0;
    let x2apic = leaf.ecx & (1 << 21) != 0;
    let bootstrap_id = (leaf.ebx >> 24) as u8;
    let base = if local_apic {
        read_msr(IA32_APIC_BASE) & 0xFFFF_F000
    } else {
        0
    };
    APIC_AVAILABLE.store(local_apic, Ordering::Release);
    APIC_BASE.store(base, Ordering::Release);
    crate::slog!(
        "EXPOS_INTERRUPT_PLATFORM apic={} x2apic={} bsp={} base={:#x} pic_fallback=true msi_phase=prepared\r\n",
        local_apic,
        x2apic,
        bootstrap_id,
        base
    );
    Platform {
        local_apic,
        x2apic,
        base,
        bootstrap_id,
    }
}

pub fn initialize_local_timer(slot: usize) {
    let base = APIC_BASE.load(Ordering::Acquire) as usize;
    if base == 0 || slot == 0 {
        return;
    }
    unsafe {
        let svr = core::ptr::read_volatile((base + 0xF0) as *const u32);
        core::ptr::write_volatile((base + 0xF0) as *mut u32, svr | 0x100 | 0xFF);
        core::ptr::write_volatile((base + 0x3E0) as *mut u32, 0x3);
        core::ptr::write_volatile((base + 0x320) as *mut u32, (1 << 17) | 32);
        core::ptr::write_volatile((base + 0x380) as *mut u32, 10_000_000);
    }
}

pub fn timer_eoi(slot: usize) {
    if slot == 0 {
        unsafe { crate::port::outb(0x20, 0x20) };
        return;
    }
    let base = APIC_BASE.load(Ordering::Acquire) as usize;
    if base != 0 {
        unsafe { core::ptr::write_volatile((base + 0xB0) as *mut u32, 0) };
    }
}

pub fn supports(function: pci::Function) -> (bool, bool) {
    (
        function.capability(MSI_CAPABILITY).is_some(),
        function.capability(MSIX_CAPABILITY).is_some(),
    )
}

/// Reserve a vector and describe the best message mechanism.  Arming remains
/// separate so a driver cannot make a device interrupt before an IDT handler
/// and ASL ownership record exist.
pub fn prepare(function: pci::Function) -> Option<Binding> {
    if !APIC_AVAILABLE.load(Ordering::Acquire) {
        return None;
    }
    let kind = if function.capability(MSIX_CAPABILITY).is_some() {
        MessageKind::MsiX
    } else if function.capability(MSI_CAPABILITY).is_some() {
        MessageKind::Msi
    } else {
        return None;
    };
    let vector = NEXT_VECTOR
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current <= LAST_DEVICE_VECTOR).then_some(current.saturating_add(1))
        })
        .ok()?;
    Some(Binding {
        vector,
        kind,
        destination_apic: (__cpuid(1).ebx >> 24) as u8,
    })
}

/// Program a prepared MSI or MSI-X entry. Callers must install their IDT gate
/// before invoking this function. It is intentionally unused by polling
/// drivers until the next interrupt-delivery phase is complete.
#[allow(dead_code)]
pub fn arm(function: pci::Function, binding: Binding) -> bool {
    match binding.kind {
        MessageKind::Msi => arm_msi(function, binding),
        MessageKind::MsiX => arm_msix(function, binding),
    }
}

fn arm_msi(function: pci::Function, binding: Binding) -> bool {
    let Some(capability) = function.capability(MSI_CAPABILITY) else {
        return false;
    };
    let header = function.read(capability.offset);
    let mut control = (header >> 16) as u16;
    let address = message_address(binding.destination_apic);
    function.write(capability.offset + 4, address as u32);
    let data_offset = if control & (1 << 7) != 0 {
        function.write(capability.offset + 8, (address >> 32) as u32);
        capability.offset + 12
    } else {
        capability.offset + 8
    };
    let original = function.read(data_offset);
    function.write(
        data_offset,
        (original & 0xFFFF_0000) | binding.vector as u32,
    );
    control &= !(0b111 << 4);
    control |= 1;
    function.write(
        capability.offset,
        (header & 0x0000_FFFF) | ((control as u32) << 16),
    );
    true
}

fn arm_msix(function: pci::Function, binding: Binding) -> bool {
    let Some(capability) = function.capability(MSIX_CAPABILITY) else {
        return false;
    };
    let table = function.read(capability.offset + 4);
    let bir = (table & 7) as u8;
    let offset = (table & !7) as u64;
    let Some(bar) = function.memory_bar(bir) else {
        return false;
    };
    let Some(address) = bar.address.checked_add(offset) else {
        return false;
    };
    let Some(end) = address.checked_add(16) else {
        return false;
    };
    if address < 0x10_0000 || end > 0x1_0000_0000 {
        return false;
    }
    let entry = address as usize;
    unsafe {
        core::ptr::write_volatile((entry + 12) as *mut u32, 1);
        let message = message_address(binding.destination_apic);
        core::ptr::write_volatile(entry as *mut u32, message as u32);
        core::ptr::write_volatile((entry + 4) as *mut u32, (message >> 32) as u32);
        core::ptr::write_volatile((entry + 8) as *mut u32, binding.vector as u32);
        core::ptr::write_volatile((entry + 12) as *mut u32, 0);
    }
    let header = function.read(capability.offset);
    let mut control = (header >> 16) as u16;
    control &= !(1 << 14);
    control |= 1 << 15;
    function.write(
        capability.offset,
        (header & 0xFFFF) | ((control as u32) << 16),
    );
    true
}

const fn message_address(apic_id: u8) -> u64 {
    0xFEE0_0000 | ((apic_id as u64) << 12)
}

fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((high as u64) << 32) | low as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msi_message_targets_the_selected_apic_without_logical_delivery_bits() {
        assert_eq!(message_address(0), 0xFEE0_0000);
        assert_eq!(message_address(0xAB), 0xFEEA_B000);
    }
}
