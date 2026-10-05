//! Native x86_64 execution boundary for executable Forms.
//!
//! A launch receives a private CR3 root. The kernel's low identity map remains
//! supervisor-only while a high virtual range contains exactly one read-only
//! code page, one ABI/data page and one stack page. CPL3 can enter the kernel
//! only through the DPL3 Form ABI gate; PIT IRQ0 is the preemption source.

use core::{
    arch::asm,
    marker::PhantomData,
    mem::size_of,
    ptr,
    sync::atomic::{AtomicU8, Ordering},
};

use expos_core::{
    AbiCall, AbiRequest, AbiResponse, AbiStatus, AddressSpace, CapabilityBroker, CfcFin, Fin,
    FormHandle, NativeCallGate, Operations, FORM_ABI_VERSION,
};

use crate::{port, println, slog};

const PAGE_BYTES: usize = 4096;
const PAGE_ENTRIES: usize = 512;
const MAX_NATIVE_SPACES: usize = 8;
const MAX_CPUS: usize = crate::smp::MAX_CPUS;
const CPU_KERNEL_STACK_BYTES: usize = 64 * 1024;
const IA32_GS_BASE: u32 = 0xC000_0101;

pub const USER_CODE: u64 = 0x0000_4000_0000_0000;
pub const USER_DATA: u64 = USER_CODE + PAGE_BYTES as u64;
pub const USER_STACK: u64 = USER_DATA + PAGE_BYTES as u64;
pub const USER_END: u64 = USER_STACK + PAGE_BYTES as u64;
const USER_STACK_TOP: u64 = USER_END - 16;
const ABI_RESPONSE_OFFSET: u64 = 80;
const ABI_PAYLOAD_OFFSET: usize = 256;
const ABI_MAGIC: u64 = 0x4558_504F_5341_4249; // "EXPOSABI"

const PAGE_PRESENT: u64 = 1 << 0;
const PAGE_WRITE: u64 = 1 << 1;
const PAGE_USER: u64 = 1 << 2;

const DISPOSITION_RESUME: u64 = 0;
const DISPOSITION_PREEMPT: u64 = 1;
const DISPOSITION_BUDGET: u64 = 2;
const DISPOSITION_EXIT: u64 = 3;
const DISPOSITION_FAULT: u64 = 4;
const TIMER_QUANTUM_TICKS: u64 = 4;

#[repr(C, align(4096))]
struct Page([u64; PAGE_ENTRIES]);

impl Page {
    const fn zeroed() -> Self {
        Self([0; PAGE_ENTRIES])
    }
}

#[repr(C, align(4096))]
struct BytePage([u8; PAGE_BYTES]);

impl BytePage {
    const fn zeroed() -> Self {
        Self([0; PAGE_BYTES])
    }
}

#[repr(C, align(4096))]
struct NativeSpace {
    pml4: Page,
    user_pdpt: Page,
    user_pd: Page,
    user_pt: Page,
    code: BytePage,
    data: BytePage,
    stack: BytePage,
    form: Fin,
    assigned: bool,
    active: bool,
}

impl NativeSpace {
    const fn empty() -> Self {
        Self {
            pml4: Page::zeroed(),
            user_pdpt: Page::zeroed(),
            user_pd: Page::zeroed(),
            user_pt: Page::zeroed(),
            code: BytePage::zeroed(),
            data: BytePage::zeroed(),
            stack: BytePage::zeroed(),
            form: Fin::ZERO,
            assigned: false,
            active: false,
        }
    }
}

static mut NATIVE_SPACES: [NativeSpace; MAX_NATIVE_SPACES] =
    [const { NativeSpace::empty() }; MAX_NATIVE_SPACES];

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl TrapFrame {
    const EMPTY: Self = Self {
        r15: 0,
        r14: 0,
        r13: 0,
        r12: 0,
        r11: 0,
        r10: 0,
        r9: 0,
        r8: 0,
        rsi: 0,
        rdi: 0,
        rbp: 0,
        rdx: 0,
        rcx: 0,
        rbx: 0,
        rax: 0,
        rip: 0,
        cs: 0,
        rflags: 0,
        rsp: 0,
        ss: 0,
    };
}

