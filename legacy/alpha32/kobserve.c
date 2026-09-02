#include "types.h"
#include "kobserve.h"
#include "process.h"
#include "paging.h"
#include "interrupts.h"
#include "log.h"
#include "hexafs.h"

extern int num_tasks;
extern void *memset(void *dest, int c, size_t len);
extern void *memcpy(void *dest, const void *src, size_t len);
extern void print_string(const char *str);
extern void itoa(int num, char *str, int base);
extern size_t strlen(const char *str);

static kernel_observer_t kobserve_table[KOBSERVE_MAX];
static int kobserve_count = 0;

extern volatile uint32_t system_ticks;

static int kobserve_find(const char *path) {
    for (int i = 0; i < kobserve_count; i++) {
        int match = 1;
        for (int j = 0; j < 64; j++) {
            if (kobserve_table[i].path[j] != path[j]) { match = 0; break; }
            if (path[j] == 0) break;
        }
        if (match) return i;
    }
    return -1;
}

int kobserve_register(const char *path, int (*query_fn)(uint32_t, char*, int), uint32_t interval) {
    if (kobserve_count >= KOBSERVE_MAX) return -1;
    int i = 0;
    while (path[i] && i < 63) {
        kobserve_table[kobserve_count].path[i] = path[i];
        i++;
    }
    kobserve_table[kobserve_count].path[i] = 0;
    kobserve_table[kobserve_count].query_fn = query_fn;
    kobserve_table[kobserve_count].update_interval_ticks = interval;
    kobserve_table[kobserve_count].last_update_tick = 0;
    kobserve_count++;
    return 0;
}

int kobserve_query(const char *path, uint32_t filter, char *out, int out_len) {
    int idx = kobserve_find(path);
    if (idx < 0) return -1;
    kernel_observer_t *obs = &kobserve_table[idx];
    if (obs->query_fn) {
        return obs->query_fn(filter, out, out_len);
    }
    return -1;
}

int kobserve_read_kernel_path(const char *path, char *out, int out_len) {
    return kobserve_query(path, 0, out, out_len);
}

