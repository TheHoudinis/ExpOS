//! Native x86_64 execution boundary for executable Forms.
//!
//! A launch receives a private CR3 root. The kernel's low identity map remains
//! supervisor-only while a high virtual range contains exactly one read-only
//! code page, one ABI/data page and one stack page. CPL3 can enter the kernel
//! only through the DPL3 Form ABI gate; PIT IRQ0 is the preemption source.

use alloc::boxed::Box;
use core::{
    arch::asm,
    marker::PhantomData,
    mem::size_of,
    ptr,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

use expos_core::{
    AbiCall, AbiRequest, AbiResponse, AbiStatus, AddressSpace, CapabilityBroker, CfcFin,
    ContextHeap, Fin, FormHandle, NativeCallGate, NativeImage, NativeImageError, Operations,
    StreamCall, StreamRequest, FORM_ABI_VERSION,
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
pub const USER_STACK: u64 = USER_DATA + 2 * PAGE_BYTES as u64;
pub const USER_END: u64 = USER_STACK + PAGE_BYTES as u64;
const USER_STACK_TOP: u64 = USER_END - 16;
const V2_CODE_PAGES: usize = 8;
const V2_DATA_INDEX: usize = 9;
const V2_DATA_PAGES: usize = 6;
const V2_HEAP_INDEX: usize = 16;
const V2_HEAP_PAGES: usize = 16;
const V2_STACK_INDEX: usize = 33;
const V2_STACK_PAGES: usize = 4;
const V2_END_INDEX: usize = 38;
const MAX_DYNAMIC_FRAMES: usize = V2_CODE_PAGES + V2_DATA_PAGES + V2_HEAP_PAGES + V2_STACK_PAGES;
const ABI_RESPONSE_OFFSET: u64 = 80;
const ABI_PAYLOAD_OFFSET: usize = 256;
const ABI_MAGIC: u64 = 0x4558_504F_5341_4249; // "EXPOSABI"
const STREAM_ABI_MAGIC: u64 = 0x4558_5053_5452_4D32; // "EXPSTRM2"

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
    dynamic_frames: [u64; MAX_DYNAMIC_FRAMES],
    dynamic_len: u8,
    dynamic_code_len: u8,
    data_physical: u64,
    data_virtual: u64,
    form: Fin,
    assigned: bool,
    active: bool,
    service: Option<Box<crate::service::Memory>>,
    output: Option<Box<crate::service::Output>>,
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
            dynamic_frames: [0; MAX_DYNAMIC_FRAMES],
            dynamic_len: 0,
            dynamic_code_len: 0,
            data_physical: 0,
            data_virtual: USER_DATA,
            form: Fin::ZERO,
            assigned: false,
            active: false,
            service: None,
            output: None,
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
    network_handle: FormHandle,
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
        network_handle: empty_handle(),
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
    network_handle: FormHandle,
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
            network_handle: empty_handle(),
        }
    }

    pub const fn with_network(mut self, handle: FormHandle) -> Self {
        self.network_handle = handle;
        self
    }
}

impl PreparedForm {
    pub const fn address_space(self) -> AddressSpace {
        self.address_space
    }

    pub fn heap(self) -> Option<ContextHeap> {
        if self.address_space.upper_bound != USER_CODE + V2_END_INDEX as u64 * PAGE_BYTES as u64 {
            return None;
        }
        let mut heap = ContextHeap::new(
            USER_CODE + V2_HEAP_INDEX as u64 * PAGE_BYTES as u64,
            V2_HEAP_PAGES as u32,
        )?;
        heap.grow(V2_HEAP_PAGES as u32).ok()?;
        Some(heap)
    }
}

