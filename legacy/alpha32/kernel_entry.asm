; Kernel-mode assembly: interrupt stubs, context switch, syscall entry
[BITS 32]
section .text

; --------------------------------------
; ISR stubs (CPU exceptions 0-31)
; For exceptions that push an error code, use ISR_ERR
; For exceptions without error code, use ISR_NOERR
; --------------------------------------
%macro ISR_NOERR 1
global isr%1
isr%1:
    push dword 0          ; dummy error code
    push dword %1         ; interrupt number
    jmp isr_common
%endmacro

%macro ISR_ERR 1
global isr%1
isr%1:
    push dword %1         ; interrupt number (CPU already pushed error code)
    jmp isr_common
%endmacro

ISR_NOERR 0
ISR_NOERR 1
ISR_NOERR 2
ISR_NOERR 3
ISR_NOERR 4
ISR_NOERR 5
ISR_NOERR 6
ISR_NOERR 7
ISR_ERR    8
ISR_NOERR 9
ISR_ERR    10
ISR_ERR    11
ISR_ERR    12
ISR_ERR    13
ISR_ERR    14
ISR_NOERR 15
ISR_NOERR 16
ISR_ERR    17
ISR_NOERR 18
ISR_NOERR 19
ISR_NOERR 20
ISR_NOERR 21
ISR_NOERR 22
ISR_NOERR 23
ISR_NOERR 24
ISR_NOERR 25
ISR_NOERR 26
ISR_NOERR 27
ISR_NOERR 28
ISR_NOERR 29
ISR_ERR    30
ISR_NOERR 31

extern isr_handler
extern syscall_handler
isr_common:
    pusha
    push ds
    push es
    push fs
    push gs
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    push esp               ; regs pointer
    cld
    call isr_handler
    add esp, 4
    pop gs
    pop fs
    pop es
    pop ds
    popa
    add esp, 8             ; pop int_no and error code
    iretd

; --------------------------------------
; IRQ stubs (PIC hardware interrupts)
; Mapped to INT 0x20-0x2F
; --------------------------------------
%macro IRQ 2
global irq%1
irq%1:
    push dword 0
    push dword %2
    jmp irq_common
%endmacro

IRQ 0, 32
IRQ 1, 33
IRQ 2, 34
IRQ 3, 35
IRQ 4, 36
IRQ 5, 37
IRQ 6, 38
IRQ 7, 39
IRQ 8, 40
IRQ 9, 41
IRQ 10, 42
IRQ 11, 43
IRQ 12, 44
IRQ 13, 45
IRQ 14, 46
IRQ 15, 47

extern irq_handler
irq_common:
    pusha
    push ds
    push es
    push fs
    push gs
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    push esp               ; regs pointer
    cld
    call irq_handler
    add esp, 4
    pop gs
    pop fs
    pop es
    pop ds
    popa
    add esp, 8
    iretd

; --------------------------------------
; Syscall interrupt handler (INT 0x80)
; eax = syscall number
; ebx, ecx, edx = args
; Returns result in eax
; --------------------------------------
global syscall_stub
syscall_stub:
    push dword 0
    push dword 0x80
    pusha
    push ds
    push es
    push fs
    push gs
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    push esp
    cld
    call syscall_handler
    add esp, 4
    pop gs
    pop fs
    pop es
    pop ds
    mov [esp + 0], eax    ; overwrite saved EAX with return value
    popa
    add esp, 8
    iretd

; --------------------------------------
; Load TSS (call after GDT has TSS entry)
; --------------------------------------
global tss_flush
tss_flush:
    mov ax, 0x2B          ; TSS segment selector (GDT index 5, RPL=3)
    ltr ax
    ret

; --------------------------------------
; Context switch — called from scheduler
; void switch_task(uint32_t *old_esp, uint32_t new_esp)
; Saves current ESP to *old_esp, loads new_esp into ESP, returns
; --------------------------------------
global switch_task
switch_task:
    mov eax, [esp + 4]    ; old_esp pointer
    mov edx, [esp + 8]    ; new_esp
    mov [eax], esp        ; save current ESP
    mov esp, edx          ; load new ESP
    ret

; --------------------------------------
; Enter user mode
; void enter_userspace(uint32_t entry, uint32_t stack, uint32_t cs, uint32_t ds)
; --------------------------------------
global enter_userspace
enter_userspace:
    mov ebx, [esp + 4]    ; entry point
    mov ecx, [esp + 8]    ; user stack
    mov edx, [esp + 12]   ; user CS (with RPL 3)
    mov eax, [esp + 16]   ; user DS (with RPL 3)
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    push eax              ; SS
    push ecx              ; ESP
    pushf                 ; EFLAGS
    push edx              ; CS
    push ebx              ; EIP
    iretd