const _: () = assert!(size_of::<TrapFrame>() == 160);
const _: () = assert!(size_of::<AbiRequest>() == 72);
const _: () = assert!(size_of::<AbiResponse>() == 40);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliceReason {
    Preempted,
    BudgetExhausted,
    Exited,
    Fault(u8),
}

#[derive(Clone, Copy, Debug)]
pub struct SliceResult {
    pub reason: SliceReason,
    pub timer_ticks: u64,
    pub result: u64,
    pub frame: TrapFrame,
}

#[derive(Clone, Copy, Debug)]
pub struct PreparedForm {
    slot: usize,
    address_space: AddressSpace,
    frame: TrapFrame,
}

const JOB_IDLE: u8 = 0;
const JOB_READY: u8 = 1;
const JOB_RUNNING: u8 = 2;
const JOB_DONE: u8 = 3;
const JOB_RESERVED: u8 = 4;

struct ApJob {
    prepared: *mut PreparedForm,
    broker: *const CapabilityBroker,
    cfc: CfcFin,
    dimension: Fin,
    form: Fin,
    execute_handle: FormHandle,
    log_handle: FormHandle,
    tick_limit: u64,
    reason: u8,
    fault: u8,
    timer_ticks: u64,
    result: u64,
    frame: TrapFrame,
}

impl ApJob {
    const EMPTY: Self = Self {
        prepared: ptr::null_mut(),
        broker: ptr::null(),
        cfc: CfcFin::ZERO,
        dimension: Fin::ZERO,
        form: Fin::ZERO,
        execute_handle: empty_handle(),
        log_handle: empty_handle(),
        tick_limit: 0,
        reason: 0,
        fault: 0,
        timer_ticks: 0,
        result: 0,
        frame: TrapFrame::EMPTY,
    };
}

const fn empty_handle() -> FormHandle {
    FormHandle {
        id: 0,
        parent_id: 0,
        cfc: CfcFin::ZERO,
        requester: Fin::ZERO,
        target: Fin::ZERO,
        dimension: Fin::ZERO,
        operations: Operations::NONE,
        valid_until_tick: 0,
        revoked: true,
    }
}

static AP_JOB_STATE: [AtomicU8; MAX_CPUS] = [const { AtomicU8::new(JOB_IDLE) }; MAX_CPUS];
static mut AP_JOBS: [ApJob; MAX_CPUS] = [const { ApJob::EMPTY }; MAX_CPUS];
static NEXT_AP: AtomicU8 = AtomicU8::new(1);

#[derive(Clone, Copy)]
pub struct RunAuthorization<'a> {
    broker: &'a CapabilityBroker,
    cfc: CfcFin,
    dimension: Fin,
    form: Fin,
    execute_handle: FormHandle,
    log_handle: FormHandle,
}

/// An AP-owned Form slice. Several tickets may be alive at once as long as
/// each one owns a different PreparedForm and scheduler slot.
#[must_use = "a submitted Form slice must be joined"]
pub struct PendingSlice<'a> {
    slot: usize,
    form: Fin,
    _prepared: PhantomData<&'a mut PreparedForm>,
    _broker: PhantomData<&'a CapabilityBroker>,
}

impl<'a> RunAuthorization<'a> {
    pub const fn new(
        broker: &'a CapabilityBroker,
        cfc: CfcFin,
        dimension: Fin,
        form: Fin,
        execute_handle: FormHandle,
        log_handle: FormHandle,
    ) -> Self {
        Self {
            broker,
            cfc,
            dimension,
            form,
            execute_handle,
            log_handle,
        }
    }
}

impl PreparedForm {
    pub const fn address_space(self) -> AddressSpace {
        self.address_space
    }
}

struct ActiveExecution {
    broker: *const CapabilityBroker,
    cfc: CfcFin,
    dimension: Fin,
    form: Fin,
    handles: [FormHandle; 2],
    frame: TrapFrame,
    slice_ticks: u64,
    tick_limit: u64,
    result: u64,
    fault: u8,
}

