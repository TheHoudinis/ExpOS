#include "types.h"
#include "hex.h"
#include "elf.h"
#include "hexafs.h"
#include "process.h"
#include "log.h"

extern void *memset(void *dest, int c, size_t len);
extern void *memcpy(void *dest, const void *src, size_t len);
extern void print_string(const char *str);
extern void print_color(const char *str, uint8_t color);
extern int strcmp(const char *s1, const char *s2);
extern size_t strlen(const char *str);
extern void itoa(int num, char *str, int base);
extern void cmd_mkform(const char *name);
extern int find_form(const char *name);
extern void *kmalloc(size_t size);
extern void kfree(void *ptr);

struct hexa_formentry {
    char name[32];
    char *content;
    int size;
    int cap;
    int owner;
    uint16_t mode;
};
extern struct hexa_formentry form_table[];
extern int form_count;

int hex_validate(hex_header_t *hdr) {
    if (!hdr) return 0;
    if (hdr->magic != HEX_MAGIC) return 0;
    uint32_t ck = 0;
    uint8_t *p = (uint8_t *)hdr;
    for (uint32_t i = 0; i < sizeof(hex_header_t) - 4; i++)
        ck = ((ck << 5) + ck) + p[i];
    if (ck != hdr->checksum) return 0;
    return 1;
}

int hex_load(hex_header_t *hdr, uint32_t *entry, uint32_t *heap_start) {
    if (!hex_validate(hdr)) return -1;

    uint32_t caller_pid = (uint32_t)(current_task >= 0 ? tasks[current_task].pid : 0);

    for (uint32_t i = 0; i < hdr->cap_count && i < 16; i++) {
        uint32_t needed_cap = hdr->caps[i];
        if (!hexafs_cap_check(caller_pid, needed_cap)) {
            log_write(LOG_LEVEL_WARN, "hex: missing capability for binary");
            print_color("[HEX] Missing cap 0x", 0x0C);
            char buf[16];
            itoa((int)needed_cap, buf, 16);
            print_string(buf);
            print_string(" - refusing to load\n");
            return -1;
        }
    }

    for (uint32_t i = 0; i < hdr->dep_count && i < 16; i++) {
        uint32_t dep_hash = hdr->dep_hashes[i];
        (void)dep_hash;
    }

    if (entry) *entry = hdr->entry_point;
    if (heap_start) *heap_start = 0x40000000;

    char buf[64];
    itoa((int)hdr->input_schema_hash, buf, 16);
    log_write(LOG_LEVEL_INFO, "hex: binary loaded");

    return 0;
}

int hex_pack_elf(const char *elf_name, const char *caps_str, uint32_t in_schema, uint32_t out_schema) {
    if (!elf_name || !elf_name[0]) {
        print_string("hexpack: no ELF name specified\n");
        return -1;
    }
    int idx = find_form(elf_name);
    if (idx < 0) {
        print_string("hexpack: ELF form '");
        print_string(elf_name);
        print_string("' not found\n");
        return -1;
    }

    hex_header_t hdr;
    memset(&hdr, 0, sizeof(hdr));
    hdr.magic = HEX_MAGIC;
    hdr.version = 1;
    hdr.entry_point = 0;
    hdr.code_offset = sizeof(hex_header_t);
    if (form_table[idx].content && form_table[idx].size >= 4) {
        uint32_t *elf_entry = (uint32_t *)(form_table[idx].content + 24);
        hdr.entry_point = *elf_entry;
    }
    hdr.code_size = (uint32_t)form_table[idx].size;
    hdr.input_schema_hash = in_schema;
    hdr.output_schema_hash = out_schema;

    int cap_idx = 0;
    if (caps_str && caps_str[0]) {
        int i = 0;
        while (caps_str[i] && cap_idx < 16) {
            while (caps_str[i] == ' ') i++;
            if (!caps_str[i]) break;
            uint32_t val = 0;
            while (caps_str[i] >= '0' && caps_str[i] <= '9') {
                val = val * 10 + (uint32_t)(caps_str[i] - '0');
                i++;
            }
            hdr.caps[cap_idx++] = val;
            while (caps_str[i] == ' ') i++;
            if (caps_str[i] == ',') i++;
        }
    }
    hdr.cap_count = (uint32_t)cap_idx;

    uint32_t ck = 0;
    uint8_t *p = (uint8_t *)&hdr;
    for (uint32_t bi = 0; bi < sizeof(hex_header_t) - 4; bi++)
        ck = ((ck << 5) + ck) + p[bi];
    hdr.checksum = ck;

    int len = (int)(sizeof(hex_header_t) + (uint32_t)form_table[idx].size);
    char *packed = (char *)kmalloc((uint32_t)(len + 1));
    if (!packed) {
        print_string("hexpack: allocation failed\n");
        return -1;
    }
    memcpy(packed, &hdr, sizeof(hex_header_t));
    if (form_table[idx].content && form_table[idx].size > 0)
        memcpy(packed + sizeof(hex_header_t), form_table[idx].content, (uint32_t)form_table[idx].size);

    char hex_name[32];
    int ni = 0;
    while (elf_name[ni] && ni < 24) { hex_name[ni] = elf_name[ni]; ni++; }
    hex_name[ni] = '.';
    hex_name[ni+1] = 'h';
    hex_name[ni+2] = 'e';
    hex_name[ni+3] = 'x';
    hex_name[ni+4] = 0;

    cmd_mkform(hex_name);
    int hidx = find_form(hex_name);
    if (hidx >= 0) {
        kfree(form_table[hidx].content);
        form_table[hidx].content = packed;
        form_table[hidx].size = len;
        form_table[hidx].cap = len;
    }

    print_string("hexpack: wrapped ");
    print_string(elf_name);
    print_string(" -> ");
    print_string(hex_name);
    print_string("\n");
    return 0;
}
