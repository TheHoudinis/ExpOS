OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)
SECTIONS {
    . = 0x400000000000;
    .text : { *(.text.entry) *(.text .text.* .ltext .ltext.*) *(.rodata .rodata.* .lrodata .lrodata.*) }
    . = 0x400000009100;
    .data : { *(.data .data.* .ldata .ldata.*) }
    .bss (NOLOAD) : { *(.bss .bss.* .lbss .lbss.*) *(COMMON) }
    /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
    ASSERT(SIZEOF(.text) > 0 && SIZEOF(.text) <= 32768, "invalid native code size")
    ASSERT(ADDR(.bss) + SIZEOF(.bss) <= 0x40000000f000, "native data exceeds v2 region")
}
