#ifndef HEXAFS_H
#define HEXAFS_H

#include "types.h"
#include "hexafs_disk.h"
#include "vfs.h"

typedef struct {
    int      active;
    int      tid;
    uint32_t parent_snap_block;
    uint32_t new_root_block;
    uint32_t snap_block;
    uint32_t journal_block;
    int      dirty;
} hexafs_tx_t;

extern int hexafs_mounted;
extern hexafs_tx_t hexafs_current_tx;
extern hexafs_superblock_t sb_cache;

int hexafs_mount_disk(void);
int hexafs_write_superblock(void);
int hexafs_save_bitmap(void);
uint32_t hexafs_abstraction_create(void);
int hexafs_abstraction_add_entry(uint32_t abs_block, const char *name, uint32_t obj_block, uint8_t type);
int hexafs_abstraction_find(uint32_t abs_block, const char *name, uint32_t *obj_block, uint8_t *type);
int hexafs_abstraction_remove_entry(uint32_t abs_block, const char *name);
int hexafs_abstraction_list(uint32_t abs_block, void (*cb)(const char *name, uint32_t obj_block, uint8_t type));

int hexafs_mount(void);
int hexafs_format(void);
int hexafs_tx_begin(void);
int hexafs_tx_commit(void);
int hexafs_tx_abort(void);

uint32_t hexafs_content_hash(const void *data, uint32_t size);
uint32_t hexafs_alloc_block(void);
void hexafs_free_block(uint32_t lba);
int hexafs_block_read(uint32_t lba, void *buf);
int hexafs_block_write(uint32_t lba, const void *buf);
int hexafs_block_verify(uint32_t lba, const void *buf, uint32_t stored_crc);

uint32_t hexafs_object_alloc(uint8_t type);
int hexafs_object_write_data(uint32_t obj_block, const void *data, uint32_t size);
int hexafs_object_read_data(uint32_t obj_block, void *buf, uint32_t *size, uint8_t *type);

uint32_t hexafs_snap_create(const char *name);
uint32_t hexafs_snap_find(const char *name);

int hexafs_vfs_open(const char *path, int flags);
int hexafs_vfs_read(int obj_id, char *buf, int count, int pos);
int hexafs_vfs_write(int obj_id, const char *buf, int count, int pos);
int hexafs_vfs_stat(const char *path, struct vfs_node *node, uint32_t *obj_id);
int hexafs_vfs_close(int obj_id);
int hexafs_vfs_list(void (*cb)(const char *name, uint32_t obj_block, uint8_t type));
void hexafs_save_all(void);
void hexafs_load_all(void);

int hexafs_cap_grant(uint32_t grantee_pid, uint32_t cap_type, uint32_t expires_tick, int delegatable);
int hexafs_cap_check(uint32_t pid, uint32_t cap_type);
int hexafs_cap_revoke(uint32_t grant_hash);
int hexafs_cap_list_pid(uint32_t pid, char *out, int out_len);
uint32_t hexafs_snap_for_pid(uint32_t pid);

#define PIMP_RULE_MAX 32
#define PIMP_NAME_MAX 32

typedef struct {
    char user[PIMP_NAME_MAX];
    uint32_t allowed_caps;
    int no_pass;
    int session_only;
} pimp_rule_t;

int pimp_load_rules(void);
int pimp_save_rules(void);
int pimp_check(const char *username, uint32_t cap_type);
int pimp_rule_add(const char *username, uint32_t caps, int no_pass, int session_only);
int pimp_rule_remove(const char *username);
int pimp_rule_list(char *out, int out_len);
int hexafs_users_save(const void *data, uint32_t size);
int hexafs_users_load(void *buf, uint32_t *size);

#endif