#[repr(C, align(64))]
struct PerCpuExecution {
    saved_rsp: u64,
    saved_cr3: u64,
    saved_rbx: u64,
    saved_rbp: u64,
    saved_r12: u64,
    saved_r13: u64,
    saved_r14: u64,
    saved_r15: u64,
    active: *mut ActiveExecution,
    slot: u64,
}

impl PerCpuExecution {
    const EMPTY: Self = Self {
        saved_rsp: 0,
        saved_cr3: 0,
        saved_rbx: 0,
        saved_rbp: 0,
        saved_r12: 0,
        saved_r13: 0,
        saved_r14: 0,
        saved_r15: 0,
        active: ptr::null_mut(),
        slot: 0,
    };
}

#[repr(C, align(16))]
struct CpuKernelStack([u8; CPU_KERNEL_STACK_BYTES]);

#[repr(C, align(16))]
struct TaskState([u8; 104]);

static mut PER_CPU: [PerCpuExecution; MAX_CPUS] = [const { PerCpuExecution::EMPTY }; MAX_CPUS];
static mut CPU_STACKS: [CpuKernelStack; MAX_CPUS] =
    [const { CpuKernelStack([0; CPU_KERNEL_STACK_BYTES]) }; MAX_CPUS];
static mut CPU_TSS: [TaskState; MAX_CPUS] = [const { TaskState([0; 104]) }; MAX_CPUS];
static mut CPU_GDT: [[u64; 7]; MAX_CPUS] = [[0; 7]; MAX_CPUS];

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        attributes: 0,
        offset_middle: 0,
        offset_high: 0,
        reserved: 0,
    };

    fn interrupt(handler: unsafe extern "C" fn(), dpl: u8) -> Self {
        let address = handler as usize as u64;
        Self {
            offset_low: address as u16,
            selector: 0x08,
            ist: 0,
            attributes: 0x8E | ((dpl & 3) << 5),
            offset_middle: (address >> 16) as u16,
            offset_high: (address >> 32) as u32,
            reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::MISSING; 256];
static mut INITIALIZED: bool = false;

unsafe extern "C" {
    fn expos_arch_enter_form(cr3: u64, state: *const TrapFrame) -> u64;
    fn expos_form_timer_stub();
    fn expos_form_abi_stub();
    fn expos_form_ud_stub();
    fn expos_form_gp_stub();
    fn expos_form_pf_stub();
}

/// Install the TSS, interrupt gates, remapped PIC and a 1 kHz PIT. IRQs remain
/// disabled in the kernel and become live only in the CPL3 iret frame.
pub fn initialize() {
    unsafe {
        if INITIALIZED {
            return;
        }
        IDT[6] = IdtEntry::interrupt(expos_form_ud_stub, 0);
        IDT[13] = IdtEntry::interrupt(expos_form_gp_stub, 0);
        IDT[14] = IdtEntry::interrupt(expos_form_pf_stub, 0);
        IDT[32] = IdtEntry::interrupt(expos_form_timer_stub, 0);
        IDT[0x80] = IdtEntry::interrupt(expos_form_abi_stub, 3);
        let descriptor = DescriptorTablePointer {
            limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: ptr::addr_of!(IDT) as u64,
        };
        asm!("lidt [{}]", in(reg) &descriptor, options(readonly, nostack, preserves_flags));
        initialize_cpu_arch(0);
        initialize_pic_and_pit();
        INITIALIZED = true;
    }
    slog!("EXPOS_FORM_PLATFORM_READY cpl=3 pit_hz=1000 abi_vector=0x80\r\n");
}

/// Install the shared interrupt table on an application processor. Per-CPU
/// TSS and native execution state are populated by the SMP scheduler before a
/// Ring-3 slice is assigned; idle APs never enter user mode.
pub fn initialize_ap(slot: usize) {
    unsafe {
        initialize_cpu_arch(slot);
        let descriptor = DescriptorTablePointer {
            limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: ptr::addr_of!(IDT) as u64,
        };
        asm!("lidt [{}]", in(reg) &descriptor, options(readonly, nostack, preserves_flags));
    }
    crate::interrupts::initialize_local_timer(slot);
}

pub fn run_ap_work(slot: usize) {
    if slot == 0 || slot >= crate::smp::online_count() {
        return;
    }
    if AP_JOB_STATE[slot]
        .compare_exchange(JOB_READY, JOB_RUNNING, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let job = unsafe { &mut *ptr::addr_of_mut!(AP_JOBS[slot]) };
    let authorization = RunAuthorization::new(
        unsafe { &*job.broker },
        job.cfc,
        job.dimension,
        job.form,
        job.execute_handle,
        job.log_handle,
    );
    let result = run_slice_local(unsafe { &mut *job.prepared }, authorization, job.tick_limit);
    (job.reason, job.fault) = encode_reason(result.reason);
    job.timer_ticks = result.timer_ticks;
    job.result = result.result;
    job.frame = result.frame;
    AP_JOB_STATE[slot].store(JOB_DONE, Ordering::Release);
}

unsafe fn initialize_cpu_arch(slot: usize) {
    assert!(slot < MAX_CPUS);
    let per_cpu = ptr::addr_of_mut!(PER_CPU[slot]);
    (*per_cpu).slot = slot as u64;
    write_msr(IA32_GS_BASE, per_cpu as u64);

    let stack_top = ptr::addr_of_mut!(CPU_STACKS[slot].0)
        .cast::<u8>()
        .add(CPU_KERNEL_STACK_BYTES) as u64;
    let tss = ptr::addr_of_mut!(CPU_TSS[slot].0).cast::<u8>();
    ptr::write_unaligned(tss.add(4).cast::<u64>(), stack_top);
    ptr::write_unaligned(tss.add(102).cast::<u16>(), 104);

    let tss_base = tss as u64;
    let limit = 103_u64;
    let mut descriptor = limit & 0xFFFF;
    descriptor |= (tss_base & 0xFF_FFFF) << 16;
    descriptor |= 0x89_u64 << 40;
    descriptor |= ((tss_base >> 24) & 0xFF) << 56;
    let gdt = &mut *ptr::addr_of_mut!(CPU_GDT[slot]);
    gdt[0] = 0;
    gdt[1] = 0x0020_9A00_0000_0000;
    gdt[2] = 0x0000_9200_0000_0000;
    gdt[3] = 0x0000_F200_0000_0000;
    gdt[4] = 0x0020_FA00_0000_0000;
    gdt[5] = descriptor;
    gdt[6] = tss_base >> 32;
    let table = DescriptorTablePointer {
        limit: (size_of::<[u64; 7]>() - 1) as u16,
        base: ptr::addr_of!(CPU_GDT[slot]) as u64,
    };
    asm!("lgdt [{}]", in(reg) &table, options(readonly, nostack, preserves_flags));
    asm!(
        "mov ax, 0x10",
        "mov ds, ax",
        "mov es, ax",
        "mov ss, ax",
        out("ax") _,
        options(nostack, preserves_flags)
    );
    asm!("ltr ax", in("ax") 0x28_u16, options(nostack, preserves_flags));
}

fn current_slot() -> usize {
    let slot: u64;
    unsafe {
        asm!("mov {}, gs:[72]", out(reg) slot, options(nomem, nostack, preserves_flags));
    }
    slot as usize
}

fn active_execution() -> Option<&'static mut ActiveExecution> {
    let slot = current_slot();
    if slot >= MAX_CPUS {
        return None;
    }
    unsafe { (*ptr::addr_of_mut!(PER_CPU[slot])).active.as_mut() }
}

unsafe fn write_msr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
        options(nostack, preserves_flags)
    );
}

