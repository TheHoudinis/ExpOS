#ifndef EXPFS_H
#define EXPFS_H

#include "types.h"
#include "expfs_disk.h"
#include "vfs.h"

typedef struct {
    int      active;
    int      tid;
    uint32_t parent_snap_block;
    uint32_t new_root_block;
    uint32_t snap_block;
    uint32_t journal_block;
    int      dirty;
} expfs_tx_t;

extern int expfs_mounted;
extern expfs_tx_t expfs_current_tx;
extern expfs_superblock_t sb_cache;

int expfs_mount_disk(void);
int expfs_write_superblock(void);
int expfs_save_bitmap(void);
uint32_t expfs_abstraction_create(void);
int expfs_abstraction_add_entry(uint32_t abs_block, const char *name, uint32_t obj_block, uint8_t type);
int expfs_abstraction_find(uint32_t abs_block, const char *name, uint32_t *obj_block, uint8_t *type);
int expfs_abstraction_remove_entry(uint32_t abs_block, const char *name);
int expfs_abstraction_list(uint32_t abs_block, void (*cb)(const char *name, uint32_t obj_block, uint8_t type));

int expfs_mount(void);
int expfs_format(void);
int expfs_tx_begin(void);
int expfs_tx_commit(void);
int expfs_tx_abort(void);

uint32_t expfs_content_hash(const void *data, uint32_t size);
uint32_t expfs_alloc_block(void);
void expfs_free_block(uint32_t lba);
int expfs_block_read(uint32_t lba, void *buf);
int expfs_block_write(uint32_t lba, const void *buf);
int expfs_block_verify(uint32_t lba, const void *buf, uint32_t stored_crc);

uint32_t expfs_object_alloc(uint8_t type);
int expfs_object_write_data(uint32_t obj_block, const void *data, uint32_t size);
int expfs_object_read_data(uint32_t obj_block, void *buf, uint32_t *size, uint8_t *type);

uint32_t expfs_snap_create(const char *name);
uint32_t expfs_snap_find(const char *name);

int expfs_vfs_open(const char *path, int flags);
int expfs_vfs_read(int obj_id, char *buf, int count, int pos);
int expfs_vfs_write(int obj_id, const char *buf, int count, int pos);
int expfs_vfs_stat(const char *path, struct vfs_node *node, uint32_t *obj_id);
int expfs_vfs_close(int obj_id);
int expfs_vfs_list(void (*cb)(const char *name, uint32_t obj_block, uint8_t type));
void expfs_save_all(void);
void expfs_load_all(void);

int expfs_cap_grant(uint32_t grantee_pid, uint32_t cap_type, uint32_t expires_tick, int delegatable);
int expfs_cap_check(uint32_t pid, uint32_t cap_type);
int expfs_cap_revoke(uint32_t grant_hash);
int expfs_cap_list_pid(uint32_t pid, char *out, int out_len);
uint32_t expfs_snap_for_pid(uint32_t pid);

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
int expfs_users_save(const void *data, uint32_t size);
int expfs_users_load(void *buf, uint32_t *size);

#endif
