/* ExpOS transitional Multiboot2 kernel linker script.
 * Image is loaded by GRUB at 1 MiB physical. The Multiboot2 header
 * section must sit within the first 32 KiB of the image, so it comes
 * first. */
ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(5); /* read + execute */
    rodata PT_LOAD FLAGS(4); /* read, never execute */
    data PT_LOAD FLAGS(6); /* read + write */
}

SECTIONS
{
    . = DEFINED(KERNEL_LOAD_BASE) ? KERNEL_LOAD_BASE : 1M;

    .multiboot_header : ALIGN(8)
    {
        KEEP(*(.multiboot_header))
    } :text

    . = ALIGN(4K);
    __kernel_text_start = .;
    .text :
    {
        *(.text .text.*)
    } :text

    . = ALIGN(4K);
    __kernel_text_end = .;
    __kernel_rodata_start = .;
    .rodata :
    {
        *(.rodata .rodata.*)
    } :rodata

    . = ALIGN(4K);
    __kernel_rodata_end = .;
    .data :
    {
        *(.data .data.*)
    } :data

    .got :
    {
        *(.got .got.*)
    } :data

    .bss :
    {
        *(COMMON)
        *(.bss .bss.*)
    } :data

    . = ALIGN(4K);
    __kernel_end = .;

    /DISCARD/ :
    {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}