unsafe fn initialize_pic_and_pit() {
    // 8259 PIC remap: IRQ0..15 -> vectors 32..47. Only PIT IRQ0 is unmasked.
    port::outb(0x20, 0x11);
    io_wait();
    port::outb(0xA0, 0x11);
    io_wait();
    port::outb(0x21, 0x20);
    io_wait();
    port::outb(0xA1, 0x28);
    io_wait();
    port::outb(0x21, 0x04);
    io_wait();
    port::outb(0xA1, 0x02);
    io_wait();
    port::outb(0x21, 0x01);
    io_wait();
    port::outb(0xA1, 0x01);
    io_wait();
    port::outb(0x21, 0xFE);
    port::outb(0xA1, 0xFF);

    // PIT channel 0, square-wave mode, approximately 1000 Hz.
    let divisor = 1193_u16;
    port::outb(0x43, 0x36);
    port::outb(0x40, divisor as u8);
    port::outb(0x40, (divisor >> 8) as u8);
}

unsafe fn io_wait() {
    port::outb(0x80, 0);
}

/// Build a per-Form page-table root and compile the bounded native capsule.
pub fn prepare(
    form: Fin,
    execute_handle: FormHandle,
    log_handle: FormHandle,
    content: &[u8],
) -> Option<PreparedForm> {
    let slot_index = unsafe {
        let spaces = &mut *ptr::addr_of_mut!(NATIVE_SPACES);
        spaces
            .iter()
            .position(|space| space.assigned && space.form == form && !space.active)
            .or_else(|| spaces.iter().position(|space| !space.assigned))
            .or_else(|| spaces.iter().position(|space| !space.active))?
    };
    let space = unsafe { &mut (*ptr::addr_of_mut!(NATIVE_SPACES))[slot_index] };
    space.assigned = true;
    space.active = true;
    space.form = form;
    space.pml4.0.fill(0);
    space.user_pdpt.0.fill(0);
    space.user_pd.0.fill(0);
    space.user_pt.0.fill(0);
    space.code.0.fill(0xCC);
    space.data.0.fill(0);
    space.stack.0.fill(0);

    let kernel_cr3 = read_cr3() & !0xFFF;
    let kernel_pml4 = kernel_cr3 as *const u64;
    space.pml4.0[0] = unsafe { ptr::read_volatile(kernel_pml4) };

    let pml4_index = ((USER_CODE >> 39) & 0x1FF) as usize;
    space.pml4.0[pml4_index] = physical(&space.user_pdpt) | page_table_flags();
    space.user_pdpt.0[0] = physical(&space.user_pd) | page_table_flags();
    space.user_pd.0[0] = physical(&space.user_pt) | page_table_flags();
    // W^X: executable Form code is read-only; writable data and stack are NX.
    let no_execute = crate::security::nx_page_flag();
    space.user_pt.0[0] = physical(&space.code) | PAGE_PRESENT | PAGE_USER;
    space.user_pt.0[1] = physical(&space.data) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER | no_execute;
    space.user_pt.0[2] =
        physical(&space.stack) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER | no_execute;

    let payload = native_payload(content);
    let payload_len = payload.len().min(PAGE_BYTES - ABI_PAYLOAD_OFFSET);
    space.data.0[ABI_PAYLOAD_OFFSET..ABI_PAYLOAD_OFFSET + payload_len]
        .copy_from_slice(&payload[..payload_len]);
    let request = AbiRequest {
        version: FORM_ABI_VERSION,
        call: AbiCall::Log as u16,
        caller: form,
        handle_id: log_handle.id,
        arguments: [ABI_PAYLOAD_OFFSET as u64, payload_len as u64, 0, 0, 0, 0],
    };
    unsafe {
        ptr::write_unaligned(space.data.0.as_mut_ptr().cast::<AbiRequest>(), request);
    }

    if content.starts_with(b"native:spin") {
        // PAUSE; JMP -4. This deliberately has no cooperative exit path.
        space.code.0[..4].copy_from_slice(&[0xF3, 0x90, 0xEB, 0xFC]);
    } else if content.starts_with(b"native:escape") {
        // mov rax, moffs64(1 MiB). That address is mapped supervisor-only in
        // every Form root, so CPL3 must take #PF rather than read kernel text.
        space.code.0[..12].copy_from_slice(&[
            0x48, 0xA1, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEB, 0xFE,
        ]);
    } else {
        emit_log_and_exit(&mut space.code.0, execute_handle.id);
    }

    let root = physical(&space.pml4);
    let address_space = AddressSpace::confined(root, USER_CODE, USER_END)?;
    Some(PreparedForm {
        slot: slot_index,
        address_space,
        frame: TrapFrame {
            rip: USER_CODE,
            rflags: 0x202,
            rsp: USER_STACK_TOP,
            ..TrapFrame::default()
        },
    })
}

