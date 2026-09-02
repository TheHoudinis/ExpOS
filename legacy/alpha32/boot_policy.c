#include "types.h"
#include "boot_policy.h"
#include "hexafs.h"
#include "log.h"

extern void print_string(const char *str);
extern void print_color(const char *str, uint8_t color);
extern void *memset(void *dest, int c, size_t len);
extern void *memcpy(void *dest, const void *src, size_t len);
extern int strcmp(const char *s1, const char *s2);
extern size_t strlen(const char *str);
extern char *strcpy(char *dest, const char *src);
extern void itoa(int num, char *str, int base);
extern volatile uint32_t system_ticks;

static boot_policy_t g_policy;
static int g_policy_loaded = 0;

static int g_boot_failed = 0;
static uint32_t g_failure_count = 0;

static uint32_t snap_block_by_name(const char *name) {
    return hexafs_snap_find(name);
}

static void write_failure_log(const char *stage_name, uint32_t tick) {
    (void)tick;
    if (!hexafs_mounted) return;
    char buf[64];
    int pos = 0;
    while (stage_name[pos] && pos < 31) { buf[pos] = stage_name[pos]; pos++; }
    buf[pos] = 0;
    log_write(LOG_LEVEL_ERROR, buf);
}

void boot_policy_init(void) {
    memset(&g_policy, 0, sizeof(g_policy));
    g_policy.stage_count = 4;

    int i = 0;
    const char *sn1 = "hardware_ok";
    while (sn1[i] && i < 31) { g_policy.stages[0].name[i] = sn1[i]; i++; }
    g_policy.stages[0].name[i] = 0;
    g_policy.stages[0].required_snap = 0;
    g_policy.stages[0].produces_snap = 0;
    g_policy.stages[0].verify_fn_hash = 0;

    i = 0; const char *sn2 = "drivers_loaded";
    while (sn2[i] && i < 31) { g_policy.stages[1].name[i] = sn2[i]; i++; }
    g_policy.stages[1].name[i] = 0;
    g_policy.stages[1].required_snap = 0;
    g_policy.stages[1].produces_snap = 0;
    g_policy.stages[1].verify_fn_hash = 0;

    i = 0; const char *sn3 = "services_ready";
    while (sn3[i] && i < 31) { g_policy.stages[2].name[i] = sn3[i]; i++; }
    g_policy.stages[2].name[i] = 0;
    g_policy.stages[2].required_snap = 0;
    g_policy.stages[2].produces_snap = 0;
    g_policy.stages[2].verify_fn_hash = 0;

    i = 0; const char *sn4 = "shell_ready";
    while (sn4[i] && i < 31) { g_policy.stages[3].name[i] = sn4[i]; i++; }
    g_policy.stages[3].name[i] = 0;
    g_policy.stages[3].required_snap = 0;
    g_policy.stages[3].produces_snap = 0;
    g_policy.stages[3].verify_fn_hash = 0;

    g_policy.fallback_snap = 0;
    g_policy.timeout_ticks = 5000;
    g_policy.policy_hash = 0;
    g_policy_loaded = 1;
}

int boot_policy_read(void) {
    uint32_t snap_block = snap_block_by_name("boot_policy");
    if (!snap_block) {
        boot_policy_init();
        boot_policy_write();
        return 0;
    }
    (void)snap_block;
    g_policy_loaded = 1;
    return 1;
}

int boot_policy_write(void) {
    if (!hexafs_mounted) return 0;
    return 1;
}

int boot_policy_run_stage(int stage_idx) {
    if (stage_idx < 0 || stage_idx >= (int)g_policy.stage_count) return 0;
    boot_stage_t *stage = &g_policy.stages[stage_idx];

    print_color("[BOOT] Stage: ", 0x0A);
    print_string(stage->name);
    print_string("\n");

    uint32_t timeout = g_policy.timeout_ticks;
    (void)timeout;
    int result = 1;

    if (!result) {
        print_color("[BOOT] Stage FAILED: ", 0x0C);
        print_string(stage->name);
        print_string("\n");
        write_failure_log(stage->name, system_ticks);
        g_boot_failed = 1;
        g_failure_count++;

        if (g_policy.fallback_snap) {
            print_color("[BOOT] Rolling back to fallback snapshot...\n", 0x0E);
        }
        return 0;
    }

    log_write(LOG_LEVEL_INFO, stage->name);
    return 1;
}

int boot_policy_execute(void) {
    if (!g_policy_loaded) {
        boot_policy_init();
    }

    print_string("[BOOT] Executing boot policy...\n");
    hexafs_tx_begin();

    for (uint32_t i = 0; i < g_policy.stage_count; i++) {
        if (!boot_policy_run_stage((int)i)) {
            print_color("[BOOT] Stage failed, rolling back...\n", 0x0C);
            hexafs_tx_abort();
            log_write(LOG_LEVEL_ERROR, "boot policy stage failed");
            return 0;
        }
    }

    hexafs_current_tx.dirty = 1;
    hexafs_snap_create("boot_ok");
    hexafs_tx_commit();

    print_color("[BOOT] Boot policy completed successfully.\n", 0x0A);
    log_write(LOG_LEVEL_INFO, "boot policy completed");
    return 1;
}

void boot_policy_set_fallback(const char *snap_name) {
    uint32_t snap_block = snap_block_by_name(snap_name);
    if (snap_block) {
        g_policy.fallback_snap = snap_block;
        print_string("Fallback snapshot set.\n");
    } else {
        print_string("Snapshot not found.\n");
    }
}

int boot_policy_get_failure_count(void) {
    return g_failure_count;
}

int boot_policy_get_stage_count(void) {
    return (int)g_policy.stage_count;
}

const char *boot_policy_get_stage_name(int idx) {
    if (idx < 0 || idx >= (int)g_policy.stage_count) return 0;
    return g_policy.stages[idx].name;
}
