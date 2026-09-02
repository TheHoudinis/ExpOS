#include "types.h"
#include "intent.h"
#include "hexafs.h"
#include "process.h"
#include "log.h"

extern void *memset(void *dest, int c, size_t len);
extern void *memcpy(void *dest, const void *src, size_t len);
extern int strcmp(const char *s1, const char *s2);
extern size_t strlen(const char *str);
extern void *kmalloc(size_t size);
extern void kfree(void *ptr);
extern int find_form(const char *name);
extern void itoa(int num, char *str, int base);
extern struct hexa_formentry {
    char name[32];
    char *content;
    int size;
    int cap;
    int owner;
    uint16_t mode;
} form_table[];
extern int form_count;

static intent_entry_t intent_table[MAX_INTENTS];
static uint32_t next_handle = 1;

int intent_init(void) {
    memset(intent_table, 0, sizeof(intent_table));
    next_handle = 1;
    log_write(LOG_LEVEL_INFO, "intent: system initialized");
    return 0;
}

int intent_create(int pid, hexaos_intent_t *intent, uint32_t *handle) {
    if (!intent || !handle) return -1;

    if (intent->intent_type == INTENT_PRODUCE || intent->intent_type == INTENT_TRANSFORM) {
        if (!hexafs_cap_check((uint32_t)pid, CAP_TYPE_INTENT) && pid != 0) {
            return -1;
        }
    }

    for (int i = 0; i < MAX_INTENTS; i++) {
        if (!intent_table[i].active) {
            intent_table[i].active = 1;
            intent_table[i].pid = pid;
            intent_table[i].intent = *intent;
            intent_table[i].handle = next_handle++;
            intent_table[i].buffer_block = 0;
            intent_table[i].buffer_len = 0;
            *handle = intent_table[i].handle;
            return 0;
        }
    }
    return -1;
}

int intent_fulfill(uint32_t handle, void *buffer, int len) {
    for (int i = 0; i < MAX_INTENTS; i++) {
        if (intent_table[i].active && intent_table[i].handle == handle) {
            intent_entry_t *e = &intent_table[i];
            if (e->intent.intent_type == INTENT_PRODUCE) {
                if (buffer && len > 0) {
                    int idx = (int)e->intent.target_hash;
                    if (idx >= 0 && idx < form_count) {
                        if (e->intent.schema_hash != 0) {
                            uint32_t content_hash = hexafs_content_hash(buffer, (uint32_t)len);
                            if (content_hash != e->intent.schema_hash) {
                                return -1;
                            }
                        }
                        int copy = len < 65528 ? len : 65528;
                        memcpy(form_table[idx].content, buffer, (uint32_t)copy);
                        form_table[idx].content[copy] = 0;
                        if (copy > form_table[idx].size) form_table[idx].size = copy;
                        e->buffer_len = copy;
                        return copy;
                    }
                }
                e->buffer_block = 0;
                e->buffer_len = len;
                return len;
            } else if (e->intent.intent_type == INTENT_CONSUME) {
                if (buffer && len > 0) {
                    int idx = (int)e->intent.target_hash;
                    if (idx >= 0 && idx < form_count) {
                        int copy = len < form_table[idx].size ? len : form_table[idx].size;
                        memcpy(buffer, form_table[idx].content, (uint32_t)copy);
                        return copy;
                    }
                    int copy = len < e->buffer_len ? len : e->buffer_len;
                    if (copy > 0) memcpy(buffer, &e->buffer_block, (uint32_t)copy);
                    return copy;
                }
            } else if (e->intent.intent_type == INTENT_OBSERVE) {
                if (buffer && len > 0) {
                    int idx = (int)e->intent.target_hash;
                    if (idx >= 0 && idx < form_count) {
                        int copy = len < form_table[idx].size ? len : form_table[idx].size;
                        memcpy(buffer, form_table[idx].content, (uint32_t)copy);
                        return copy;
                    }
                }
            } else if (e->intent.intent_type == INTENT_TRANSFORM) {
                if (buffer && len > 0) {
                    int idx = (int)e->intent.target_hash;
                    if (idx >= 0 && idx < form_count) {
                        uint32_t content_hash = hexafs_content_hash(buffer, (uint32_t)len);
                        if (e->intent.schema_hash != 0 && content_hash != e->intent.schema_hash) {
                            return -1;
                        }
                        int copy = len < 65528 ? len : 65528;
                        memcpy(form_table[idx].content, buffer, (uint32_t)copy);
                        form_table[idx].content[copy] = 0;
                        if (copy > form_table[idx].size) form_table[idx].size = copy;
                        e->buffer_len = copy;
                        return copy;
                    }
                }
            }
            return 0;
        }
    }
    return -1;
}

