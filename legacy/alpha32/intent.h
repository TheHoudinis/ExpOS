#ifndef INTENT_H
#define INTENT_H

#include "types.h"

#define INTENT_CONSUME   0x01
#define INTENT_PRODUCE   0x02
#define INTENT_OBSERVE   0x03
#define INTENT_TRANSFORM 0x04

#define INTENT_STREAM    0x01
#define INTENT_ONCE      0x02
#define INTENT_CACHED    0x04

#define MAX_INTENTS 64

typedef struct {
    uint32_t intent_type;
    uint32_t target_hash;
    uint32_t schema_hash;
    uint32_t filter_offset;
    uint32_t flags;
} hexaos_intent_t;

typedef struct {
    int active;
    int pid;
    hexaos_intent_t intent;
    uint32_t handle;
    uint32_t buffer_block;
    int buffer_len;
} intent_entry_t;

int intent_init(void);
int intent_create(int pid, hexaos_intent_t *intent, uint32_t *handle);
int intent_fulfill(uint32_t handle, void *buffer, int len);
int intent_close(uint32_t handle);
int intent_compat_open(const char *path, int flags);
int intent_compat_read(int fd, char *buf, int count, int pos);
int intent_compat_write(int fd, const char *buf, int count, int pos);
int intent_compat_stat(const char *path, void *node);
int intent_table_active_count(void);
void intent_list_active(char *out, int out_len);

#endif