int kobserve_sched_tasks(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    char buf[16];
    const char *hdr = "PID  PPID STATE NAME\n---  ---- ----- ----\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    for (int i = 0; i < MAX_TASKS; i++) {
        if (tasks[i].state == TASK_DEAD) continue;
        itoa(tasks[i].pid, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        out[pos++] = ' ';
        itoa(tasks[i].parent_pid, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        out[pos++] = ' ';
        const char *state_str = "DEAD";
        switch (tasks[i].state) {
            case TASK_RUNNING: state_str = "RUN"; break;
            case TASK_READY:   state_str = "RDY"; break;
            case TASK_BLOCKED: state_str = "BLK"; break;
            case TASK_ZOMBIE:  state_str = "ZOM"; break;
        }
        for (int j = 0; state_str[j] && pos < out_len - 1; j++) out[pos++] = state_str[j];
        out[pos++] = ' ';
        for (int j = 0; tasks[i].name[j] && pos < out_len - 1; j++) out[pos++] = tasks[i].name[j];
        out[pos++] = '\n';
    }
    out[pos] = 0;
    return pos;
}

int kobserve_sched_history(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    const char *msg = "Scheduling history: recorded in snapshot events.\n";
    for (int i = 0; msg[i] && pos < out_len - 1; i++) out[pos++] = msg[i];
    out[pos] = 0;
    return pos;
}

int kobserve_mem_pages(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    char buf[16];
    uint32_t total = pmm_get_total_pages();
    uint32_t free_p = pmm_count_free();
    uint32_t used = total - free_p;
    const char *hdr = "Memory Page Info:\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    itoa(total * 4, buf, 10);
    for (int i = 0; buf[i] && pos < out_len - 1; i++) out[pos++] = buf[i];
    const char *s1 = " KB total (";
    for (int i = 0; s1[i] && pos < out_len - 1; i++) out[pos++] = s1[i];
    itoa(total, buf, 10);
    for (int i = 0; buf[i] && pos < out_len - 1; i++) out[pos++] = buf[i];
    const char *s2 = " pages)\n";
    for (int i = 0; s2[i] && pos < out_len - 1; i++) out[pos++] = s2[i];
    itoa(used * 4, buf, 10);
    for (int i = 0; buf[i] && pos < out_len - 1; i++) out[pos++] = buf[i];
    const char *s3 = " KB used\n";
    for (int i = 0; s3[i] && pos < out_len - 1; i++) out[pos++] = s3[i];
    itoa(free_p * 4, buf, 10);
    for (int i = 0; buf[i] && pos < out_len - 1; i++) out[pos++] = buf[i];
    const char *s4 = " KB free\n";
    for (int i = 0; s4[i] && pos < out_len - 1; i++) out[pos++] = s4[i];
    out[pos] = 0;
    return pos;
}

int kobserve_mem_heap(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    const char *msg = "Kernel Heap: 0x800000 - 0x900000 (1MB)\n";
    for (int i = 0; msg[i] && pos < out_len - 1; i++) out[pos++] = msg[i];
    out[pos] = 0;
    return pos;
}

int kobserve_intr_log(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    const char *log_data = log_get();
    if (!log_data) {
        const char *empty = "No log entries.\n";
        for (int i = 0; empty[i] && pos < out_len - 1; i++) out[pos++] = empty[i];
        out[pos] = 0;
        return pos;
    }
    for (int i = 0; log_data[i] && pos < out_len - 1; i++) out[pos++] = log_data[i];
    out[pos] = 0;
    return pos;
}

int kobserve_intr_stats(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    char buf[16];
    const char *hdr = "Interrupt Statistics:\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    const char *line = "  IRQ 0 (PIT): active\n  IRQ 1 (Keyboard): active\n  Exception handlers: 0-31 registered\n  Syscall (int 0x80): active\n";
    for (int i = 0; line[i] && pos < out_len - 1; i++) out[pos++] = line[i];
    itoa(system_ticks, buf, 10);
    const char *tick_prefix = "  Total timer ticks: ";
    for (int i = 0; tick_prefix[i] && pos < out_len - 1; i++) out[pos++] = tick_prefix[i];
    for (int i = 0; buf[i] && pos < out_len - 1; i++) out[pos++] = buf[i];
    out[pos++] = '\n';
    out[pos] = 0;
    return pos;
}

int kobserve_vfs_mounts(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    const char *hdr = "VFS Mounts:\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    const char *m1 = "  /  -> hexafs_disk (persistent)\n";
    for (int i = 0; m1[i] && pos < out_len - 1; i++) out[pos++] = m1[i];
    out[pos] = 0;
    return pos;
}

extern int intent_table_active_count(void);
extern void intent_list_active(char *out, int out_len);

int kobserve_open_intents(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    const char *hdr = "Active Intent Handles:\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    intent_list_active(out + pos, out_len - pos);
    pos = strlen(out);
    if (pos == 0 || out[pos-1] != '\n') {
        const char *none = "  (none)\n";
        for (int i = 0; none[i] && pos < out_len - 1; i++) out[pos++] = none[i];
        out[pos] = 0;
    }
    return pos;
}

int kobserve_caps_grants(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    const char *hdr = "Capability Grants:\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    const char *none = "  (no active grants)\n";
    for (int i = 0; none[i] && pos < out_len - 1; i++) out[pos++] = none[i];
    out[pos] = 0;
    return pos;
}

int kobserve_caps_revoked(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    const char *hdr = "Revoked Capabilities:\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    const char *none = "  (no revoked caps)\n";
    for (int i = 0; none[i] && pos < out_len - 1; i++) out[pos++] = none[i];
    out[pos] = 0;
    return pos;
}

extern int pipe_typed_list(char *out, int out_len);

int kobserve_pipes_active(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    const char *hdr = "Active Typed Pipes:\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    pipe_typed_list(out + pos, out_len - pos);
    pos = strlen(out);
    if (pos == 0 || out[pos-1] != '\n') {
        const char *none = "  (none)\n";
        for (int i = 0; none[i] && pos < out_len - 1; i++) out[pos++] = none[i];
        out[pos] = 0;
    }
    return pos;
}

int kobserve_snapshots_tree(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    const char *hdr = "Snapshot Tree:\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    uint32_t snap_block = 0;
    extern hexafs_superblock_t sb_cache;
    if (sb_cache.root_snap_block) {
        snap_block = sb_cache.root_snap_block;
        hexafs_snap_t snap;
        while (snap_block) {
            if (!hexafs_block_read(snap_block, &snap)) break;
            if (snap.magic != HEXAFS_SNAP_MAGIC) break;
            char buf[16];
            itoa(snap_block, buf, 10);
            for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
            out[pos++] = ' ';
            for (int j = 0; snap.name[j] && j < 31 && pos < out_len - 1; j++) out[pos++] = snap.name[j];
            out[pos++] = '\n';
            snap_block = snap.parent_snap_block;
        }
    } else {
        const char *none = "  (no snapshots)\n";
        for (int i = 0; none[i] && pos < out_len - 1; i++) out[pos++] = none[i];
    }
    out[pos] = 0;
    return pos;
}

int kobserve_replay_last(uint32_t filter, char *out, int out_len) {
    (void)filter;
    int pos = 0;
    const char *msg = "Last Replay Session:\n  (no replay sessions recorded)\n";
    for (int i = 0; msg[i] && pos < out_len - 1; i++) out[pos++] = msg[i];
    out[pos] = 0;
    return pos;
}

void kobserve_init(void) {
    memset(kobserve_table, 0, sizeof(kobserve_table));
    kobserve_count = 0;
    kobserve_register("/@kernel/scheduler/tasks", kobserve_sched_tasks, 0);
    kobserve_register("/@kernel/scheduler/history", kobserve_sched_history, 0);
    kobserve_register("/@kernel/memory/pages", kobserve_mem_pages, 0);
    kobserve_register("/@kernel/memory/heap", kobserve_mem_heap, 0);
    kobserve_register("/@kernel/interrupts/log", kobserve_intr_log, 0);
    kobserve_register("/@kernel/interrupts/stats", kobserve_intr_stats, 0);
    kobserve_register("/@kernel/vfs/mounts", kobserve_vfs_mounts, 0);
    kobserve_register("/@kernel/vfs/open_intents", kobserve_open_intents, 0);
    kobserve_register("/@kernel/capabilities/grants", kobserve_caps_grants, 0);
    kobserve_register("/@kernel/capabilities/revoked", kobserve_caps_revoked, 0);
    kobserve_register("/@kernel/pipes/active", kobserve_pipes_active, 0);
    kobserve_register("/@kernel/snapshots/tree", kobserve_snapshots_tree, 0);
    kobserve_register("/@kernel/replay/last", kobserve_replay_last, 0);
    log_write(LOG_LEVEL_INFO, "kobserve: initialized 13 kernel observers");
}
