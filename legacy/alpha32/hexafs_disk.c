#include "types.h"
#include "hexafs_disk.h"
#include "hexafs.h"
#include "log.h"

extern int ata_read_sector(uint32_t lba, uint16_t *buf);
extern int ata_write_sector(uint32_t lba, const uint16_t *buf);
extern void *memset(void *dest, int c, size_t len);
extern void *memcpy(void *dest, const void *src, size_t len);
extern int memcmp(const void *s1, const void *s2, size_t n);

static uint8_t alloc_bitmap[HEXAFS_ALLOC_BLOCKS * HEXAFS_BLOCK_SIZE];
static int alloc_bitmap_dirty = 0;
static int next_alloc_hint = HEXAFS_OBJECT_LBA;
int hexafs_mounted = 0;
hexafs_superblock_t hexafs_sb;

hexafs_superblock_t sb_cache;
static int sb_valid = 0;

static hexafs_cache_slot_t block_cache[HEXAFS_CACHE_SLOTS];

static int cache_find(uint32_t lba) {
    for (int i = 0; i < HEXAFS_CACHE_SLOTS; i++)
        if (block_cache[i].lba == lba) return i;
    return -1;
}

static int cache_evict(void) {
    for (int i = 0; i < HEXAFS_CACHE_SLOTS; i++) {
        if (!block_cache[i].lba) return i;
    }
    return 0;
}

static void cache_flush_dirty(void) {
    for (int i = 0; i < HEXAFS_CACHE_SLOTS; i++) {
        if (block_cache[i].lba && block_cache[i].dirty) {
            uint16_t tmp[256];
            memcpy(tmp, block_cache[i].data, HEXAFS_BLOCK_SIZE);
            ata_write_sector(block_cache[i].lba, tmp);
            block_cache[i].dirty = 0;
        }
    }
}

static int cache_read(uint32_t lba, void *buf) {
    int idx = cache_find(lba);
    if (idx >= 0) {
        memcpy(buf, block_cache[idx].data, HEXAFS_BLOCK_SIZE);
        return 1;
    }
    uint16_t tmp[256];
    if (!ata_read_sector(lba, tmp)) return 0;
    idx = cache_evict();
    block_cache[idx].lba = lba;
    block_cache[idx].dirty = 0;
    memcpy(block_cache[idx].data, tmp, HEXAFS_BLOCK_SIZE);
    memcpy(buf, tmp, HEXAFS_BLOCK_SIZE);
    return 1;
}

static int cache_write(uint32_t lba, const void *buf) {
    int idx = cache_find(lba);
    if (idx < 0) {
        idx = cache_evict();
        block_cache[idx].lba = lba;
    }
    block_cache[idx].dirty = 1;
    memcpy(block_cache[idx].data, buf, HEXAFS_BLOCK_SIZE);
    uint16_t tmp[256];
    memcpy(tmp, buf, HEXAFS_BLOCK_SIZE);
    return ata_write_sector(lba, tmp);
}

static int bitmap_test(int block) {
    int byte_idx = block / 8;
    int bit_idx = block % 8;
    if (byte_idx >= (int)sizeof(alloc_bitmap)) return 1;
    return (alloc_bitmap[byte_idx] >> bit_idx) & 1;
}

static void bitmap_set(int block) {
    int byte_idx = block / 8;
    int bit_idx = block % 8;
    if (byte_idx >= (int)sizeof(alloc_bitmap)) return;
    alloc_bitmap[byte_idx] |= (1 << bit_idx);
    alloc_bitmap_dirty = 1;
}

static void bitmap_clear(int block) {
    int byte_idx = block / 8;
    int bit_idx = block % 8;
    if (byte_idx >= (int)sizeof(alloc_bitmap)) return;
    alloc_bitmap[byte_idx] &= ~(1 << bit_idx);
    alloc_bitmap_dirty = 1;
}

uint32_t hexafs_content_hash(const void *data, uint32_t size) {
    uint32_t h = 5381;
    const uint8_t *p = (const uint8_t *)data;
    for (uint32_t i = 0; i < size; i++)
        h = ((h << 5) + h) + p[i];
    return h;
}

