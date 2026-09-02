#ifndef ELF_H
#define ELF_H

#include "types.h"

#define ELF_MAGIC 0x464C457F

struct elf_header {
    uint32_t magic;
    uint8_t  arch;
    uint8_t  endian;
    uint8_t  hdr_ver;
    uint8_t  abi;
    uint8_t  pad[8];
    uint16_t type;
    uint16_t machine;
    uint32_t version;
    uint32_t entry;
    uint32_t phoff;
    uint32_t shoff;
    uint32_t flags;
    uint16_t ehsize;
    uint16_t phentsize;
    uint16_t phnum;
    uint16_t shentsize;
    uint16_t shnum;
    uint16_t shstrndx;
} __attribute__((packed));

struct elf_prog_header {
    uint32_t type;
    uint32_t offset;
    uint32_t vaddr;
    uint32_t paddr;
    uint32_t filesz;
    uint32_t memsz;
    uint32_t flags;
    uint32_t align;
} __attribute__((packed));

#define PT_LOAD 1

int elf_validate(struct elf_header *hdr);
int elf_load(struct elf_header *hdr, uint32_t *entry, uint32_t *heap_start);

#endif