fn emit_log_and_exit(code: &mut [u8; PAGE_BYTES], execute_handle: u32) {
    let mut cursor = 0;
    let mut emit = |bytes: &[u8]| {
        code[cursor..cursor + bytes.len()].copy_from_slice(bytes);
        cursor += bytes.len();
    };
    emit(&[0x48, 0xBB]); // mov rbx, USER_DATA
    emit(&USER_DATA.to_le_bytes());
    emit(&[0x48, 0x89, 0xDF]); // mov rdi, rbx
    emit(&[0x48, 0x8D, 0x73, ABI_RESPONSE_OFFSET as u8]); // lea rsi,[rbx+80]
    emit(&[0x48, 0xB8]); // mov rax, ABI_MAGIC
    emit(&ABI_MAGIC.to_le_bytes());
    emit(&[0xCD, 0x80]);
    emit(&[0x66, 0xC7, 0x43, 0x02]); // mov word [rbx+2], ExecutionExit
    emit(&(AbiCall::ExecutionExit as u16).to_le_bytes());
    emit(&[0xC7, 0x43, 0x14]); // mov dword [rbx+20], execute handle
    emit(&execute_handle.to_le_bytes());
    emit(&[0x48, 0xC7, 0x43, 0x18, 0, 0, 0, 0]); // args[0] = 0
    emit(&[0x48, 0x89, 0xDF]);
    emit(&[0x48, 0x8D, 0x73, ABI_RESPONSE_OFFSET as u8]);
    emit(&[0x48, 0xB8]);
    emit(&ABI_MAGIC.to_le_bytes());
    emit(&[0xCD, 0x80, 0x0F, 0x0B]); // int 0x80; ud2 if exit returned
}