struct ActiveExecution {
    service_reply: bool,
    output: *mut crate::service::Output,
    data: *mut u8,
    data_virtual: u64,
    service_memory: *mut crate::service::Memory,
    broker: *const CapabilityBroker,
    cfc: CfcFin,
    dimension: Fin,
    form: Fin,
    handles: [FormHandle; 3],
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
    fn expos_form_de_stub();
    fn expos_form_db_stub();
    fn expos_form_bp_stub();
    fn expos_form_of_stub();
    fn expos_form_br_stub();
    fn expos_form_nm_stub();
    fn expos_form_ts_stub();
    fn expos_form_np_stub();
    fn expos_form_ss_stub();
    fn expos_form_mf_stub();
    fn expos_form_ac_stub();
    fn expos_form_xm_stub();
    fn expos_legacy_irq1_stub();
    fn expos_legacy_irq2_stub();
    fn expos_legacy_irq3_stub();
    fn expos_legacy_irq4_stub();
    fn expos_legacy_irq5_stub();
    fn expos_legacy_irq6_stub();
    fn expos_legacy_irq7_stub();
    fn expos_legacy_irq8_stub();
    fn expos_legacy_irq9_stub();
    fn expos_legacy_irq10_stub();
    fn expos_legacy_irq11_stub();
    fn expos_legacy_irq12_stub();
    fn expos_legacy_irq13_stub();
    fn expos_legacy_irq14_stub();
    fn expos_legacy_irq15_stub();
    fn expos_device_irq_stub();
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
        IDT[0] = IdtEntry::interrupt(expos_form_de_stub, 0);
        IDT[1] = IdtEntry::interrupt(expos_form_db_stub, 0);
        IDT[3] = IdtEntry::interrupt(expos_form_bp_stub, 3);
        IDT[4] = IdtEntry::interrupt(expos_form_of_stub, 3);
        IDT[5] = IdtEntry::interrupt(expos_form_br_stub, 0);
        IDT[7] = IdtEntry::interrupt(expos_form_nm_stub, 0);
        IDT[10] = IdtEntry::interrupt(expos_form_ts_stub, 0);
        IDT[11] = IdtEntry::interrupt(expos_form_np_stub, 0);
        IDT[12] = IdtEntry::interrupt(expos_form_ss_stub, 0);
        IDT[16] = IdtEntry::interrupt(expos_form_mf_stub, 0);
        IDT[17] = IdtEntry::interrupt(expos_form_ac_stub, 0);
        IDT[19] = IdtEntry::interrupt(expos_form_xm_stub, 0);
        IDT[32] = IdtEntry::interrupt(expos_form_timer_stub, 0);
        IDT[33] = IdtEntry::interrupt(expos_legacy_irq1_stub, 0);
        IDT[34] = IdtEntry::interrupt(expos_legacy_irq2_stub, 0);
        IDT[35] = IdtEntry::interrupt(expos_legacy_irq3_stub, 0);
        IDT[36] = IdtEntry::interrupt(expos_legacy_irq4_stub, 0);
        IDT[37] = IdtEntry::interrupt(expos_legacy_irq5_stub, 0);
        IDT[38] = IdtEntry::interrupt(expos_legacy_irq6_stub, 0);
        IDT[39] = IdtEntry::interrupt(expos_legacy_irq7_stub, 0);
        IDT[40] = IdtEntry::interrupt(expos_legacy_irq8_stub, 0);
        IDT[41] = IdtEntry::interrupt(expos_legacy_irq9_stub, 0);
        IDT[42] = IdtEntry::interrupt(expos_legacy_irq10_stub, 0);
        IDT[43] = IdtEntry::interrupt(expos_legacy_irq11_stub, 0);
        IDT[44] = IdtEntry::interrupt(expos_legacy_irq12_stub, 0);
        IDT[45] = IdtEntry::interrupt(expos_legacy_irq13_stub, 0);
        IDT[46] = IdtEntry::interrupt(expos_legacy_irq14_stub, 0);
        IDT[47] = IdtEntry::interrupt(expos_legacy_irq15_stub, 0);
        let mut vector = 0x40;
        while vector <= 0x7F {
            IDT[vector] = IdtEntry::interrupt(expos_device_irq_stub, 0);
            vector += 1;
        }
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
    )
    .with_network(job.network_handle);
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
    write_msr(0xC000_0102, 0); // user GS base, exchanged by SWAPGS

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

    program_periodic_pit();
}

const PIT_HZ: u64 = 1_193_182;
const PERIODIC_DIVISOR: u16 = 1193;
static TICKLESS_REPORTED: AtomicBool = AtomicBool::new(false);

unsafe fn program_periodic_pit() {
    // PIT channel 0, square-wave mode, approximately 1000 Hz. This remains the
    // preemption source while a Ring 3 Form owns the CPU.
    port::outb(0x43, 0x36);
    port::outb(0x40, PERIODIC_DIVISOR as u8);
    port::outb(0x40, (PERIODIC_DIVISOR >> 8) as u8);
}

const fn one_shot_divisor(microseconds: u32) -> u16 {
    let ticks = (microseconds as u64 * PIT_HZ).div_ceil(1_000_000);
    if ticks == 0 {
        1
    } else if ticks > u16::MAX as u64 {
        u16::MAX
    } else {
        ticks as u16
    }
}

/// Enter a bounded tickless idle period on the bootstrap CPU. Device and input
/// IRQs may wake the CPU early. Ring 3 execution always restores the 1 kHz
/// preemption source before it can run, so this optimization cannot weaken the
/// ExpBudget boundary.
pub fn wait_tickless(max_microseconds: u32) {
    let divisor = one_shot_divisor(max_microseconds);
    unsafe {
        // Channel 0, access low/high byte, interrupt-on-terminal-count mode.
        port::outb(0x43, 0x30);
        port::outb(0x40, divisor as u8);
        port::outb(0x40, (divisor >> 8) as u8);
    }
    if !TICKLESS_REPORTED.swap(true, Ordering::AcqRel) {
        crate::slog!(
            "EXPOS_TICKLESS_IDLE mode=pit-one-shot maximum_us={} divisor={} ring3_preemption_hz=1000\r\n",
            max_microseconds,
            divisor
        );
    }
    crate::port::wait_for_interrupt();
    unsafe { program_periodic_pit() };
}

unsafe fn io_wait() {
    port::outb(0x80, 0);
}

#[derive(Debug)]
pub enum PrepareError {
    InvalidImage(NativeImageError),
    NxRequired,
    NoSlot,
    OutOfMemory,
    WxSeal,
}

