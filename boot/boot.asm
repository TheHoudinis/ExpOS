; ExpOS transitional Multiboot2 boot stub
;
; GRUB (Multiboot2) enters here in 32-bit protected mode with paging
; disabled. This stub:
;   1. verifies the CPU supports 64-bit long mode
;   2. builds identity-mapped page tables covering RAM and PCI/MMIO below
;      4 GiB (2 MiB huge pages)
;   3. enables PAE + LME + paging, loads a 64-bit GDT, far-jumps
;      into long mode
;   4. hands control to the Rust kernel: kernel_main(magic, mbi_phys)

MB2_MAGIC   equ 0xE85250D6
ARCH_I386   equ 0
MSR_EFER    equ 0xC0000080
EFER_LME    equ 1 << 8

CODE_SEG    equ 0x08
DATA_SEG    equ 0x10
USER_DATA_SEG equ 0x18
USER_CODE_SEG equ 0x20
TSS_SEG       equ 0x28

; ---------------------------------------------------------------------------
; Multiboot2 header (must live in the first 32 KiB of the image, 8-aligned)
; ---------------------------------------------------------------------------
section .multiboot_header
align 8
header_start:
        dd MB2_MAGIC                    ; magic
        dd ARCH_I386                    ; architecture: i386 (GRUB entry)
        dd header_end - header_start    ; header length
        dd -(MB2_MAGIC + ARCH_I386 + (header_end - header_start)) ; checksum
        dw 0                            ; end tag
        dw 0
        dd 8
header_end:

; ---------------------------------------------------------------------------
; Page tables (identity map below 4 GiB, including firmware-assigned PCI BARs)
; ---------------------------------------------------------------------------
section .bss
align 4096
pml4_table:
        resb 4096
pdpt_table:
        resb 4096
pd_table:
        resb 4096 * 4

align 16
stack_bottom:
        ; The graphical Form session uses fixed-capacity, allocation-free
        ; document and surface state. Give those values room without letting
        ; the downward-growing stack collide with the page tables.
        ; TLS certificate verification plus a 16 KiB response and two bounded
        ; browser documents can coexist during a search projection. Keep the
        ; early single-core stack explicit until per-Form stacks land.
        resb 1048576
stack_top:

; Ring-3 interrupts always enter on this kernel-owned stack through the TSS.
; A Form can corrupt its own user stack without controlling an interrupt frame.
align 16
form_interrupt_stack_bottom:
        resb 65536
form_interrupt_stack_top:

align 16
tss64:
        resd 1                         ; reserved
        resq 1                         ; RSP0 (installed below)
        resq 2                         ; RSP1/RSP2
        resq 8                         ; reserved + IST1..IST7
        resq 1                         ; reserved
        resw 1                         ; reserved
        resw 1                         ; I/O bitmap offset
tss64_end:

align 8
form_kernel_rsp:
        resq 1
form_kernel_cr3:
        resq 1
form_kernel_rbx:
        resq 1
form_kernel_rbp:
        resq 1
form_kernel_r12:
        resq 1
form_kernel_r13:
        resq 1
form_kernel_r14:
        resq 1
form_kernel_r15:
        resq 1

; ---------------------------------------------------------------------------
; Minimal 64-bit GDT
; ---------------------------------------------------------------------------
section .rodata
align 16
gdt64:
        dq 0                            ; null descriptor
        dq 0x00209A0000000000           ; 0x08: kernel code (L=1, D=0)
        dq 0x0000920000000000           ; 0x10: kernel data
        dq 0x0000F20000000000           ; 0x18: user data (DPL=3)
        dq 0x0020FA0000000000           ; 0x20: user code (DPL=3, L=1)
gdt64_tss:
        dq 0                            ; 0x28: 64-bit available TSS (low)
        dq 0                            ;       base bits 32..63 (high)
gdt64_end:

gdt64_desc:
        dw gdt64_end - gdt64 - 1        ; limit
        dq gdt64                        ; base

; ---------------------------------------------------------------------------
; Code
; ---------------------------------------------------------------------------
section .text
bits 32
global _start
extern kernel_main

_start:
        ; stash Multiboot2 registers (SysV: edi = magic, esi = mbi phys ptr)
        mov edi, eax
        mov esi, ebx

        ; ---- CPUID available? toggle EFLAGS.ID -------------------------
        pushfd
        pop eax
        mov ecx, eax
        xor eax, 1 << 21
        push eax
        popfd
        pushfd
        pop eax
        push ecx
        popfd
        cmp eax, ecx
        je .no_long_mode

        ; ---- extended CPUID leaf present? ------------------------------
        mov eax, 0x80000000
        cpuid
        cmp eax, 0x80000001
        jb .no_long_mode

        ; ---- long mode supported? (EDX.LM bit 29) ----------------------
        mov eax, 0x80000001
        cpuid
        test edx, 1 << 29
        jz .no_long_mode

        ; ---- build page tables ----------------------------------------
        ; pml4[0] -> pdpt | PRESENT | WRITE
        mov eax, pdpt_table
        or  eax, 0b11
        mov [pml4_table], eax

        ; Firmware chooses the VGA BAR: SeaBIOS commonly uses 0xFD000000,
        ; OVMF 0x80000000. Map all four directories before accessing it.
        xor ecx, ecx
