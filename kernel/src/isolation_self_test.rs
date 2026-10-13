//! Fault injection for disposable QEMU images only; absent from normal builds.
//! Recovery matches the CPU, exact RIP, CR2 and page-fault error code. Nothing
//! here is a production mechanism for recovering a supervisor fault.

use core::{
    arch::{asm, global_asm},
    ptr,
    sync::atomic::{AtomicU64, Ordering},
};

global_asm!(
    r#"
.section .text
.global expos_test_write, expos_test_write_instruction, expos_test_write_resume
expos_test_write:
expos_test_write_instruction:
    mov byte ptr [rdi], 0xc3
expos_test_write_resume:
    ret
.global expos_test_read, expos_test_read_instruction, expos_test_read_resume
expos_test_read:
expos_test_read_instruction:
    mov al, byte ptr [rdi]
expos_test_read_resume:
    ret
.global expos_test_execute, expos_test_execute_resume
expos_test_execute:
    call rdi
    ret
expos_test_execute_resume:
    add rsp, 8
    ret
.global expos_test_text
expos_test_text:
    ret
.section .rodata
.global expos_test_rodata
expos_test_rodata:
    .byte 0xc3
.section .data
.global expos_test_data
expos_test_data:
    .byte 0xc3
"#
);

unsafe extern "C" {
    fn expos_test_write(address: u64);
    fn expos_test_read(address: u64);
    fn expos_test_execute(address: u64);
    static expos_test_write_instruction: u8;
    static expos_test_write_resume: u8;
    static expos_test_read_instruction: u8;
    static expos_test_read_resume: u8;
    static expos_test_execute_resume: u8;
    static expos_test_text: u8;
    static expos_test_rodata: u8;
    static expos_test_data: u8;
}

struct Probe {
    rip: AtomicU64,
    resume: AtomicU64,
    address: AtomicU64,
    error: AtomicU64,
    hit: AtomicU64,
}

static PROBES: [Probe; crate::smp::MAX_CPUS] = [const {
    Probe {
        rip: AtomicU64::new(0),
        resume: AtomicU64::new(0),
        address: AtomicU64::new(0),
        error: AtomicU64::new(0),
        hit: AtomicU64::new(0),
    }
}; crate::smp::MAX_CPUS];

fn run(
    slot: usize,
    name: &str,
    target: u64,
    rip: u64,
    resume: u64,
    error: u64,
    operation: unsafe extern "C" fn(u64),
) {
    let probe = &PROBES[slot];
    probe.address.store(target, Ordering::SeqCst);
    probe.resume.store(resume, Ordering::SeqCst);
    probe.error.store(error, Ordering::SeqCst);
    probe.hit.store(0, Ordering::SeqCst);
    probe.rip.store(rip, Ordering::SeqCst);
    unsafe {
        operation(target);
    }
    probe.rip.store(0, Ordering::SeqCst);
    assert_eq!(
        probe.hit.load(Ordering::SeqCst),
        1,
        "protection probe did not fault: {name}"
    );
    crate::slog!(
        "EXPOS_ISOLATION_PROBE name={} cpu={} blocked=true\r\n",
        name,
        slot
    );
}

pub fn recover(slot: usize, vector: u64, error: u64, address: u64, rip: &mut u64) -> bool {
    let Some(probe) = PROBES.get(slot) else {
        return false;
    };
    if vector != 14
        || *rip == 0
        || *rip != probe.rip.load(Ordering::SeqCst)
        || address != probe.address.load(Ordering::SeqCst)
        || error != probe.error.load(Ordering::SeqCst)
    {
        return false;
    }
    *rip = probe.resume.load(Ordering::SeqCst);
    probe.rip.store(0, Ordering::SeqCst);
    probe.hit.store(1, Ordering::SeqCst);
    true
}

pub fn kernel_permissions() {
    for (name, address) in [
        ("text-write", ptr::addr_of!(expos_test_text) as u64),
        ("rodata-write", ptr::addr_of!(expos_test_rodata) as u64),
    ] {
        run(
            0,
            name,
            address,
            ptr::addr_of!(expos_test_write_instruction) as u64,
            ptr::addr_of!(expos_test_write_resume) as u64,
            3,
            expos_test_write,
        );
    }
    if crate::security::nx_active() {
        for (name, address) in [
            ("rodata-execute", ptr::addr_of!(expos_test_rodata) as u64),
            ("data-execute", ptr::addr_of!(expos_test_data) as u64),
        ] {
            run(
                0,
                name,
                address,
                address,
                ptr::addr_of!(expos_test_execute_resume) as u64,
                17,
                expos_test_execute,
            );
        }
    }
}

pub fn smap(slot: usize) {
    let flags: u64;
    unsafe {
        asm!("pushfq; pop {}", out(reg) flags);
    }
    assert_eq!(flags & (1 << 18), 0, "user AC leaked into the kernel");
    if crate::security::smap_active() {
        run(
            slot,
            "smap-user-read",
            crate::form_runtime::USER_DATA,
            ptr::addr_of!(expos_test_read_instruction) as u64,
            ptr::addr_of!(expos_test_read_resume) as u64,
            1,
            expos_test_read,
        );
    }
}

pub fn fatal_kernel_fault() {
    // No probe is armed: this must halt as a kernel fault even with a Form
    // active. The QEMU runner observes the fault marker and kills the VM.
    unsafe {
        expos_test_read(0);
    }
    panic!("null supervisor read unexpectedly succeeded");
}

/// Real worker faults and watchdog stops must leave the next VM usable.
pub fn services(cfc: expos_core::CfcFin) {
    use crate::service::{self, Error};
    let mut write_input = [0_u8; 13];
    write_input[..2].copy_from_slice(&[0x48, 0xb8]);
    write_input[2..10]
        .copy_from_slice(&(service::BASE + service::INPUT_OFFSET as u64).to_le_bytes());
    write_input[10..].copy_from_slice(&[0xc6, 0x00, 0x01]);
    for (name, code, expected) in [
        ("opcode", &[0x0f, 0x0b][..], Error::Fault(6)),
        ("input-write", &write_input[..], Error::Fault(14)),
        ("watchdog", &[0xeb, 0xfe][..], Error::Budget),
    ] {
        let mut elf = [0_u8; 256];
        elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        elf[16..20].copy_from_slice(&[2, 0, 62, 0]);
        elf[24..32].copy_from_slice(&service::BASE.to_le_bytes());
        elf[32..40].copy_from_slice(&64_u64.to_le_bytes());
        elf[54] = 56;
        elf[56] = 1;
        elf[64] = 1;
        elf[68] = 5;
        elf[72] = 120;
        elf[80..88].copy_from_slice(&service::BASE.to_le_bytes());
        elf[96..104].copy_from_slice(&(code.len() as u64).to_le_bytes());
        elf[104..112].copy_from_slice(&(code.len() as u64).to_le_bytes());
        elf[120..120 + code.len()].copy_from_slice(code);
        let result = service::run(
            &elf,
            b"immutable",
            cfc,
            expos_core::Fin::from_u128(0x53525654455354),
            8,
        );
        assert_eq!(result.err(), Some(expected));
        let survivor =
            crate::python::run("print('worker recovered')", cfc).expect("worker recovery");
        assert_eq!(survivor.result, 0);
        assert_eq!(
            &survivor.output.bytes[..survivor.output.length],
            b"worker recovered\n"
        );
        crate::slog!(
            "EXPOS_SERVICE_PROBE name={} contained=true recovered=true\r\n",
            name
        );
    }
}
