OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)
PHDRS { text PT_LOAD FLAGS(5); rodata PT_LOAD FLAGS(4); data PT_LOAD FLAGS(6); }
SECTIONS {
    . = 0x400000200000;
    .text : { *(.text.entry) *(.text .text.* .ltext .ltext.*) } :text
    . = ALIGN(4096);
    .rodata : { *(.rodata .rodata.* .lrodata .lrodata.*) } :rodata
    . = ALIGN(4096);
    .data : { *(.data .data.* .ldata .ldata.*) } :data
    .got : { *(.got) *(.got.plt) } :data
    .bss (NOLOAD) : { *(.bss .bss.* .lbss .lbss.*) *(COMMON) } :data
    /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
    ASSERT(. <= 0x400000380000, "service exceeds private image region")
}