fn native_payload(content: &[u8]) -> &[u8] {
    if let Some(payload) = content.strip_prefix(b"native:log ") {
        return payload;
    }
    if content.len() >= 9 && content.starts_with(b"print(\"") && content.ends_with(b"\")") {
        return &content[7..content.len() - 2];
    }
    if content.len() >= 9 && content.starts_with(b"print('") && content.ends_with(b"')") {
        return &content[7..content.len() - 2];
    }
    if content.is_empty() {
        b"Form entered Ring 3"
    } else {
        content
    }
}

/// Run one interrupt-bounded slice. A malicious Form cannot avoid the PIT
/// callback because IF is forced in the CPL3 iret frame.
pub fn run_slice(
    prepared: &mut PreparedForm,
    authorization: RunAuthorization<'_>,
    tick_limit: u64,
) -> SliceResult {
    if current_slot() == 0 && crate::smp::online_count() > 1 {
        if let Some(pending) = submit_ap_slice(prepared, authorization, tick_limit) {
            return join_ap_slice(pending);
        }
    }
    run_slice_local(prepared, authorization, tick_limit)
}

/// Reserve an application processor and start a slice without waiting for it.
/// The caller can submit other prepared Forms before joining this ticket.
pub fn submit_ap_slice<'a>(
    prepared: &'a mut PreparedForm,
    authorization: RunAuthorization<'a>,
    tick_limit: u64,
) -> Option<PendingSlice<'a>> {
    if current_slot() != 0 {
        return None;
    }
    let online = crate::smp::online_count().min(MAX_CPUS);
    let workers = online.saturating_sub(1);
    if workers == 0 {
        return None;
    }
    let start = 1 + usize::from(NEXT_AP.fetch_add(1, Ordering::Relaxed)) % workers;
    let slot = (0..workers)
        .map(|offset| 1 + (start - 1 + offset) % workers)
        .find(|slot| {
            AP_JOB_STATE[*slot]
                .compare_exchange(JOB_IDLE, JOB_RESERVED, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        })?;
    let job = unsafe { &mut *ptr::addr_of_mut!(AP_JOBS[slot]) };
    job.prepared = prepared;
    job.broker = authorization.broker;
    job.cfc = authorization.cfc;
    job.dimension = authorization.dimension;
    job.form = authorization.form;
    job.execute_handle = authorization.execute_handle;
    job.log_handle = authorization.log_handle;
    job.tick_limit = tick_limit;
    AP_JOB_STATE[slot].store(JOB_READY, Ordering::Release);
    slog!(
        "EXPOS_SMP_FORM_DISPATCH fin={} cpu_slot={}\r\n",
        authorization.form,
        slot
    );
    Some(PendingSlice {
        slot,
        form: authorization.form,
        _prepared: PhantomData,
        _broker: PhantomData,
    })
}

