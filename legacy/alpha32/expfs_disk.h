#ifndef EXPFS_DISK_H
#define EXPFS_DISK_H

#include "types.h"

#define EXPFS_BLOCK_SIZE      512
#define EXPFS_DISK_BLOCKS     4096
#define EXPFS_MAX_FORMS       64

#define EXPFS_SUPER_MAGIC     "HEXAFS0" /* legacy on-disk signature, preserved for compatibility */
#define EXPFS_SNAP_MAGIC      0x534E4150
#define EXPFS_OBJ_MAGIC_BASE  0x4F424A00
#define EXPFS_JOURNAL_MAGIC   0x4C475458
#define EXPFS_ABSTRACT_MAGIC  0x42534241

#define EXPFS_SUPER_LBA       100
#define EXPFS_ALLOC_LBA       101
#define EXPFS_ALLOC_BLOCKS    2
#define EXPFS_ROOT_SNAP_LBA   103
#define EXPFS_OBJECT_LBA      109

#define EXPFS_OBJECT_TYPE(t)  (EXPFS_OBJ_MAGIC_BASE | ((t) & 0xFF))

#define EXPFS_FORM            0x01
#define EXPFS_ABSTRACTION     0x02
#define EXPFS_ALIAS          0x03
#define EXPFS_PROCSTATE       0x04
#define EXPFS_CONFIG          0x05
#define EXPFS_SNAPSHOT        0x06
#define EXPFS_CAPABILITY      0x07
#define EXPFS_EVENT           0x08

#define CAP_TYPE_ROOT          0x00000001
#define CAP_TYPE_INTENT        0x00000002
#define CAP_TYPE_REPLAY_WRITE  0x00000004
#define CAP_TYPE_SNAP_CREATE   0x00000008
#define CAP_TYPE_GRANT_AUTH    0x00000010
#define CAP_TYPE_SEND_TO_PID   0x00000100
#define CAP_TYPE_RECV_TERMINATE 0x00000200
#define CAP_TYPE_RECV_PAUSE    0x00000400
#define CAP_TYPE_NET_ADMIN     0x00001000
#define CAP_TYPE_BOOT_POLICY   0x00002000

typedef struct __attribute__((packed)) {
    uint32_t cap_type;
    uint32_t grantee_snap;
    uint32_t grantor_snap;
    uint32_t expires_tick;
    uint32_t delegatable;
    uint32_t grant_block_hash;
} cap_grant_t;

typedef struct __attribute__((packed)) {
    uint32_t cap_type;
    uint32_t grant_hash;
    uint32_t revoked_by_snap;
    uint32_t timestamp;
} cap_revoke_t;

typedef struct __attribute__((packed)) {
    char     magic[8];
    uint32_t version;
    uint32_t total_blocks;
    uint32_t allocator_lba;
    uint32_t allocator_blocks;
    uint32_t object_store_lba;
    uint32_t root_snap_block;
    uint32_t checksum;
    uint32_t timestamp;
    uint32_t format_gen;
    uint8_t  pad[468];
} expfs_superblock_t;

typedef struct __attribute__((packed)) {
    uint32_t magic;
    uint32_t parent_snap_block;
    uint32_t root_object_block;
    uint32_t timestamp;
    char     name[32];
    uint32_t checksum;
    uint8_t  pad[460];
} expfs_snap_t;

typedef struct __attribute__((packed)) {
    uint32_t magic;
    uint8_t  type;
    uint8_t  pad1[3];
    uint32_t schema_hash;
    uint32_t content_size;
    uint32_t content_block;
    uint32_t content_blocks_extra[3];
    uint32_t content_hash;
    uint32_t parent_snap;
    uint32_t cap_blocks[4];
    uint32_t timestamp;
    uint32_t checksum;
    uint8_t  pad2[448];
} expfs_object_t;

typedef struct __attribute__((packed)) {
    uint32_t magic;
    uint32_t state;
    uint32_t gen;
    uint32_t new_snap_block;
    uint32_t root_snap_saved;
    uint8_t  pad[492];
} expfs_journal_t;

#define EXPFS_ABS_ENTRY_SIZE  44
typedef struct __attribute__((packed)) {
    char     name[32];
    uint32_t object_block;
    uint8_t  type;
    uint8_t  pad[7];
} expfs_abs_entry_t;

#define EXPFS_ABS_MAX_ENTRIES  11
#define EXPFS_ABS_CHAIN_MAX    8

#define EXPFS_CACHE_SLOTS      16
#define EXPFS_JOURNAL_BLOCKS   4
#define EXPFS_NEXT_ALLOC_LBA   200

typedef struct __attribute__((packed)) {
    uint32_t lba;
    uint8_t  dirty;
    uint8_t  data[EXPFS_BLOCK_SIZE];
} expfs_cache_slot_t;

typedef struct __attribute__((packed)) {
    uint32_t magic;
    uint32_t entry_count;
    uint32_t checksum;
    expfs_abs_entry_t entries[EXPFS_ABS_MAX_ENTRIES];
    uint8_t pad[16];
} expfs_abs_header_t;

int expfs_cache_flush_all(void);

#endif