int intent_close(uint32_t handle) {
    for (int i = 0; i < MAX_INTENTS; i++) {
        if (intent_table[i].active && intent_table[i].handle == handle) {
            intent_table[i].active = 0;
            return 0;
        }
    }
    return -1;
}

int intent_compat_open(const char *path, int flags) {
    int idx = find_form(path);
    if (idx < 0) {
        if (!(flags & 4)) return -1;
        extern void cmd_mkform(const char *name);
        cmd_mkform(path);
        idx = find_form(path);
        if (idx < 0) return -1;
    }
    hexaos_intent_t intent;
    if (flags & 1)
        intent.intent_type = INTENT_PRODUCE;
    else
        intent.intent_type = INTENT_CONSUME;
    intent.target_hash = (uint32_t)(uint32_t)idx;
    intent.schema_hash = 0;
    intent.filter_offset = 0;
    intent.flags = flags & 1 ? INTENT_ONCE : INTENT_STREAM;
    uint32_t handle;
    if (intent_create(tasks[current_task].pid, &intent, &handle) < 0) return -1;
    return (int)handle;
}

int intent_compat_read(int fd, char *buf, int count, int pos) {
    (void)pos;
    intent_entry_t *e = 0;
    for (int i = 0; i < MAX_INTENTS; i++) {
        if (intent_table[i].active && (int)intent_table[i].handle == fd) {
            e = &intent_table[i];
            break;
        }
    }
    if (!e) return -1;
    if (e->intent.intent_type != INTENT_CONSUME && e->intent.intent_type != INTENT_OBSERVE) return -1;
    return intent_fulfill((uint32_t)fd, buf, count);
}

int intent_compat_write(int fd, const char *buf, int count, int pos) {
    (void)pos;
    intent_entry_t *e = 0;
    for (int i = 0; i < MAX_INTENTS; i++) {
        if (intent_table[i].active && (int)intent_table[i].handle == fd) {
            e = &intent_table[i];
            break;
        }
    }
    if (!e) {
        int idx = find_form((const char *)(uint32_t)fd);
        if (idx >= 0) {
            hexaos_intent_t intent;
            intent.intent_type = INTENT_PRODUCE;
            intent.target_hash = (uint32_t)idx;
            intent.schema_hash = 0;
            intent.filter_offset = 0;
            intent.flags = INTENT_ONCE;
            uint32_t handle;
            if (intent_create(tasks[current_task].pid, &intent, &handle) < 0) return -1;
            return intent_fulfill(handle, (void *)buf, count);
        }
        return -1;
    }
    if (e->intent.intent_type != INTENT_PRODUCE) return -1;
    return intent_fulfill((uint32_t)fd, (void *)buf, count);
}

int intent_compat_stat(const char *path, void *node) {
    if (!path || !node) return -1;
    int idx = find_form(path);
    if (idx < 0) return -1;
    struct vfs_node *vn = (struct vfs_node *)node;
    int i = 0;
    while (form_table[idx].name[i] && i < 31) { vn->name[i] = form_table[idx].name[i]; i++; }
    vn->name[i] = 0;
    vn->type = 0;
    vn->size = form_table[idx].size;
    vn->owner = form_table[idx].owner;
    vn->mode = form_table[idx].mode;
    vn->ref_count = 1;
    return 0;
}

int intent_table_active_count(void) {
    int n = 0;
    for (int i = 0; i < MAX_INTENTS; i++)
        if (intent_table[i].active) n++;
    return n;
}

void intent_list_active(char *out, int out_len) {
    int pos = 0;
    char buf[16];
    for (int i = 0; i < MAX_INTENTS; i++) {
        if (!intent_table[i].active) continue;
        itoa((int)intent_table[i].handle, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        itoa((int)intent_table[i].pid, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        const char *tstr = "CONS";
        if (intent_table[i].intent.intent_type == INTENT_PRODUCE) tstr = "PROD";
        else if (intent_table[i].intent.intent_type == INTENT_OBSERVE) tstr = "OBSV";
        else if (intent_table[i].intent.intent_type == INTENT_TRANSFORM) tstr = "XFrm";
        for (int j = 0; tstr[j] && pos < out_len - 1; j++) out[pos++] = tstr[j];
        out[pos++] = '\n';
    }
    out[pos] = 0;
}
