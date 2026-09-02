#ifndef REPLAY_H
#define REPLAY_H

#include "types.h"

#define MAX_REPLAY_EVENTS 1024

typedef struct {
    uint32_t tick;
    uint32_t pid;
    uint32_t syscall_num;
    uint32_t args[4];
    uint32_t snap_before;
    uint32_t snap_after;
} replay_event_t;

int replay_init(void);
int replay_record(uint32_t syscall_num, uint32_t arg0, uint32_t arg1, uint32_t arg2, uint32_t arg3);
int replay_execute(uint32_t snap_id, uint32_t event_count);
int replay_dry_run(uint32_t snap_id, uint32_t event_count);
int replay_get_count(void);
int replay_get_errors(void);
void replay_save_log(void);

#endif
