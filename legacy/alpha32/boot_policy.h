#ifndef BOOT_POLICY_H
#define BOOT_POLICY_H

#include "types.h"

#define BOOT_MAX_STAGES 8

typedef struct {
    char name[32];
    uint32_t required_snap;
    uint32_t produces_snap;
    uint32_t verify_fn_hash;
} boot_stage_t;

typedef struct {
    uint32_t stage_count;
    boot_stage_t stages[BOOT_MAX_STAGES];
    uint32_t fallback_snap;
    uint32_t timeout_ticks;
    uint32_t policy_hash;
} boot_policy_t;

void boot_policy_init(void);
int boot_policy_read(void);
int boot_policy_write(void);
int boot_policy_run_stage(int stage_idx);
int boot_policy_execute(void);
void boot_policy_set_fallback(const char *snap_name);
int boot_policy_get_failure_count(void);
int boot_policy_get_stage_count(void);
const char *boot_policy_get_stage_name(int idx);

#endif
