; ExpOS transitional Multiboot2 boot stub
;
; GRUB (Multiboot2) enters here in 32-bit protected mode with paging
; disabled. This stub:
;   1. verifies the CPU supports 64-bit long mode
;   2. builds identity-mapped page tables covering the first 1 GiB and the
;      fourth-GiB PCI/MMIO window (2 MiB huge pages)
;   3. enables PAE + LME + paging, loads a 64-bit GDT, far-jumps
;      into long mode
;   4. hands control to the Rust kernel: kernel_main(magic, mbi_phys)

MB2_MAGIC   equ 0xE85250D6
ARCH_I386   equ 0
MSR_EFER    equ 0xC0000080
EFER_LME    equ 1 << 8

CODE_SEG    equ 0x08
DATA_SEG    equ 0x10

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
; Page tables (identity map of RAM below 1 GiB plus PCI/MMIO in the fourth GiB)
; ---------------------------------------------------------------------------
section .bss
align 4096
pml4_table:
        resb 4096
pdpt_table:
        resb 4096
pd_table:
        resb 4096
pd_high_table:
        resb 4096

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

; ---------------------------------------------------------------------------
; Minimal 64-bit GDT
; ---------------------------------------------------------------------------
section .rodata
align 16
gdt64:
        dq 0                            ; null descriptor
        dq 0x00209A0000000000           ; 0x08: kernel code (L=1, D=0)
        dq 0x0000920000000000           ; 0x10: kernel data
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

        ; pdpt[0] -> pd | PRESENT | WRITE
        mov eax, pd_table
        or  eax, 0b11
        mov [pdpt_table], eax

        ; pdpt[3] -> high identity map for 0xC0000000..0xFFFFFFFF.
        ; QEMU standard VGA maps its 16 MiB linear framebuffer BAR at
        ; 0xFD000000. A 1920x1080x32 scanout consumes 8,294,400 bytes and
        ; therefore ends at 0xFD7E8FFF, well within this high mapping.
        mov eax, pd_high_table
        or  eax, 0b11
        mov [pdpt_table + 3*8], eax

        ; pd[i] = i * 2MiB | PRESENT | WRITE | HUGE  (512 entries = 1 GiB)
        xor ecx, ecx
.fill_pd:
        mov eax, 0x200000               ; 2 MiB
        mul ecx                         ; eax = ecx * 2MiB (< 4 GiB)
        or  eax, 0b10000011             ; P | RW | PS
        mov [pd_table + ecx*8], eax
        inc ecx
        cmp ecx, 512
        jne .fill_pd

        ; Map the fourth GiB with 2 MiB pages for PCI MMIO/framebuffer access.
        xor ecx, ecx
.fill_pd_high:
        mov eax, ecx
        shl eax, 21
        add eax, 0xC0000000
        or  eax, 0b10000011
        mov [pd_high_table + ecx*8], eax
        inc ecx
        cmp ecx, 512
        jne .fill_pd_high

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
        mov [pml4_table], rax
        mov rax, pd_table
        or rax, 3
        mov [pdpt_table], rax
        mov rax, pd_high_table
        or rax, 3
        mov [pdpt_table + 3*8], rax
        xor ecx, ecx
.native_map:
        mov rax, rcx
        shl rax, 21
        or rax, 0x83
        mov [pd_table + rcx*8], rax
        add eax, 0xC0000000
        mov [pd_high_table + rcx*8], rax
        inc ecx
        cmp ecx, 512
        jne .native_map
        mov rax, pml4_table
        mov cr3, rax
        call kernel_main
        cli
.native_halt:
        hlt
        jmp .native_halt

section .note.GNU-stack noalloc noexec nowrite progbits
