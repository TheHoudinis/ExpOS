#ifndef HEX_BIN_H
#define HEX_BIN_H

#include "types.h"
#include "hexafs_disk.h"

#define HEX_MAGIC 0x48455841

typedef struct __attribute__((packed)) {
    uint32_t magic;
    uint32_t version;
    uint32_t entry_point;
    uint32_t code_offset;
    uint32_t code_size;
    uint32_t input_schema_hash;
    uint32_t output_schema_hash;
    uint32_t cap_count;
    uint32_t caps[16];
    uint32_t dep_count;
    uint32_t dep_hashes[16];
    uint32_t checksum;
} hex_header_t;

int hex_validate(hex_header_t *hdr);
int hex_load(hex_header_t *hdr, uint32_t *entry, uint32_t *heap_start);
int hex_pack_elf(const char *elf_name, const char *caps_str, uint32_t in_schema, uint32_t out_schema);

#endif
