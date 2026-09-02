/* HexaOS transitional Multiboot2 kernel linker script.
 * Image is loaded by GRUB at 1 MiB physical. The Multiboot2 header
 * section must sit within the first 32 KiB of the image, so it comes
 * first. */
ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(5); /* read + execute */
    data PT_LOAD FLAGS(6); /* read + write */
}

SECTIONS
{
    . = 1M;

    .multiboot_header : ALIGN(8)
    {
        KEEP(*(.multiboot_header))
    } :text

    .text :
    {
        *(.text .text.*)
    } :text

    .rodata :
    {
        *(.rodata .rodata.*)
    } :text

    . = ALIGN(4K);
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

    /DISCARD/ :
    {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}