.fill_pdpt:
        mov eax, ecx
        shl eax, 12
        add eax, pd_table
        or eax, 0b11
        mov [pdpt_table + ecx*8], eax
        inc ecx
        cmp ecx, 4
        jne .fill_pdpt

        ; pd[i] = i * 2MiB | PRESENT | WRITE | HUGE (2048 entries = 4 GiB)
        xor ecx, ecx
.fill_pd:
        mov eax, 0x200000               ; 2 MiB
        mul ecx                         ; eax = ecx * 2MiB (< 4 GiB)
        or  eax, 0b10000011             ; P | RW | PS
        mov [pd_table + ecx*8], eax
        inc ecx
        cmp ecx, 2048
        jne .fill_pd

        ; ---- enter long mode -------------------------------------------
        mov eax, pml4_table
        mov cr3, eax

        mov ecx, MSR_EFER
        rdmsr
        or  eax, EFER_LME               ; LME
        wrmsr

        mov eax, cr4
        or  eax, 1 << 5                 ; PAE
        mov cr4, eax

        mov eax, cr0
        or  eax, 1 << 31                ; PG (PE already set by GRUB)
        mov cr0, eax

        lgdt [gdt64_desc]
        jmp CODE_SEG:.long_mode

.no_long_mode:                          ; 64-bit unsupported: blink and die
        cli
.hang_err:
        hlt
        jmp .hang_err

bits 64
.long_mode:
        mov ax, DATA_SEG
        mov ds, ax
        mov es, ax
        mov fs, ax
        mov gs, ax
        mov ss, ax

        mov rsp, stack_top
        xor ebp, ebp

        ; kernel_main(magic: u32 /*edi*/, mbi_phys: u64 /*rsi*/)
        call kernel_main

        cli                             ; kernel_main is diverging; belt+suspenders
.hang:
        hlt
        jmp .hang

; Native UEFI already runs in long mode. EFI has exited boot services before
; this entry; switch to an owned stack, GDT and page tables before Rust starts.
; Arguments use the kernel's SysV ABI, not UEFI's Microsoft x64 ABI.
global _uefi_start
_uefi_start:
        cli
        cld
        mov rsp, stack_top
        xor ebp, ebp
        lgdt [rel gdt64_desc]
        push CODE_SEG
        lea rax, [rel .native_cs]
        push rax
        retfq
.native_cs:
        mov ax, DATA_SEG
        mov ds, ax
        mov es, ax
        mov ss, ax
        xor eax, eax
        mov fs, ax
        mov gs, ax
        mov rax, pdpt_table
        or rax, 3
        mov [rel pml4_table], rax
        xor ecx, ecx
.native_pdpt:
        mov rax, rcx
        shl rax, 12
        add rax, pd_table
        or rax, 3
        mov [pdpt_table + rcx*8], rax
        inc ecx
        cmp ecx, 4
        jne .native_pdpt
        xor ecx, ecx
.native_map:
        mov rax, rcx
        shl rax, 21
        or rax, 0x83
        mov [pd_table + rcx*8], rax
        inc ecx
        cmp ecx, 2048
        jne .native_map
        mov rax, pml4_table
        mov cr3, rax
        call kernel_main
        cli
.native_halt:
        hlt
        jmp .native_halt

; ---------------------------------------------------------------------------
; x86_64 Form execution boundary
; ---------------------------------------------------------------------------

; Install the long-mode TSS descriptor and load TR. The IDT itself is owned by
; Rust because its entries are easier to audit there.
global expos_arch_install_form_tss
expos_arch_install_form_tss:
        cli
        lea rax, [rel form_interrupt_stack_top]
        mov [rel tss64 + 4], rax
        mov word [rel tss64 + 102], tss64_end - tss64

        lea rax, [rel tss64]
        mov rcx, rax
        and rax, 0xFFFFFF
        shl rax, 16
        or rax, (tss64_end - tss64 - 1)
        mov rdx, 0x89
        shl rdx, 40
        or rax, rdx
        lea rcx, [rel tss64]
        mov rdx, rcx
        shr rdx, 24
        and rdx, 0xFF
        shl rdx, 56
        or rax, rdx
        mov [rel gdt64_tss], rax
        shr rcx, 32
        mov [rel gdt64_tss + 8], ecx
        lgdt [rel gdt64_desc]
        mov ax, TSS_SEG
        ltr ax
        ret

