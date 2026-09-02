#include "types.h"
#include "elf.h"

int elf_validate(struct elf_header *hdr) {
    if (hdr->magic != ELF_MAGIC) return 0;
    if (hdr->arch != 1) return 0;
    if (hdr->machine != 3) return 0;
    if (hdr->type != 2) return 0;
    return 1;
}

int elf_load(struct elf_header *hdr, uint32_t *entry, uint32_t *heap_start) {
    if (!elf_validate(hdr)) return -1;
    *entry = hdr->entry;
    uint32_t max_addr = 0;
    struct elf_prog_header *ph = (struct elf_prog_header *)((uint32_t)hdr + hdr->phoff);
    for (int i = 0; i < hdr->phnum; i++) {
        if (ph[i].type == PT_LOAD) {
            uint32_t src = (uint32_t)hdr + ph[i].offset;
            uint32_t dst = ph[i].vaddr;
            uint32_t memsz = ph[i].memsz;
            uint32_t filesz = ph[i].filesz;
            // Copy segment
            uint8_t *s = (uint8_t *)src;
            uint8_t *d = (uint8_t *)dst;
            for (uint32_t j = 0; j < filesz; j++) d[j] = s[j];
            // Zero BSS
            for (uint32_t j = filesz; j < memsz; j++) d[j] = 0;
            uint32_t seg_end = dst + memsz;
            if (seg_end > max_addr) max_addr = seg_end;
        }
    }
    if (heap_start) *heap_start = (max_addr + 0xFFF) & ~0xFFF;
    return 0;
}
