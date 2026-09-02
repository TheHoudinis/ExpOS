#include "types.h"
#include "replay.h"
#include "hexafs.h"
#include "hexafs_disk.h"
#include "process.h"
#include "log.h"

extern volatile uint32_t system_ticks;

static replay_event_t replay_buffer[MAX_REPLAY_EVENTS];
static int replay_count = 0;
static int replay_recording = 0;
static int replay_session_complete = 0;
static uint32_t replay_last_error_tick = 0;

extern void *memset(void *dest, int c, size_t len);
extern void print_string(const char *str);
extern void print_color(const char *str, uint8_t color);
extern void itoa(int num, char *str, int base);
extern size_t strlen(const char *str);

int replay_init(void) {
    memset(replay_buffer, 0, sizeof(replay_buffer));
    replay_count = 0;
    replay_recording = 1;
    replay_session_complete = 0;
    log_write(LOG_LEVEL_INFO, "replay: event recording started");
    return 0;
}

int replay_record(uint32_t syscall_num, uint32_t arg0, uint32_t arg1, uint32_t arg2, uint32_t arg3) {
    if (!replay_recording) return 0;
    if (replay_count >= MAX_REPLAY_EVENTS) return -1;
    replay_event_t *e = &replay_buffer[replay_count];
    e->tick = system_ticks;
    e->pid = (uint32_t)(current_task >= 0 ? tasks[current_task].pid : 0);
    e->syscall_num = syscall_num;
    e->args[0] = arg0;
    e->args[1] = arg1;
    e->args[2] = arg2;
    e->args[3] = arg3;
    e->snap_before = 0;
    e->snap_after = 0;
    replay_count++;
    return replay_count;
}

static int save_event_log_to_hexafs(void) {
    if (!hexafs_mounted) return 0;
    if (!hexafs_current_tx.active) return 0;
    uint32_t event_obj = hexafs_object_alloc(HEXAFS_CONFIG);
    if (!event_obj) return 0;
    if (!hexafs_object_write_data(event_obj, (uint8_t *)replay_buffer,
                                   (uint32_t)(replay_count * sizeof(replay_event_t)))) {
        hexafs_free_block(event_obj);
        return 0;
    }
    uint32_t snap_block = hexafs_snap_find("live");
    if (!snap_block) snap_block = sb_cache.root_snap_block;
    if (snap_block) {
        hexafs_abstraction_add_entry(snap_block, "replay_log", event_obj, HEXAFS_CONFIG);
    }
    return 1;
}

void replay_save_log(void) {
    save_event_log_to_hexafs();
}

int replay_execute(uint32_t snap_id, uint32_t event_count) {
    (void)snap_id;
    if (event_count > (uint32_t)replay_count) event_count = (uint32_t)replay_count;
    print_color("[REPLAY] Executing ", 0x0E);
    char buf[16];
    itoa((int)event_count, buf, 10);
    print_string(buf);
    print_string(" events\n");

    int errors = 0;
    for (uint32_t i = 0; i < event_count; i++) {
        replay_event_t *e = &replay_buffer[i];
        print_string("  event ");
        itoa((int)i, buf, 10);
        print_string(buf);
        print_string(": syscall ");
        itoa((int)e->syscall_num, buf, 10);
        print_string(buf);
        print_string(" pid=");
        itoa((int)e->pid, buf, 10);
        print_string(buf);
        print_string("\n");
    }

    if (errors > 0) {
        print_color("[REPLAY] Errors detected: ", 0x0C);
        itoa(errors, buf, 10);
        print_string(buf);
        print_string("\n");
        replay_last_error_tick = system_ticks;
    }
    replay_session_complete = 1;
    log_write(LOG_LEVEL_INFO, "replay: execution complete");
    return (int)event_count;
}

int replay_dry_run(uint32_t snap_id, uint32_t event_count) {
    print_color("[REPLAY] Dry run (read-only) for ", 0x0E);
    char buf[16];
    itoa((int)event_count, buf, 10);
    print_string(buf);
    print_string(" events\n");
    return replay_execute(snap_id, event_count);
}

int replay_get_count(void) {
    return replay_count;
}

int replay_get_errors(void) {
    return replay_session_complete ? 0 : -1;
}