impl PrepareError {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidImage(error) => match error {
                NativeImageError::Header => "invalid executable header",
                NativeImageError::Unsupported => "unsupported executable format",
                NativeImageError::Length => "invalid executable segment lengths",
                NativeImageError::Entry => "entry point is outside executable code",
            },
            Self::NxRequired => "native images require CPU NX protection",
            Self::NoSlot => "no isolated native address-space slot is available",
            Self::OutOfMemory => "physical page-frame quota is exhausted",
            Self::WxSeal => "executable frame could not be physically sealed",
        }
    }
}

/// Validate a native image before reserving a slot, or compile a legacy capsule.
pub fn prepare(
    form: Fin,
    execute_handle: FormHandle,
    log_handle: FormHandle,
    content: &[u8],
) -> Result<PreparedForm, PrepareError> {
    let image = if content.starts_with(b"EXF")
        || content
            .iter()
            .any(|b| !b.is_ascii_graphic() && !b.is_ascii_whitespace())
    {
        let image = NativeImage::parse(content).map_err(PrepareError::InvalidImage)?;
        if !crate::security::nx_active() {
            return Err(PrepareError::NxRequired);
        }
        Some(image)
    } else {
        None
    };
    let slot_index = unsafe {
        let spaces = &mut *ptr::addr_of_mut!(NATIVE_SPACES);
        spaces
            .iter()
            .position(|space| space.assigned && space.form == form && !space.active)
            .or_else(|| spaces.iter().position(|space| !space.assigned))
            .or_else(|| spaces.iter().position(|space| !space.active))
            .ok_or(PrepareError::NoSlot)?
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
    space.dynamic_frames.fill(0);
    space.dynamic_len = 0;
    space.dynamic_code_len = 0;
    space.data_physical = physical(&space.data);
    space.data_virtual = USER_DATA;

    let kernel_cr3 = read_cr3() & !0xFFF;
    let kernel_pml4 = kernel_cr3 as *const u64;
    space.pml4.0[0] = unsafe { ptr::read_volatile(kernel_pml4) };

    let pml4_index = ((USER_CODE >> 39) & 0x1FF) as usize;
    space.pml4.0[pml4_index] = physical(&space.user_pdpt) | page_table_flags();
    space.user_pdpt.0[0] = physical(&space.user_pd) | page_table_flags();
    space.user_pd.0[0] = physical(&space.user_pt) | page_table_flags();
    // EXF1 format v1 retains its historical addresses. Format v2 gets
    // independently allocated multi-page code/data/stack ranges with explicit
    // guard holes and a private per-context heap window.
    let no_execute = crate::security::nx_page_flag();
    let v2 = image.is_some_and(|image| image.format_version == 2);
    let (data_virtual, stack_top, address_end) = if let Some(image) =
        image.filter(|image| image.format_version == 2)
    {
        let code_pages = image.code.len().div_ceil(PAGE_BYTES);
        let data_bytes = ABI_PAYLOAD_OFFSET + image.data.len() + image.zero_bytes;
        let data_pages = data_bytes.div_ceil(PAGE_BYTES);
        let total = code_pages + data_pages + V2_HEAP_PAGES + V2_STACK_PAGES;
        if code_pages > V2_CODE_PAGES || data_pages > V2_DATA_PAGES || total > MAX_DYNAMIC_FRAMES {
            return reject_preparation(space, PrepareError::InvalidImage(NativeImageError::Length));
        }
        for index in 0..total {
            let Some(frame) = crate::memory::allocate_frame() else {
                return reject_preparation(space, PrepareError::OutOfMemory);
            };
            space.dynamic_frames[index] = frame;
            space.dynamic_len += 1;
        }
        space.dynamic_code_len = code_pages as u8;
        for page in 0..code_pages {
            let frame = space.dynamic_frames[page];
            let start = page * PAGE_BYTES;
            let end = (start + PAGE_BYTES).min(image.code.len());
            unsafe {
                core::slice::from_raw_parts_mut(frame as *mut u8, PAGE_BYTES)[..end - start]
                    .copy_from_slice(&image.code[start..end]);
            }
            space.user_pt.0[page] = frame | PAGE_PRESENT | PAGE_USER;
        }
        let data_start = code_pages;
        for page in 0..data_pages {
            let frame = space.dynamic_frames[data_start + page];
            space.user_pt.0[V2_DATA_INDEX + page] =
                frame | PAGE_PRESENT | PAGE_WRITE | PAGE_USER | no_execute;
        }
        let heap_start = code_pages + data_pages;
        for page in 0..V2_HEAP_PAGES {
            let frame = space.dynamic_frames[heap_start + page];
            space.user_pt.0[V2_HEAP_INDEX + page] =
                frame | PAGE_PRESENT | PAGE_WRITE | PAGE_USER | no_execute;
        }
        let stack_start = heap_start + V2_HEAP_PAGES;
        for page in 0..V2_STACK_PAGES {
            let frame = space.dynamic_frames[stack_start + page];
            space.user_pt.0[V2_STACK_INDEX + page] =
                frame | PAGE_PRESENT | PAGE_WRITE | PAGE_USER | no_execute;
        }
        space.data_physical = space.dynamic_frames[data_start];
        space.data_virtual = USER_CODE + V2_DATA_INDEX as u64 * PAGE_BYTES as u64;
        copy_to_frames(
            &space.dynamic_frames[data_start..data_start + data_pages],
            ABI_PAYLOAD_OFFSET,
            image.data,
        );
        for page in 0..code_pages {
            if !crate::paging::seal_form_code_frame(space.dynamic_frames[page]) {
                return reject_preparation(space, PrepareError::WxSeal);
            }
        }
        crate::slog!(
            "EXPOS_FORM_MAP fin={} format=2 code_pages={} data_pages={} heap_pages={} stack_pages={} guards=4\r\n",
            form,
            code_pages,
            data_pages,
            V2_HEAP_PAGES,
            V2_STACK_PAGES
        );
        (
            space.data_virtual,
            USER_CODE + (V2_STACK_INDEX + V2_STACK_PAGES) as u64 * PAGE_BYTES as u64 - 16,
            USER_CODE + V2_END_INDEX as u64 * PAGE_BYTES as u64,
        )
    } else {
        space.user_pt.0[0] = physical(&space.code) | PAGE_PRESENT | PAGE_USER;
        space.user_pt.0[1] =
            physical(&space.data) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER | no_execute;
        // Index 2 is deliberately unmapped as a guard page.
        space.user_pt.0[3] =
            physical(&space.stack) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER | no_execute;
        let payload = image.map_or_else(|| native_payload(content), |image| image.data);
        let payload_len = payload.len().min(PAGE_BYTES - ABI_PAYLOAD_OFFSET);
        space.data.0[ABI_PAYLOAD_OFFSET..ABI_PAYLOAD_OFFSET + payload_len]
            .copy_from_slice(&payload[..payload_len]);
        (USER_DATA, USER_STACK_TOP, USER_END)
    };

    let payload_len = image
        .map_or_else(|| native_payload(content).len(), |image| image.data.len())
        .min(PAGE_BYTES - ABI_PAYLOAD_OFFSET);
    let request = AbiRequest {
        version: FORM_ABI_VERSION,
        call: AbiCall::Log as u16,
        caller: form,
        handle_id: log_handle.id,
        arguments: [ABI_PAYLOAD_OFFSET as u64, payload_len as u64, 0, 0, 0, 0],
    };
    unsafe {
        ptr::write_unaligned(
            (space.data_physical as *mut u8).cast::<AbiRequest>(),
            request,
        );
    }

    if let Some(image) = image.filter(|image| image.format_version == 1) {
        space.code.0[..image.code.len()].copy_from_slice(image.code);
    } else if v2 {
        // v2 code was copied into physical frames before their aliases were sealed.
    } else if content.starts_with(b"native:spin") {
        // PAUSE; JMP -4. This deliberately has no cooperative exit path.
        space.code.0[..4].copy_from_slice(&[0xF3, 0x90, 0xEB, 0xFC]);
    } else if content.starts_with(b"native:escape") {
        // mov rax, moffs64(1 MiB). That address is mapped supervisor-only in
        // every Form root, so CPL3 must take #PF rather than read kernel text.
        space.code.0[..12].copy_from_slice(&[
            0x48, 0xA1, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEB, 0xFE,
        ]);
    } else if content == b"native:write-code" {
        space.code.0[..2].copy_from_slice(&[0x48, 0xB8]); // mov rax, code
        space.code.0[2..10].copy_from_slice(&USER_CODE.to_le_bytes());
        space.code.0[10..15].copy_from_slice(&[0xC6, 0x00, 0x90, 0x0F, 0x0B]);
    } else if content == b"native:exec-data" {
        space.code.0[..2].copy_from_slice(&[0x48, 0xB8]);
        space.code.0[2..10].copy_from_slice(&USER_DATA.to_le_bytes());
        space.code.0[10..12].copy_from_slice(&[0xFF, 0xE0]); // jmp data
    } else if content == b"native:divide-zero" {
        space.code.0[..8].copy_from_slice(&[0x31, 0xD2, 0x31, 0xC9, 0xF7, 0xF1, 0x0F, 0x0B]);
    } else if content == b"native:invalid-opcode" {
        space.code.0[..2].copy_from_slice(&[0x0F, 0x0B]);
    } else if content.starts_with(b"native:ac-log ") {
        // User POPF can set AC; the ABI stub must clear it on kernel entry.
        space.code.0[..10].copy_from_slice(&[0x9C, 0x81, 0x0C, 0x24, 0, 0, 4, 0, 0x9D, 0x90]);
        emit_log_and_exit(&mut space.code.0[10..], execute_handle.id);
    } else {
        emit_log_and_exit(&mut space.code.0, execute_handle.id);
    }

    let root = physical(&space.pml4);
    let address_space = AddressSpace::confined(root, USER_CODE, address_end)
        .expect("fixed native address-space bounds are valid");
    Ok(PreparedForm {
        slot: slot_index,
        address_space,
        frame: TrapFrame {
            rip: USER_CODE + image.map_or(0, |image| image.entry as u64),
            rdi: data_virtual,
            rsi: data_virtual + ABI_RESPONSE_OFFSET,
            rdx: execute_handle.id as u64,
            rcx: log_handle.id as u64,
            rflags: 0x202,
            rsp: stack_top,
            ..TrapFrame::default()
        },
    })
}

fn emit_log_and_exit(code: &mut [u8], execute_handle: u32) {
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

fn copy_to_frames(frames: &[u64], offset: usize, bytes: &[u8]) {
    let mut copied = 0;
    while copied < bytes.len() {
        let position = offset + copied;
        let page = position / PAGE_BYTES;
        let within = position % PAGE_BYTES;
        let count = (PAGE_BYTES - within).min(bytes.len() - copied);
        unsafe {
            core::slice::from_raw_parts_mut(frames[page] as *mut u8, PAGE_BYTES)
                [within..within + count]
                .copy_from_slice(&bytes[copied..copied + count]);
        }
        copied += count;
    }
}

fn release_dynamic_frames(space: &mut NativeSpace) {
    for index in 0..space.dynamic_len as usize {
        let frame = space.dynamic_frames[index];
        if index < space.dynamic_code_len as usize {
            let _ = crate::paging::unseal_form_frame(frame);
        }
        let _ = crate::memory::release_frame(frame);
        space.dynamic_frames[index] = 0;
    }
    space.dynamic_len = 0;
    space.dynamic_code_len = 0;
    space.data_physical = physical(&space.data);
    space.data_virtual = USER_DATA;
}

fn reject_preparation(
    space: &mut NativeSpace,
    error: PrepareError,
) -> Result<PreparedForm, PrepareError> {
    release_dynamic_frames(space);
    space.user_pt.0.fill(0);
    space.user_pd.0.fill(0);
    space.user_pdpt.0.fill(0);
    space.pml4.0.fill(0);
    space.active = false;
    Err(error)
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
    job.network_handle = authorization.network_handle;
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
        service_reply: unsafe {
            (*ptr::addr_of!(NATIVE_SPACES[prepared.slot]))
                .service
                .is_some()
        },
        output: unsafe {
            (*ptr::addr_of_mut!(NATIVE_SPACES[prepared.slot]))
                .output
                .as_deref_mut()
                .map_or(ptr::null_mut(), |out| out as *mut _)
        },
        // The Form cannot edit its PTEs. Use the admitted slot's supervisor
        // alias for bounded ABI copies, keeping SMAP active with AC clear.
        data: unsafe { (*ptr::addr_of!(NATIVE_SPACES[prepared.slot])).data_physical as *mut u8 },
        data_virtual: unsafe { (*ptr::addr_of!(NATIVE_SPACES[prepared.slot])).data_virtual },
        service_memory: unsafe {
            (*ptr::addr_of_mut!(NATIVE_SPACES[prepared.slot]))
                .service
                .as_deref_mut()
                .map_or(ptr::null_mut(), |memory| memory as *mut _)
        },
        broker: authorization.broker,
        cfc: authorization.cfc,
        dimension: authorization.dimension,
        form: authorization.form,
        handles: [
            authorization.execute_handle,
            authorization.log_handle,
            authorization.network_handle,
        ],
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
        let space = &mut (*ptr::addr_of_mut!(NATIVE_SPACES))[prepared.slot];
        space.service = None;
        space.output = None;
        release_dynamic_frames(space);
        space.active = false;
    }
}

#[no_mangle]
pub extern "C" fn expos_form_timer_interrupt(frame: *mut TrapFrame) -> u64 {
    if unsafe { (*frame).cs } & 3 != 3 {
        return DISPOSITION_RESUME;
    }
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
pub extern "C" fn expos_legacy_irq_interrupt(irq: u64) {
    crate::interrupts::handle_legacy_irq(irq as u8);
}

#[no_mangle]
pub extern "C" fn expos_device_interrupt() {
    crate::interrupts::handle_message_interrupt();
}

#[no_mangle]
pub extern "C" fn expos_form_abi_interrupt(frame: *mut TrapFrame) -> u64 {
    #[cfg(feature = "isolation-self-test")]
    crate::isolation_self_test::smap(current_slot());
    let Some(active) = active_execution() else {
        return DISPOSITION_FAULT;
    };
    let frame = unsafe { &mut *frame };
    if frame.rdi != active.data_virtual || frame.rsi != active.data_virtual + ABI_RESPONSE_OFFSET {
        active.fault = 0x80;
        active.frame = *frame;
        return DISPOSITION_FAULT;
    }
    if frame.rax == STREAM_ABI_MAGIC {
        let response = dispatch_stream(active);
        write_abi_response(active.data, response);
        frame.rax = response.status as u64;
        return DISPOSITION_RESUME;
    }
    if frame.rax != ABI_MAGIC {
        active.fault = 0x80;
        active.frame = *frame;
        return DISPOSITION_FAULT;
    }
    let request = unsafe { ptr::read_unaligned(active.data.cast::<AbiRequest>()) };
    #[cfg(feature = "isolation-self-test")]
    if unsafe {
        core::slice::from_raw_parts(
            active.data.add(ABI_PAYLOAD_OFFSET),
            b"kernel-fault-test!".len(),
        )
    } == b"kernel-fault-test!"
    {
        crate::isolation_self_test::fatal_kernel_fault();
    }
    let Some(handle) = active
        .handles
        .iter()
        .find(|handle| handle.id == request.handle_id)
        .copied()
    else {
        write_abi_response(active.data, AbiResponse::status(AbiStatus::Denied));
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
                AbiCall::Log if !active.output.is_null() && !active.service_reply => {
                    capture_reply(active.data, active.output, arguments)
                }
                AbiCall::Log => service_log(active.data, arguments),
                AbiCall::IpcSend if active.service_reply && !active.output.is_null() => {
                    capture_reply(active.data, active.output, arguments)
                }
                AbiCall::HandleAuthorize => {
                    AbiResponse::ok([handle.id as u64, handle.operations.bits() as u64, 0, 0])
                }
                AbiCall::TimeNow => AbiResponse::ok([crate::hardware::timestamp(), 0, 0, 0]),
                AbiCall::NetworkSend if handle.id == active.handles[2].id => {
                    service_network_send(active, arguments)
                }
                AbiCall::NetworkReceive if handle.id == active.handles[2].id => {
                    service_network_receive(active, arguments)
                }
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
    write_abi_response(active.data, response);
    frame.rax = response.status as u64;
    if requested_disposition != DISPOSITION_RESUME {
        active.frame = *frame;
    }
    // Register polling is normal driver flow and can occur hundreds of
    // thousands of times per bounded exchange. The lease, rejected portal
    // requests, service lifecycle and DMA revocation remain audited; avoid
    // serializing every successful allowlisted register read.
    let successful_driver_portal = request.arguments[2] == 3
        && matches!(
            request.call,
            value if value == AbiCall::NetworkSend as u16
                || value == AbiCall::NetworkReceive as u16
        )
        && response.status == AbiStatus::Ok as u16;
    if !successful_driver_portal
        && (request.call != AbiCall::NetworkReceive as u16
            || response.status != AbiStatus::WouldBlock as u16)
    {
        slog!(
            "EXPOS_FORM_ABI fin={} call={} handle={} status={}\r\n",
            active.form,
            request.call,
            request.handle_id,
            response.status
        );
    }
    requested_disposition
}

fn dispatch_stream(active: &mut ActiveExecution) -> AbiResponse {
    const TRANSFER_OFFSET: usize = ABI_PAYLOAD_OFFSET;
    const TRANSFER_BYTES: usize = expos_core::STREAM_CHUNK_BYTES;
    let request = unsafe { ptr::read_unaligned(active.data.cast::<StreamRequest>()) };
    let operation = match request.validate() {
        Ok(operation) => operation,
        Err(status) => return AbiResponse::status(status),
    };
    let Some(handle) = active
        .handles
        .iter()
        .find(|handle| handle.id == request.handle_id)
        .copied()
    else {
        return AbiResponse::status(AbiStatus::Denied);
    };
    let broker = unsafe { &*active.broker };
    if broker
        .authorize_requester(
            handle.id,
            active.form,
            handle.target,
            active.dimension,
            operation,
            crate::hardware::timestamp(),
        )
        .is_err()
    {
        return AbiResponse::status(AbiStatus::Denied);
    }
    let Some(call) = StreamCall::from_raw(request.call) else {
        return AbiResponse::status(AbiStatus::Unsupported);
    };
    if request.length as usize > TRANSFER_BYTES {
        return AbiResponse::status(AbiStatus::Invalid);
    }
    let offset = match usize::try_from(request.offset) {
        Ok(offset) => offset,
        Err(_) => return AbiResponse::status(AbiStatus::Invalid),
    };
    let result = match call {
        StreamCall::OpenRead => {
            crate::expfs_store::open_blob_stream(active.cfc, handle.target, false)
                .map(|(id, length)| [id as u64, length as u64, 0, 0])
        }
        StreamCall::OpenWrite => {
            crate::expfs_store::open_blob_stream(active.cfc, handle.target, true)
                .map(|(id, length)| [id as u64, length as u64, 0, 0])
        }
        StreamCall::ReadChunk => {
            let output = unsafe {
                core::slice::from_raw_parts_mut(
                    active.data.add(TRANSFER_OFFSET),
                    request.length as usize,
                )
            };
            crate::expfs_store::read_blob_stream(
                active.cfc,
                handle.target,
                request.stream_id,
                offset,
                output,
            )
            .map(|(count, total)| [count as u64, total as u64, TRANSFER_OFFSET as u64, 0])
        }
        StreamCall::WriteChunk => {
            let input = unsafe {
                core::slice::from_raw_parts(
                    active.data.add(TRANSFER_OFFSET),
                    request.length as usize,
                )
            };
            crate::expfs_store::write_blob_stream(
                active.cfc,
                handle.target,
                request.stream_id,
                offset,
                input,
            )
            .map(|end| [end as u64, 0, 0, 0])
        }
        StreamCall::Commit => {
            crate::expfs_store::commit_blob_stream(active.cfc, handle.target, request.stream_id)
                .map(|(revision, generation)| [revision as u64, generation, 0, 0])
        }
        StreamCall::Abort => {
            crate::expfs_store::abort_blob_stream(active.cfc, handle.target, request.stream_id)
                .map(|()| [0; 4])
        }
    };
    match result {
        Ok(values) => AbiResponse::ok(values),
        Err(_) => AbiResponse::status(AbiStatus::Invalid),
    }
}

fn capture_reply(
    data: *const u8,
    output: *mut crate::service::Output,
    arguments: [u64; 6],
) -> AbiResponse {
    let (offset, length) = (arguments[0] as usize, arguments[1] as usize);
    if offset < ABI_PAYLOAD_OFFSET
        || length > 512
        || offset
            .checked_add(length)
            .is_none_or(|end| end > PAGE_BYTES)
    {
        return AbiResponse::status(AbiStatus::Invalid);
    }
    let output = unsafe { &mut *output };
    if length > output.bytes.len() - output.length {
        return AbiResponse::status(AbiStatus::Denied);
    }
    let bytes = unsafe { core::slice::from_raw_parts(data.add(offset), length) };
    output.bytes[output.length..output.length + length].copy_from_slice(bytes);
    output.length += length;
    AbiResponse::ok([length as u64, 0, 0, 0])
}

fn service_network_send(active: &mut ActiveExecution, arguments: [u64; 6]) -> AbiResponse {
    if arguments[2] == 4 {
        let Some(memory) = (unsafe { active.service_memory.as_ref() }) else {
            return AbiResponse::status(AbiStatus::Denied);
        };
        let Some(physical) = memory.writable_alias_range(arguments[0], arguments[1] as usize)
        else {
            return AbiResponse::status(AbiStatus::Invalid);
        };
        let frame =
            unsafe { core::slice::from_raw_parts(physical as *const u8, arguments[1] as usize) };
        return match crate::network::service_frame_send(frame) {
            Ok(()) => AbiResponse::ok([arguments[1], 0, 0, 0]),
            Err(_) => AbiResponse::status(AbiStatus::Denied),
        };
    }
    if arguments[2] == 3 {
        return match crate::network::service_driver_write(
            arguments[0] as u16,
            arguments[1] as u8,
            arguments[3] as u32,
        ) {
            Ok(()) => AbiResponse::ok([0; 4]),
            Err(_) => AbiResponse::status(AbiStatus::Denied),
        };
    }
    AbiResponse::status(AbiStatus::Unsupported)
}

fn service_network_receive(active: &mut ActiveExecution, arguments: [u64; 6]) -> AbiResponse {
    if arguments[2] == 4 {
        let Some(memory) = (unsafe { active.service_memory.as_ref() }) else {
            return AbiResponse::status(AbiStatus::Denied);
        };
        let Some(physical) = memory.writable_alias_range(arguments[0], arguments[1] as usize)
        else {
            return AbiResponse::status(AbiStatus::Invalid);
        };
        let frame =
            unsafe { core::slice::from_raw_parts_mut(physical as *mut u8, arguments[1] as usize) };
        return match crate::network::service_frame_receive(frame) {
            Ok(length) => AbiResponse::ok([length as u64, 0, 0, 0]),
            Err(_) => AbiResponse::status(AbiStatus::Denied),
        };
    }
    if arguments[2] == 2 {
        let Some(memory) = (unsafe { active.service_memory.as_ref() }) else {
            return AbiResponse::status(AbiStatus::Denied);
        };
        let Some(physical) = memory.writable_physical_range(arguments[0], arguments[1] as usize)
        else {
            return AbiResponse::status(AbiStatus::Invalid);
        };
        return match crate::network::service_driver_acquire(
            physical,
            arguments[1] as usize,
            arguments[3] as usize,
            arguments[4] as usize,
            arguments[5] as usize,
        ) {
            Ok(()) => AbiResponse::ok([0; 4]),
            Err(_) => AbiResponse::status(AbiStatus::Denied),
        };
    }
    if arguments[2] == 3 {
        return match crate::network::service_driver_read(arguments[0] as u16, arguments[1] as u8) {
            Ok(value) => AbiResponse::ok([value as u64, 0, 0, 0]),
            Err(_) => AbiResponse::status(AbiStatus::Denied),
        };
    }
    if arguments[2] == 1 {
        return match crate::network::service_configuration() {
            Ok(values) => AbiResponse::ok(values),
            Err(_) => AbiResponse::status(AbiStatus::Denied),
        };
    }
    AbiResponse::status(AbiStatus::Unsupported)
}

pub fn prepare_service(
    form: Fin,
    execute: FormHandle,
    reply: FormHandle,
    elf: &[u8],
    input: &[u8],
    network_handle: Option<FormHandle>,
) -> Result<PreparedForm, crate::service::Error> {
    let (mut memory, entry) = crate::service::Memory::load(elf, input)?;
    let stack_top = crate::service::STACK_OFFSET + crate::service::STACK_BYTES;
    if let Some(handle) = network_handle {
        // Seventh SysV argument: the stack begins with a synthetic return
        // address and the sealed Network Handle follows at RSP+8.
        memory.bytes[stack_top - 16..stack_top - 8]
            .copy_from_slice(&(handle.id as u64).to_le_bytes());
    }
    let mut prepared = prepare(form, execute, reply, b"native:spin")
        .map_err(|_| crate::service::Error::Admission)?;
    let space = unsafe { &mut (*ptr::addr_of_mut!(NATIVE_SPACES))[prepared.slot] };
    space.user_pd.0[1] = memory.table.as_ptr() as u64 | page_table_flags();
    space.user_pt.0[0] = 0; // no legacy code/stack in service address spaces
    space.user_pt.0[2] = 0;
    space.service = Some(memory);
    space.output = Some(Box::new(crate::service::Output {
        bytes: [0; crate::service::OUTPUT_BYTES],
        length: 0,
    }));
    prepared.frame.rip = entry;
    prepared.frame.r8 = crate::service::BASE + crate::service::INPUT_OFFSET as u64;
    prepared.frame.r9 = input.len() as u64;
    prepared.frame.r10 = network_handle.map_or(0, |handle| handle.id as u64);
    prepared.frame.rsp =
        crate::service::BASE + stack_top as u64 - if network_handle.is_some() { 24 } else { 8 };
    prepared.address_space = AddressSpace::confined(
        physical(&space.pml4),
        USER_CODE,
        crate::service::BASE + crate::service::MEMORY_BYTES as u64,
    )
    .expect("fixed service range");
    Ok(prepared)
}

pub fn capture_output(prepared: PreparedForm) {
    unsafe {
        (*ptr::addr_of_mut!(NATIVE_SPACES))[prepared.slot].output =
            Some(Box::new(crate::service::Output {
                bytes: [0; crate::service::OUTPUT_BYTES],
                length: 0,
            }));
    }
}

pub fn take_service_output(prepared: PreparedForm) -> Option<Box<crate::service::Output>> {
    unsafe {
        (*ptr::addr_of_mut!(NATIVE_SPACES))[prepared.slot]
            .output
            .take()
    }
}

fn service_log(data: *const u8, arguments: [u64; 6]) -> AbiResponse {
    let offset = arguments[0] as usize;
    let length = arguments[1] as usize;
    let Some(end) = offset.checked_add(length) else {
        return AbiResponse::status(AbiStatus::Invalid);
    };
    if offset < ABI_PAYLOAD_OFFSET || end > PAGE_BYTES || length > 512 {
        return AbiResponse::status(AbiStatus::Invalid);
    }
    let bytes = unsafe { core::slice::from_raw_parts(data.add(offset), length) };
    let Ok(message) = core::str::from_utf8(bytes) else {
        return AbiResponse::status(AbiStatus::Invalid);
    };
    println!("{}", message);
    AbiResponse::ok([length as u64, 0, 0, 0])
}

fn write_abi_response(data: *mut u8, response: AbiResponse) {
    unsafe {
        ptr::write_unaligned(
            data.add(ABI_RESPONSE_OFFSET as usize).cast::<AbiResponse>(),
            response,
        );
    }
}

#[repr(C)]
pub struct FaultFrame {
    registers: [u64; 15],
    vector: u64,
    error: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

const _: () = assert!(size_of::<FaultFrame>() == 176);
const _: () = assert!(core::mem::offset_of!(FaultFrame, vector) == 120);
const _: () = assert!(core::mem::offset_of!(FaultFrame, rip) == 136);
const _: () = assert!(core::mem::offset_of!(FaultFrame, cs) == 144);

#[no_mangle]
pub extern "C" fn expos_form_fault_interrupt(frame: *mut FaultFrame) -> u64 {
    let frame = unsafe { &mut *frame };
    let address: u64;
    unsafe { asm!("mov {}, cr2", out(reg) address, options(nomem, nostack, preserves_flags)) };
    #[cfg(feature = "isolation-self-test")]
    if frame.cs & 3 == 0
        && crate::isolation_self_test::recover(
            current_slot(),
            frame.vector,
            frame.error,
            address,
            &mut frame.rip,
        )
    {
        return DISPOSITION_RESUME;
    }
    if frame.cs & 3 == 3 {
        if let Some(active) = active_execution() {
            active.fault = frame.vector as u8;
            slog!(
                "EXPOS_FORM_FAULT fin={} vector={} error={:#x} rip={:#x} address={:#x} cpl=3\r\n",
                active.form,
                frame.vector,
                frame.error,
                frame.rip,
                address
            );
            return DISPOSITION_FAULT;
        }
    }
    // A supervisor fault is never recoverable through an active Form's saved
    // stack/CR3. Use COM1 directly: the fault may have interrupted a log lock.
    {
        let mut serial = crate::serial::SerialPort::new(0x3F8);
        let _ = core::fmt::Write::write_fmt(
            &mut serial,
            format_args!(
                "EXPOS_KERNEL_FAULT vector={} error={:#x} rip={:#x} address={:#x} cpl={}\r\n",
                frame.vector,
                frame.error,
                frame.rip,
                address,
                frame.cs & 3
            ),
        );
    }
    loop {
        unsafe {
            asm!("cli; hlt", options(nomem, nostack));
        }
    }
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