; void expos_arch_enter_form(u64 cr3, const TrapFrame *state)
; The eventual interrupt exit jumps back to the return address on this saved
; kernel stack, so this ordinary-looking call encloses the complete CPL3 slice.
global expos_arch_enter_form
expos_arch_enter_form:
        cli
        mov [rel form_kernel_rsp], rsp
        mov [rel form_kernel_rbx], rbx
        mov [rel form_kernel_rbp], rbp
        mov [rel form_kernel_r12], r12
        mov [rel form_kernel_r13], r13
        mov [rel form_kernel_r14], r14
        mov [rel form_kernel_r15], r15
        mov rax, cr3
        mov [rel form_kernel_cr3], rax
        mov cr3, rdi

        ; Hardware iret frame: SS, RSP, RFLAGS, CS, RIP.
        push qword (USER_DATA_SEG | 3)
        push qword [rsi + 144]
        mov rax, [rsi + 136]
        or rax, 0x202                   ; reserved bit plus IF
        push rax
        push qword (USER_CODE_SEG | 3)
        push qword [rsi + 120]

        ; Restore the user register image. RSP is supplied by the iret frame.
        mov r15, [rsi + 0]
        mov r14, [rsi + 8]
        mov r13, [rsi + 16]
        mov r12, [rsi + 24]
        mov r11, [rsi + 32]
        mov r10, [rsi + 40]
        mov r9,  [rsi + 48]
        mov r8,  [rsi + 56]
        mov rdi, [rsi + 72]
        mov rbp, [rsi + 80]
        mov rdx, [rsi + 88]
        mov rcx, [rsi + 96]
        mov rbx, [rsi + 104]
        mov rax, [rsi + 112]
        mov rsi, [rsi + 64]
        iretq

; TrapFrame layout shared with kernel/src/form_runtime.rs. Push every general
; register so a timer slice can resume without cooperative save points.
%macro FORM_PUSH_REGS 0
        push rax
        push rbx
        push rcx
        push rdx
        push rbp
        push rdi
        push rsi
        push r8
        push r9
        push r10
        push r11
        push r12
        push r13
        push r14
        push r15
%endmacro

%macro FORM_POP_REGS 0
        pop r15
        pop r14
        pop r13
        pop r12
        pop r11
        pop r10
        pop r9
        pop r8
        pop rsi
        pop rdi
        pop rbp
        pop rdx
        pop rcx
        pop rbx
        pop rax
%endmacro

extern expos_form_timer_interrupt
extern expos_form_abi_interrupt
extern expos_form_fault_interrupt

global expos_form_timer_stub
expos_form_timer_stub:
        cld
        FORM_PUSH_REGS
        mov rdi, rsp
        call expos_form_timer_interrupt
        mov rdi, rax
        mov al, 0x20
        out 0x20, al                    ; master PIC EOI
        test rdi, rdi
        jnz .leave
        FORM_POP_REGS
        iretq
.leave:
        mov rax, rdi
        jmp expos_arch_leave_form

global expos_form_abi_stub
expos_form_abi_stub:
        cld
        FORM_PUSH_REGS
        mov rdi, rsp
        call expos_form_abi_interrupt
        test rax, rax
        jnz expos_arch_leave_form
        FORM_POP_REGS
        iretq

; User faults terminate only the active Form. Kernel faults are rejected by
; the Rust side and halt rather than being mistaken for a Form exit.
global expos_form_ud_stub
expos_form_ud_stub:
        mov edi, 6
        sub rsp, 8
        call expos_form_fault_interrupt
        add rsp, 8
        mov eax, 4
        jmp expos_arch_leave_form

global expos_form_gp_stub
expos_form_gp_stub:
        mov edi, 13
        call expos_form_fault_interrupt
        mov eax, 4
        jmp expos_arch_leave_form

global expos_form_pf_stub
expos_form_pf_stub:
        mov edi, 14
        call expos_form_fault_interrupt
        mov eax, 4
        jmp expos_arch_leave_form

expos_arch_leave_form:
        cli
        mov rdx, [rel form_kernel_cr3]
        mov cr3, rdx
        mov rsp, [rel form_kernel_rsp]
        mov rbx, [rel form_kernel_rbx]
        mov rbp, [rel form_kernel_rbp]
        mov r12, [rel form_kernel_r12]
        mov r13, [rel form_kernel_r13]
        mov r14, [rel form_kernel_r14]
        mov r15, [rel form_kernel_r15]
        ret

section .note.GNU-stack noalloc noexec nowrite progbits