/// Wait for a previously submitted AP slice and release its CPU slot.
pub fn join_ap_slice(pending: PendingSlice<'_>) -> SliceResult {
    let slot = pending.slot;
    while AP_JOB_STATE[slot].load(Ordering::Acquire) != JOB_DONE {
        core::hint::spin_loop();
    }
    let job = unsafe { &mut *ptr::addr_of_mut!(AP_JOBS[slot]) };
    let result = SliceResult {
        reason: decode_reason(job.reason, job.fault),
        timer_ticks: job.timer_ticks,
        result: job.result,
        frame: job.frame,
    };
    AP_JOB_STATE[slot].store(JOB_IDLE, Ordering::Release);
    slog!(
        "EXPOS_SMP_FORM_COMPLETE fin={} cpu_slot={} ticks={}\r\n",
        pending.form,
        slot,
        result.timer_ticks
    );
    result
}

const fn encode_reason(reason: SliceReason) -> (u8, u8) {
    match reason {
        SliceReason::Preempted => (1, 0),
        SliceReason::BudgetExhausted => (2, 0),
        SliceReason::Exited => (3, 0),
        SliceReason::Fault(vector) => (4, vector),
    }
}

const fn decode_reason(reason: u8, fault: u8) -> SliceReason {
    match reason {
        1 => SliceReason::Preempted,
        2 => SliceReason::BudgetExhausted,
        3 => SliceReason::Exited,
        _ => SliceReason::Fault(fault),
    }
}

fn run_slice_local(
    prepared: &mut PreparedForm,
    authorization: RunAuthorization<'_>,
    tick_limit: u64,
) -> SliceResult {
    let mut active = ActiveExecution {
        broker: authorization.broker,
        cfc: authorization.cfc,
        dimension: authorization.dimension,
        form: authorization.form,
        handles: [authorization.execute_handle, authorization.log_handle],
        frame: prepared.frame,
        slice_ticks: 0,
        tick_limit: tick_limit.max(1),
        result: 0,
        fault: 0,
    };
    let cpu = unsafe { &mut *ptr::addr_of_mut!(PER_CPU[current_slot()]) };
    assert!(
        cpu.active.is_null(),
        "CPU already owns a native Form context"
    );
    cpu.active = &mut active;
    let disposition =
        unsafe { expos_arch_enter_form(prepared.address_space.root, ptr::addr_of!(active.frame)) };
    cpu.active = ptr::null_mut();
    prepared.frame = active.frame;
    let reason = match disposition {
        DISPOSITION_PREEMPT => SliceReason::Preempted,
        DISPOSITION_BUDGET => SliceReason::BudgetExhausted,
        DISPOSITION_EXIT => SliceReason::Exited,
        DISPOSITION_FAULT => SliceReason::Fault(active.fault),
        _ => SliceReason::Fault(0xFF),
    };
    SliceResult {
        reason,
        timer_ticks: active.slice_ticks,
        result: active.result,
        frame: active.frame,
    }
}

pub fn finish(prepared: PreparedForm) {
    unsafe {
        (*ptr::addr_of_mut!(NATIVE_SPACES))[prepared.slot].active = false;
    }
}

#[no_mangle]
pub extern "C" fn expos_form_timer_interrupt(frame: *mut TrapFrame) -> u64 {
    let Some(active) = active_execution() else {
        return DISPOSITION_FAULT;
    };
    active.slice_ticks = active.slice_ticks.saturating_add(1);
    if active.slice_ticks >= active.tick_limit {
        active.frame = unsafe { *frame };
        return DISPOSITION_BUDGET;
    }
    if active.slice_ticks >= TIMER_QUANTUM_TICKS {
        active.frame = unsafe { *frame };
        return DISPOSITION_PREEMPT;
    }
    DISPOSITION_RESUME
}

