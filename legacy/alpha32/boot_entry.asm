[BITS 16]
[ORG 0x7C00]

; Kernel sector count - patched at build time by Makefile (offset 27)
; Default 160, Makefile overwrites byte at offset 27

start:
    cli
    cld
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00
    
    ; Enable A20 gate
    in al, 0x92
    or al, 0x02
    out 0x92, al

    ; Load kernel from floppy using CHS
    ; Use multiple 128-sector banks to handle kernels > 64KB
    mov ax, 0x1000
    mov es, ax
    xor bx, bx

    mov si, 160  ; patched by Makefile
    mov ch, 0
    mov cl, 2               ; start at sector 2
    mov dh, 0
    mov dl, 0               ; floppy drive 0

.read_loop:
    mov ah, 0x02
    mov al, 1
    int 0x13
    jc .disk_fail
    
    add bx, 512
    inc cl
    cmp cl, 19
    jne .no_ch_wrap
    mov cl, 1
    inc dh
    cmp dh, 2
    jne .no_cyl_inc
    mov dh, 0
    inc ch
.no_cyl_inc:
.no_ch_wrap:

    ; Check if BX wrapped past segment boundary
    cmp bx, 0
    jne .no_seg_wrap
    ; BX wrapped: advance ES by 0x1000 (64KB)
    push ax
    mov ax, es
    add ax, 0x1000
    mov es, ax
    pop ax
.no_seg_wrap:

    dec si
    jnz .read_loop

    ; Load GDT
    lgdt [gdt_ptr]
    
    ; Enter protected mode
    mov eax, cr0
    or eax, 1
    mov cr0, eax
    
    ; Long jump to 32-bit code
    jmp 0x08:enter_32bit

.disk_fail:
    hlt
    jmp .disk_fail

[BITS 32]
enter_32bit:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax
    
    mov esp, 0x7A00
    xor ebp, ebp
    
    jmp 0x10000

[BITS 16]
align 4
gdt:
    dq 0
    dw 0xFFFF, 0x0000, 0x9A00, 0x00CF
    dw 0xFFFF, 0x0000, 0x9200, 0x00CF

gdt_ptr:
    dw 23
    dd gdt

times 510 - ($ - $$) db 0x00
dw 0xAA55