int hexafs_block_read(uint32_t lba, void *buf) {
    return cache_read(lba, buf);
}

int hexafs_block_write(uint32_t lba, const void *buf) {
    return cache_write(lba, buf);
}

int hexafs_block_verify(uint32_t lba, const void *buf, uint32_t stored_crc) {
    (void)lba;
    uint32_t computed = hexafs_content_hash(buf, HEXAFS_BLOCK_SIZE);
    if (computed != stored_crc) {
        log_write(LOG_LEVEL_WARN, "HEXAFS: CRC mismatch on block");
        return 0;
    }
    return 1;
}

static uint32_t obj_checksum(hexafs_object_t *obj) {
    uint32_t saved = obj->checksum;
    obj->checksum = 0;
    uint32_t cksum = hexafs_content_hash(obj, sizeof(hexafs_object_t));
    obj->checksum = saved;
    return cksum;
}

uint32_t hexafs_alloc_block(void) {
    for (int i = next_alloc_hint; i < HEXAFS_DISK_BLOCKS; i++) {
        if (!bitmap_test(i)) {
            bitmap_set(i);
            uint8_t zero[HEXAFS_BLOCK_SIZE];
            memset(zero, 0, sizeof(zero));
            hexafs_block_write(i, zero);
            next_alloc_hint = i + 1;
            return (uint32_t)i;
        }
    }
    for (int i = HEXAFS_OBJECT_LBA; i < next_alloc_hint; i++) {
        if (!bitmap_test(i)) {
            bitmap_set(i);
            uint8_t zero[HEXAFS_BLOCK_SIZE];
            memset(zero, 0, sizeof(zero));
            hexafs_block_write(i, zero);
            next_alloc_hint = i + 1;
            return (uint32_t)i;
        }
    }
    log_write(LOG_LEVEL_WARN, "HEXAFS: out of disk blocks");
    return 0;
}

void hexafs_free_block(uint32_t lba) {
    if (lba < HEXAFS_OBJECT_LBA || lba >= HEXAFS_DISK_BLOCKS) return;
    bitmap_clear(lba);
    if (lba < (uint32_t)next_alloc_hint) next_alloc_hint = (int)lba;
}

static int bitmap_init(void) {
    memset(alloc_bitmap, 0, sizeof(alloc_bitmap));
    for (int i = 0; i < HEXAFS_OBJECT_LBA; i++)
        bitmap_set(i);
    for (int i = HEXAFS_ALLOC_LBA; i < HEXAFS_ALLOC_LBA + HEXAFS_ALLOC_BLOCKS; i++)
        bitmap_set(i);
    alloc_bitmap_dirty = 1;
    return 1;
}

int hexafs_save_bitmap(void) {
    if (!alloc_bitmap_dirty) return 1;
    uint8_t buf[HEXAFS_BLOCK_SIZE];
    for (int i = 0; i < HEXAFS_ALLOC_BLOCKS; i++) {
        memset(buf, 0, sizeof(buf));
        memcpy(buf, alloc_bitmap + i * HEXAFS_BLOCK_SIZE, HEXAFS_BLOCK_SIZE);
        if (!hexafs_block_write(HEXAFS_ALLOC_LBA + i, buf)) return 0;
    }
    alloc_bitmap_dirty = 0;
    return 1;
}

static int bitmap_load(void) {
    uint8_t buf[HEXAFS_BLOCK_SIZE];
    for (int i = 0; i < HEXAFS_ALLOC_BLOCKS; i++) {
        if (!hexafs_block_read(HEXAFS_ALLOC_LBA + i, buf)) return 0;
        memcpy(alloc_bitmap + i * HEXAFS_BLOCK_SIZE, buf, HEXAFS_BLOCK_SIZE);
    }
    alloc_bitmap_dirty = 0;
    return 1;
}

static uint32_t sb_checksum(hexafs_superblock_t *sb) {
    uint32_t saved = sb->checksum;
    sb->checksum = 0;
    uint32_t cksum = hexafs_content_hash(sb, sizeof(hexafs_superblock_t));
    sb->checksum = saved;
    return cksum;
}