#[no_mangle]
pub extern "C" fn expos_form_interrupt_eoi() {
    crate::interrupts::timer_eoi(current_slot());
}

#[no_mangle]
pub extern "C" fn expos_form_abi_interrupt(frame: *mut TrapFrame) -> u64 {
    let Some(active) = active_execution() else {
        return DISPOSITION_FAULT;
    };
    let frame = unsafe { &mut *frame };
    if frame.rax != ABI_MAGIC
        || frame.rdi != USER_DATA
        || frame.rsi != USER_DATA + ABI_RESPONSE_OFFSET
    {
        active.fault = 0x80;
        active.frame = *frame;
        return DISPOSITION_FAULT;
    }
    let request = unsafe { ptr::read_unaligned(USER_DATA as *const AbiRequest) };
    let Some(handle) = active
        .handles
        .iter()
        .find(|handle| handle.id == request.handle_id)
        .copied()
    else {
        write_abi_response(AbiResponse::status(AbiStatus::Denied));
        frame.rax = AbiStatus::Denied as u64;
        return DISPOSITION_RESUME;
    };
    let broker = unsafe { &*active.broker };
    let mut requested_disposition = DISPOSITION_RESUME;
    let response = NativeCallGate::new(broker, active.cfc, handle.target, active.dimension)
        .dispatch(
            request,
            crate::hardware::timestamp(),
            |call, arguments| match call {
                AbiCall::Log => service_log(arguments),
                AbiCall::HandleAuthorize => {
                    AbiResponse::ok([handle.id as u64, handle.operations.bits() as u64, 0, 0])
                }
                AbiCall::TimeNow => AbiResponse::ok([crate::hardware::timestamp(), 0, 0, 0]),
                AbiCall::ExecutionYield => {
                    requested_disposition = DISPOSITION_PREEMPT;
                    AbiResponse::ok([0; 4])
                }
                AbiCall::ExecutionExit => {
                    active.result = arguments[0];
                    requested_disposition = DISPOSITION_EXIT;
                    AbiResponse::ok([arguments[0], 0, 0, 0])
                }
                _ => AbiResponse::status(AbiStatus::Unsupported),
            },
        );
    write_abi_response(response);
    frame.rax = response.status as u64;
    if requested_disposition != DISPOSITION_RESUME {
        active.frame = *frame;
    }
    slog!(
        "EXPOS_FORM_ABI fin={} call={} handle={} status={}\r\n",
        active.form,
        request.call,
        request.handle_id,
        response.status
    );
    requested_disposition
}

fn service_log(arguments: [u64; 6]) -> AbiResponse {
    let offset = arguments[0] as usize;
    let length = arguments[1] as usize;
    let Some(end) = offset.checked_add(length) else {
        return AbiResponse::status(AbiStatus::Invalid);
    };
    if offset < ABI_PAYLOAD_OFFSET || end > PAGE_BYTES || length > 512 {
        return AbiResponse::status(AbiStatus::Invalid);
    }
    let bytes =
        unsafe { core::slice::from_raw_parts((USER_DATA as *const u8).add(offset), length) };
    let Ok(message) = core::str::from_utf8(bytes) else {
        return AbiResponse::status(AbiStatus::Invalid);
    };
    println!("{}", message);
    AbiResponse::ok([length as u64, 0, 0, 0])
}

fn write_abi_response(response: AbiResponse) {
    unsafe {
        ptr::write_unaligned(
            (USER_DATA + ABI_RESPONSE_OFFSET) as *mut AbiResponse,
            response,
        );
    }
}

#[no_mangle]
pub extern "C" fn expos_form_fault_interrupt(vector: u64) {
    let Some(active) = active_execution() else {
        loop {
            port::halt();
        }
    };
    active.fault = vector as u8;
    slog!("EXPOS_FORM_FAULT fin={} vector={}\r\n", active.form, vector);
}

fn physical<T>(value: &T) -> u64 {
    value as *const T as u64
}

const fn page_table_flags() -> u64 {
    PAGE_PRESENT | PAGE_WRITE | PAGE_USER
}

fn read_cr3() -> u64 {
    let value: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}
