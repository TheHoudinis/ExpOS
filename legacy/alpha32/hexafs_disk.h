#ifndef HEXAFS_DISK_H
#define HEXAFS_DISK_H

#include "types.h"

#define HEXAFS_BLOCK_SIZE      512
#define HEXAFS_DISK_BLOCKS     4096
#define HEXAFS_MAX_FORMS       64

#define HEXAFS_SUPER_MAGIC     "HEXAFS0"
#define HEXAFS_SNAP_MAGIC      0x534E4150
#define HEXAFS_OBJ_MAGIC_BASE  0x4F424A00
#define HEXAFS_JOURNAL_MAGIC   0x4C475458
#define HEXAFS_ABSTRACT_MAGIC  0x42534241

#define HEXAFS_SUPER_LBA       100
#define HEXAFS_ALLOC_LBA       101
#define HEXAFS_ALLOC_BLOCKS    2
#define HEXAFS_ROOT_SNAP_LBA   103
#define HEXAFS_OBJECT_LBA      109

#define HEXAFS_OBJECT_TYPE(t)  (HEXAFS_OBJ_MAGIC_BASE | ((t) & 0xFF))

#define HEXAFS_FORM            0x01
#define HEXAFS_ABSTRACTION     0x02
#define HEXAFS_ALIAS          0x03
#define HEXAFS_PROCSTATE       0x04
#define HEXAFS_CONFIG          0x05
#define HEXAFS_SNAPSHOT        0x06
#define HEXAFS_CAPABILITY      0x07
#define HEXAFS_EVENT           0x08

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
} hexafs_superblock_t;

typedef struct __attribute__((packed)) {
    uint32_t magic;
    uint32_t parent_snap_block;
    uint32_t root_object_block;
    uint32_t timestamp;
    char     name[32];
    uint32_t checksum;
    uint8_t  pad[460];
} hexafs_snap_t;

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
} hexafs_object_t;

typedef struct __attribute__((packed)) {
    uint32_t magic;
    uint32_t state;
    uint32_t gen;
    uint32_t new_snap_block;
    uint32_t root_snap_saved;
    uint8_t  pad[492];
} hexafs_journal_t;

#define HEXAFS_ABS_ENTRY_SIZE  44
typedef struct __attribute__((packed)) {
    char     name[32];
    uint32_t object_block;
    uint8_t  type;
    uint8_t  pad[7];
} hexafs_abs_entry_t;

#define HEXAFS_ABS_MAX_ENTRIES  11
#define HEXAFS_ABS_CHAIN_MAX    8

#define HEXAFS_CACHE_SLOTS      16
#define HEXAFS_JOURNAL_BLOCKS   4
#define HEXAFS_NEXT_ALLOC_LBA   200

typedef struct __attribute__((packed)) {
    uint32_t lba;
    uint8_t  dirty;
    uint8_t  data[HEXAFS_BLOCK_SIZE];
} hexafs_cache_slot_t;

typedef struct __attribute__((packed)) {
    uint32_t magic;
    uint32_t entry_count;
    uint32_t checksum;
    hexafs_abs_entry_t entries[HEXAFS_ABS_MAX_ENTRIES];
    uint8_t pad[16];
} hexafs_abs_header_t;

int hexafs_cache_flush_all(void);

#endif