int hexafs_write_superblock(void) {
    hexafs_superblock_t sb;
    memset(&sb, 0, sizeof(sb));
    memcpy(sb.magic, HEXAFS_SUPER_MAGIC, 8);
    sb.version = 1;
    sb.total_blocks = HEXAFS_DISK_BLOCKS;
    sb.allocator_lba = HEXAFS_ALLOC_LBA;
    sb.allocator_blocks = HEXAFS_ALLOC_BLOCKS;
    sb.object_store_lba = HEXAFS_OBJECT_LBA;
    sb.root_snap_block = sb_cache.root_snap_block;
    sb.timestamp = sb_cache.timestamp;
    sb.format_gen = sb_cache.format_gen + 1;
    sb.checksum = sb_checksum(&sb);
    sb_cache = sb;
    sb_valid = 1;
    return hexafs_block_write(HEXAFS_SUPER_LBA, &sb);
}

int hexafs_read_superblock(void) {
    hexafs_superblock_t sb;
    if (!hexafs_block_read(HEXAFS_SUPER_LBA, &sb)) return 0;
    if (memcmp(sb.magic, HEXAFS_SUPER_MAGIC, 8) != 0) return 0;
    uint32_t ck = sb_checksum(&sb);
    if (ck != sb.checksum) {
        log_write(LOG_LEVEL_WARN, "HEXAFS: superblock CRC mismatch");
        return 0;
    }
    sb_cache = sb;
    sb_valid = 1;
    return 1;
}

int hexafs_format(void) {
    memset(&sb_cache, 0, sizeof(sb_cache));
    sb_cache.timestamp = 0;
    sb_cache.format_gen = 1;
    sb_cache.root_snap_block = 0;
    memcpy(sb_cache.magic, HEXAFS_SUPER_MAGIC, 8);
    sb_cache.version = 1;
    sb_cache.total_blocks = HEXAFS_DISK_BLOCKS;
    sb_cache.allocator_lba = HEXAFS_ALLOC_LBA;
    sb_cache.allocator_blocks = HEXAFS_ALLOC_BLOCKS;
    sb_cache.object_store_lba = HEXAFS_OBJECT_LBA;
    memset(block_cache, 0, sizeof(block_cache));
    bitmap_init();
    if (!hexafs_write_superblock()) return 0;
    if (!hexafs_save_bitmap()) return 0;
    hexafs_mounted = 1;
    log_write(LOG_LEVEL_INFO, "HEXAFS: formatted");
    return 1;
}

int hexafs_mount_disk(void) {
    memset(block_cache, 0, sizeof(block_cache));
    if (!hexafs_read_superblock()) {
        log_write(LOG_LEVEL_WARN, "HEXAFS: no superblock, need format");
        return 0;
    }
    if (!bitmap_load()) {
        log_write(LOG_LEVEL_WARN, "HEXAFS: bitmap load failed");
        return 0;
    }
    hexafs_mounted = 1;
    log_write(LOG_LEVEL_INFO, "HEXAFS: mounted");
    return 1;
}

uint32_t hexafs_object_alloc(uint8_t type) {
    uint32_t block = hexafs_alloc_block();
    if (!block) return 0;
    hexafs_object_t obj;
    memset(&obj, 0, sizeof(obj));
    obj.magic = HEXAFS_OBJECT_TYPE(type);
    obj.type = type;
    obj.timestamp = sb_cache.timestamp;
    obj.checksum = obj_checksum(&obj);
    if (!hexafs_block_write(block, &obj)) {
        hexafs_free_block(block);
        return 0;
    }
    return block;
}

