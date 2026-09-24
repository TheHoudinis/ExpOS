bits 64
section .rdata align=16
global kernel_start
global kernel_end
kernel_start:
incbin "build/genesis/kernel.elf"
kernel_end:
