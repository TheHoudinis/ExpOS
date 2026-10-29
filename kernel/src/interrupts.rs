//! APIC, PCI message-signalled and legacy INTx interrupt foundations.
//!
//! Device drivers still poll in this milestone, but MSI/MSI-X discovery,
//! bounded vector allocation and auditable message construction are centralized
//! here.  No driver is allowed to rewrite PCI capability lists itself.

use crate::pci;
use core::arch::x86_64::__cpuid;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

const IA32_APIC_BASE: u32 = 0x1B;
const FIRST_DEVICE_VECTOR: u8 = 0x40;
const LAST_DEVICE_VECTOR: u8 = 0x7F;
const MSI_CAPABILITY: u8 = 0x05;
const MSIX_CAPABILITY: u8 = 0x11;
static NEXT_VECTOR: AtomicU8 = AtomicU8::new(FIRST_DEVICE_VECTOR);
static APIC_AVAILABLE: AtomicBool = AtomicBool::new(false);
static APIC_BASE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static X2APIC_ENABLED: AtomicBool = AtomicBool::new(false);
static IOAPIC_AVAILABLE: AtomicBool = AtomicBool::new(false);
static BSP_APIC_ID: AtomicU8 = AtomicU8::new(0);
static DEVICE_MESSAGES: AtomicU64 = AtomicU64::new(0);
const IOAPIC_BASE: usize = 0xFEC0_0000;

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
    let bootstrap_id = (leaf.ebx >> 24) as u8;
    let mut apic_control = if local_apic {
        read_msr(IA32_APIC_BASE)
    } else {
        0
    };
    if local_apic && apic_control & (1 << 11) == 0 {
        apic_control |= 1 << 11;
        write_msr(IA32_APIC_BASE, apic_control);
    }
    let x2apic = apic_control & (1 << 10) != 0;
    let base = if local_apic {
        apic_control & 0xFFFF_F000
    } else {
        0
    };
    if local_apic {
        // MSI delivery requires the local APIC's software-enable bit in
        // addition to the hardware-enable bit in IA32_APIC_BASE. Firmware
        // commonly leaves the bootstrap processor's SVR disabled when it
        // boots through the legacy PIC path.
        if x2apic {
            let svr = read_msr(0x80F);
            write_msr(0x80F, svr | 0x100 | 0xFF);
        } else {
            unsafe {
                let svr = core::ptr::read_volatile((base as usize + 0xF0) as *const u32);
                core::ptr::write_volatile((base as usize + 0xF0) as *mut u32, svr | 0x100 | 0xFF);
            }
        }
    }
    APIC_AVAILABLE.store(local_apic, Ordering::Release);
    APIC_BASE.store(base, Ordering::Release);
    X2APIC_ENABLED.store(x2apic, Ordering::Release);
    BSP_APIC_ID.store(bootstrap_id, Ordering::Release);
    let ioapic = if local_apic {
        let version = unsafe { ioapic_read(1) };
        version != u32::MAX && ((version >> 16) & 0xFF) >= 15
    } else {
        false
    };
    IOAPIC_AVAILABLE.store(ioapic, Ordering::Release);
    crate::slog!(
        "EXPOS_INTERRUPT_PLATFORM apic={} x2apic={} ioapic={} bsp={} base={:#x} pic_fallback=true msi_phase=prepared\r\n",
        local_apic,
        x2apic,
        ioapic,
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

/// Admit one firmware-routed PCI INTx line through the I/O APIC when present,
/// retaining the remapped PIC as a compatibility fallback. Device
/// handlers must be lock-free because an interrupt may arrive while normal
/// kernel code owns a spin lock.
pub fn enable_legacy_irq(irq: u8) -> bool {
    if irq == 0 || irq == 2 || irq >= 16 {
        return false;
    }
    if IOAPIC_AVAILABLE.load(Ordering::Acquire) {
        let register = 0x10 + u32::from(irq) * 2;
        let destination = u32::from(BSP_APIC_ID.load(Ordering::Acquire)) << 24;
        unsafe {
            // PCI INTx is active-low and level-triggered. Program the masked
            // destination half first, then atomically expose the low half.
            ioapic_write(register, 1 << 16);
            ioapic_write(register + 1, destination);
            ioapic_write(register, u32::from(32 + irq) | (1 << 13) | (1 << 15));
        }
        return true;
    }
    unsafe {
        if irq < 8 {
            let mask = crate::port::inb(0x21) & !(1 << irq);
            crate::port::outb(0x21, mask);
        } else {
            let slave_mask = crate::port::inb(0xA1) & !(1 << (irq - 8));
            crate::port::outb(0xA1, slave_mask);
            let master_mask = crate::port::inb(0x21) & !(1 << 2);
            crate::port::outb(0x21, master_mask);
        }
    }
    true
}

pub fn handle_legacy_irq(irq: u8) {
    crate::audio::handle_legacy_interrupt(irq);
    crate::storage::handle_legacy_interrupt(irq);
    crate::network::handle_legacy_interrupt(irq);
    unsafe {
        local_apic_eoi();
        if irq >= 8 {
            crate::port::outb(0xA0, 0x20);
        }
        crate::port::outb(0x20, 0x20);
    }
}

unsafe fn ioapic_read(register: u32) -> u32 {
    unsafe {
        core::ptr::write_volatile(IOAPIC_BASE as *mut u32, register);
        core::ptr::read_volatile((IOAPIC_BASE + 0x10) as *const u32)
    }
}

unsafe fn ioapic_write(register: u32, value: u32) {
    unsafe {
        core::ptr::write_volatile(IOAPIC_BASE as *mut u32, register);
        core::ptr::write_volatile((IOAPIC_BASE + 0x10) as *mut u32, value);
    }
}

unsafe fn local_apic_eoi() {
    if X2APIC_ENABLED.load(Ordering::Acquire) {
        write_msr(0x80B, 0);
    } else {
        let base = APIC_BASE.load(Ordering::Acquire) as usize;
        if base != 0 {
            unsafe { core::ptr::write_volatile((base + 0xB0) as *mut u32, 0) };
        }
    }
}

pub fn handle_message_interrupt() {
    DEVICE_MESSAGES.fetch_add(1, Ordering::AcqRel);
    crate::audio::handle_message_interrupt();
    crate::storage::handle_message_interrupt();
    crate::network::handle_message_interrupt();
    let base = APIC_BASE.load(Ordering::Acquire) as usize;
    if base != 0 {
        unsafe { core::ptr::write_volatile((base + 0xB0) as *mut u32, 0) };
    }
}

pub fn device_message_count() -> u64 {
    DEVICE_MESSAGES.load(Ordering::Acquire)
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
    if control & (1 << 8) != 0 {
        // A per-vector-mask capable function may retain firmware mask bits.
        // The single-vector binding always starts fully unmasked.
        function.write(data_offset + 4, 0);
    }
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

fn write_msr(msr: u32, value: u64) {
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nomem, nostack, preserves_flags)
        );
    }
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