int hexafs_object_write_data(uint32_t obj_block, const void *data, uint32_t size) {
    hexafs_object_t obj;
    if (!hexafs_block_read(obj_block, &obj)) return 0;
    if ((obj.magic & 0xFFFFFF00) != HEXAFS_OBJ_MAGIC_BASE) return 0;
    uint32_t ck = obj_checksum(&obj);
    if (ck != obj.checksum) {
        log_write(LOG_LEVEL_WARN, "HEXAFS: object write CRC fail (self-healing)");
        obj.checksum = ck;
    }
    uint32_t blocks[4] = {0};
    blocks[0] = obj.content_block;
    blocks[1] = obj.content_blocks_extra[0];
    blocks[2] = obj.content_blocks_extra[1];
    blocks[3] = obj.content_blocks_extra[2];
    uint32_t remaining = size;
    uint32_t offset = 0;
    uint8_t block_buf[HEXAFS_BLOCK_SIZE];
    int nblocks = (size + HEXAFS_BLOCK_SIZE - 1) / HEXAFS_BLOCK_SIZE;
    if (nblocks > 4) nblocks = 4;
    for (int i = 0; i < nblocks; i++) {
        uint32_t db = blocks[i];
        if (!db) {
            db = hexafs_alloc_block();
            if (!db) return 0;
        }
        memset(block_buf, 0, sizeof(block_buf));
        uint32_t copy = remaining < HEXAFS_BLOCK_SIZE ? remaining : HEXAFS_BLOCK_SIZE;
        memcpy(block_buf, (const uint8_t*)data + offset, copy);
        if (!hexafs_block_write(db, block_buf)) { if (i > 0 && db != blocks[i]) hexafs_free_block(db); return 0; }
        if (i == 0) obj.content_block = db;
        else obj.content_blocks_extra[i - 1] = db;
        offset += copy;
        remaining -= copy;
    }
    for (int i = nblocks; i < 4; i++) {
        if (i == 0) obj.content_block = 0;
        else obj.content_blocks_extra[i - 1] = 0;
    }
    obj.content_size = size;
    obj.content_hash = hexafs_content_hash(data, size);
    obj.checksum = obj_checksum(&obj);
    if (!hexafs_block_write(obj_block, &obj)) return 0;
    return 1;
}

int hexafs_object_read_data(uint32_t obj_block, void *buf, uint32_t *size, uint8_t *type) {
    hexafs_object_t obj;
    if (!hexafs_block_read(obj_block, &obj)) return 0;
    if ((obj.magic & 0xFFFFFF00) != HEXAFS_OBJ_MAGIC_BASE) return 0;
    uint32_t ck = obj_checksum(&obj);
    if (ck != obj.checksum) {
        obj.checksum = ck;
    }
    if (type) *type = obj.type;
    if (size) *size = obj.content_size;
    if (buf && obj.content_block && obj.content_size > 0) {
        uint32_t remaining = obj.content_size;
        uint32_t offset = 0;
        uint32_t db[4] = {obj.content_block, obj.content_blocks_extra[0], obj.content_blocks_extra[1], obj.content_blocks_extra[2]};
        for (int i = 0; i < 4 && remaining > 0; i++) {
            if (!db[i]) break;
            uint8_t block_buf[HEXAFS_BLOCK_SIZE];
            if (!hexafs_block_read(db[i], block_buf)) return 0;
            uint32_t copy = remaining < HEXAFS_BLOCK_SIZE ? remaining : HEXAFS_BLOCK_SIZE;
            memcpy((uint8_t*)buf + offset, block_buf, copy);
            offset += copy;
            remaining -= copy;
        }
    }
    return 1;
}

uint32_t hexafs_snap_create(const char *name) {
    if (!hexafs_current_tx.active || !hexafs_current_tx.dirty) return 0;
    uint32_t snap_block = hexafs_alloc_block();
    if (!snap_block) return 0;
    hexafs_snap_t snap;
    memset(&snap, 0, sizeof(snap));
    snap.magic = HEXAFS_SNAP_MAGIC;
    snap.parent_snap_block = sb_cache.root_snap_block;
    snap.timestamp = sb_cache.timestamp;
    snap.root_object_block = hexafs_current_tx.new_root_block;
    if (name) {
        int i = 0;
        while (name[i] && i < 31) { snap.name[i] = name[i]; i++; }
        snap.name[i] = 0;
    }
    snap.checksum = hexafs_content_hash(&snap, sizeof(snap));
    if (!hexafs_block_write(snap_block, &snap)) {
        hexafs_free_block(snap_block);
        return 0;
    }
    hexafs_current_tx.snap_block = snap_block;
    sb_cache.root_snap_block = snap_block;
    return snap_block;
}

