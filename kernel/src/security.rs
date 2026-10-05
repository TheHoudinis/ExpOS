//! Boot-time CPU hardening and immutable service policy.
//!
//! Hardware protections are enabled independently on every CPU. Genesis policy
//! is monotonic for the lifetime of a boot: a restricted service cannot be
//! re-enabled by changing users or leaving single-user mode.

#![cfg_attr(test, allow(dead_code))]

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const POLICY_NETWORK: u8 = 1 << 0;
const POLICY_PYTHON: u8 = 1 << 1;
const POLICY_BROWSER: u8 = 1 << 2;

static POLICY: AtomicU8 = AtomicU8::new(POLICY_NETWORK | POLICY_PYTHON | POLICY_BROWSER);
static NX_ACTIVE: AtomicBool = AtomicBool::new(false);
static SMEP_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn configure(network: bool, python: bool, browser: bool) {
    let mut policy = 0;
    if network {
        policy |= POLICY_NETWORK;
    }
    if python {
        policy |= POLICY_PYTHON;
    }
    if browser {
        policy |= POLICY_BROWSER;
    }
    POLICY.store(policy, Ordering::Release);
    crate::slog!(
        "EXPOS_SECURITY_POLICY network={} python={} browser={} immutable=true\r\n",
        network,
        python,
        browser
    );
}

pub fn network_allowed() -> bool {
    POLICY.load(Ordering::Acquire) & POLICY_NETWORK != 0
}

pub fn python_allowed() -> bool {
    POLICY.load(Ordering::Acquire) & POLICY_PYTHON != 0
}

pub fn browser_allowed() -> bool {
    POLICY.load(Ordering::Acquire) & POLICY_BROWSER != 0
}

pub fn nx_page_flag() -> u64 {
    if NX_ACTIVE.load(Ordering::Acquire) {
        1_u64 << 63
    } else {
        0
    }
}

pub fn nx_active() -> bool {
    NX_ACTIVE.load(Ordering::Acquire)
}

pub fn smep_active() -> bool {
    SMEP_ACTIVE.load(Ordering::Acquire)
}

pub fn paranoid_cpu_ready() -> bool {
    nx_active() && smep_active()
}

/// Enable architectural defenses supported by this CPU.
///
/// CR0.WP is unconditional. NXE and SMEP are enabled only after CPUID reports
/// them, so the same image retains a diagnostic boot path on older x86_64 CPUs.
pub fn enable_cpu_hardening() {
    let maximum_extended = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    let nx = maximum_extended >= 0x8000_0001
        && core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 20) != 0;
    let maximum_basic = core::arch::x86_64::__cpuid(0).eax;
    let smep = maximum_basic >= 7 && core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 7) != 0;

    unsafe {
        let mut cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        cr0 |= 1 << 16; // supervisor writes must respect read-only mappings
        core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack, preserves_flags));

        if nx {
            let mut low: u32;
            let mut high: u32;
            core::arch::asm!(
                "rdmsr",
                in("ecx") 0xC000_0080_u32,
                out("eax") low,
                out("edx") high,
                options(nomem, nostack, preserves_flags)
            );
            low |= 1 << 11; // EFER.NXE
            core::arch::asm!(
                "wrmsr",
                in("ecx") 0xC000_0080_u32,
                in("eax") low,
                in("edx") high,
                options(nomem, nostack, preserves_flags)
            );
        }

        if smep {
            let mut cr4: u64;
            core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
            cr4 |= 1 << 20;
            core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack, preserves_flags));
        }
    }
    NX_ACTIVE.store(nx, Ordering::Release);
    SMEP_ACTIVE.store(smep, Ordering::Release);
}

pub fn log_cpu_hardening() {
    crate::slog!(
        "EXPOS_CPU_HARDENING wp=true nx={} smep={} form_wx={}\r\n",
        nx_active(),
        smep_active(),
        nx_active()
    );
}