uint32_t hexafs_snap_find(const char *name) {
    uint32_t snap_block = sb_cache.root_snap_block;
    while (snap_block) {
        hexafs_snap_t snap;
        if (!hexafs_block_read(snap_block, &snap)) return 0;
        if (snap.magic != HEXAFS_SNAP_MAGIC) return 0;
        if (name && name[0]) {
            int match = 1;
            for (int i = 0; i < 32; i++) {
                if (snap.name[i] != name[i]) { match = 0; break; }
                if (name[i] == 0) break;
            }
            if (match) return snap_block;
        }
        snap_block = snap.parent_snap_block;
    }
    return 0;
}

static uint32_t snap_timestamp = 0;
uint32_t hexafs_get_timestamp(void) {
    return ++snap_timestamp;
}

static uint32_t abs_checksum(hexafs_abs_header_t *hdr) {
    uint32_t saved = hdr->checksum;
    hdr->checksum = 0;
    uint32_t cksum = hexafs_content_hash(hdr, sizeof(hexafs_abs_header_t));
    hdr->checksum = saved;
    return cksum;
}

uint32_t hexafs_abstraction_create(void) {
    uint32_t block = hexafs_alloc_block();
    if (!block) return 0;
    hexafs_abs_header_t hdr;
    memset(&hdr, 0, sizeof(hdr));
    hdr.magic = HEXAFS_ABSTRACT_MAGIC;
    hdr.entry_count = 0;
    hdr.checksum = abs_checksum(&hdr);
    if (!hexafs_block_write(block, &hdr)) {
        hexafs_free_block(block);
        return 0;
    }
    return block;
}

static uint32_t hexafs_abstraction_next_chain(uint32_t abs_block) {
    uint8_t buf[HEXAFS_BLOCK_SIZE];
    if (!hexafs_block_read(abs_block, buf)) return 0;
    hexafs_abs_header_t *hdr = (hexafs_abs_header_t *)buf;
    return hdr->pad[0] | ((uint32_t)hdr->pad[1] << 8) | ((uint32_t)hdr->pad[2] << 16) | ((uint32_t)hdr->pad[3] << 24);
}

static void hexafs_abstraction_set_chain(uint32_t abs_block, uint32_t next_block) {
    uint8_t buf[HEXAFS_BLOCK_SIZE];
    if (!hexafs_block_read(abs_block, buf)) return;
    hexafs_abs_header_t *hdr = (hexafs_abs_header_t *)buf;
    hdr->pad[0] = next_block & 0xFF;
    hdr->pad[1] = (next_block >> 8) & 0xFF;
    hdr->pad[2] = (next_block >> 16) & 0xFF;
    hdr->pad[3] = (next_block >> 24) & 0xFF;
    hexafs_block_write(abs_block, buf);
}

int hexafs_abstraction_add_entry(uint32_t abs_block, const char *name, uint32_t obj_block, uint8_t type) {
    uint32_t cur = abs_block;
    while (cur) {
        uint8_t buf[HEXAFS_BLOCK_SIZE];
        if (!hexafs_block_read(cur, buf)) return 0;
        hexafs_abs_header_t *hdr = (hexafs_abs_header_t *)buf;
        uint32_t ck = abs_checksum(hdr);
        if (ck != hdr->checksum) {
            log_write(LOG_LEVEL_WARN, "HEXAFS: abstraction CRC fail");
            return 0;
        }
        if (hdr->magic != HEXAFS_ABSTRACT_MAGIC) return 0;
        if (hdr->entry_count < HEXAFS_ABS_MAX_ENTRIES) {
            int count = hdr->entry_count;
            int i = 0;
            while (name[i] && i < 31) { hdr->entries[count].name[i] = name[i]; i++; }
            hdr->entries[count].name[i] = 0;
            hdr->entries[count].object_block = obj_block;
            hdr->entries[count].type = type;
            hdr->entry_count = count + 1;
            hdr->checksum = abs_checksum(hdr);
            return hexafs_block_write(cur, buf);
        }
        uint32_t next = hexafs_abstraction_next_chain(cur);
        if (!next) break;
        cur = next;
    }
    uint32_t new_block = hexafs_abstraction_create();
    if (!new_block) return 0;
    hexafs_abstraction_set_chain(cur, new_block);
    uint8_t buf[HEXAFS_BLOCK_SIZE];
    if (!hexafs_block_read(new_block, buf)) return 0;
    hexafs_abs_header_t *hdr = (hexafs_abs_header_t *)buf;
    int i = 0;
    while (name[i] && i < 31) { hdr->entries[0].name[i] = name[i]; i++; }
    hdr->entries[0].name[i] = 0;
    hdr->entries[0].object_block = obj_block;
    hdr->entries[0].type = type;
    hdr->entry_count = 1;
    hdr->checksum = abs_checksum(hdr);
    return hexafs_block_write(new_block, buf);
}

int hexafs_abstraction_find(uint32_t abs_block, const char *name, uint32_t *obj_block, uint8_t *type) {
    uint32_t cur = abs_block;
    while (cur) {
        uint8_t buf[HEXAFS_BLOCK_SIZE];
        if (!hexafs_block_read(cur, buf)) return 0;
        hexafs_abs_header_t *hdr = (hexafs_abs_header_t *)buf;
        if (hdr->magic != HEXAFS_ABSTRACT_MAGIC) return 0;
        for (int i = 0; i < (int)hdr->entry_count; i++) {
            int match = 1;
            for (int j = 0; j < 32; j++) {
                if (hdr->entries[i].name[j] != name[j]) { match = 0; break; }
                if (name[j] == 0) break;
            }
            if (match) {
                if (obj_block) *obj_block = hdr->entries[i].object_block;
                if (type) *type = hdr->entries[i].type;
                return 1;
            }
        }
        cur = hexafs_abstraction_next_chain(cur);
    }
    return 0;
}

int hexafs_abstraction_remove_entry(uint32_t abs_block, const char *name) {
    uint32_t cur = abs_block;
    while (cur) {
        uint8_t buf[HEXAFS_BLOCK_SIZE];
        if (!hexafs_block_read(cur, buf)) return 0;
        hexafs_abs_header_t *hdr = (hexafs_abs_header_t *)buf;
        uint32_t ck = abs_checksum(hdr);
        if (ck != hdr->checksum) return 0;
        if (hdr->magic != HEXAFS_ABSTRACT_MAGIC) return 0;
        int found = -1;
        for (int i = 0; i < (int)hdr->entry_count; i++) {
            int match = 1;
            for (int j = 0; j < 32; j++) {
                if (hdr->entries[i].name[j] != name[j]) { match = 0; break; }
                if (name[j] == 0) break;
            }
            if (match) { found = i; break; }
        }
        if (found >= 0) {
            for (int i = found; i < (int)hdr->entry_count - 1; i++)
                hdr->entries[i] = hdr->entries[i + 1];
            hdr->entry_count--;
            hdr->checksum = abs_checksum(hdr);
            return hexafs_block_write(cur, buf);
        }
        cur = hexafs_abstraction_next_chain(cur);
    }
    return 0;
}

int hexafs_abstraction_list(uint32_t abs_block, void (*cb)(const char *name, uint32_t obj_block, uint8_t type)) {
    int total = 0;
    uint32_t cur = abs_block;
    while (cur) {
        uint8_t buf[HEXAFS_BLOCK_SIZE];
        if (!hexafs_block_read(cur, buf)) return total;
        hexafs_abs_header_t *hdr = (hexafs_abs_header_t *)buf;
        if (hdr->magic != HEXAFS_ABSTRACT_MAGIC) return total;
        for (int i = 0; i < (int)hdr->entry_count; i++) {
            if (cb) cb(hdr->entries[i].name, hdr->entries[i].object_block, hdr->entries[i].type);
            total++;
        }
        cur = hexafs_abstraction_next_chain(cur);
    }
    return total;
}

int hexafs_cache_flush_all(void) {
    cache_flush_dirty();
    if (alloc_bitmap_dirty) {
        if (!hexafs_save_bitmap()) return 0;
    }
    return 1;
}
