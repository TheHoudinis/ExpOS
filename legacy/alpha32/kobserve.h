#ifndef KOBSERVE_H
#define KOBSERVE_H

#include "types.h"
#include "hexafs.h"

typedef struct {
    char path[64];
    int (*query_fn)(uint32_t filter, char *out, int out_len);
    uint32_t update_interval_ticks;
    uint32_t last_update_tick;
} kernel_observer_t;

#define KOBSERVE_MAX 32

int kobserve_register(const char *path, int (*query_fn)(uint32_t, char*, int), uint32_t interval);
int kobserve_query(const char *path, uint32_t filter, char *out, int out_len);
void kobserve_init(void);
int kobserve_read_kernel_path(const char *path, char *out, int out_len);

int kobserve_sched_tasks(uint32_t filter, char *out, int out_len);
int kobserve_mem_pages(uint32_t filter, char *out, int out_len);
int kobserve_mem_heap(uint32_t filter, char *out, int out_len);
int kobserve_intr_log(uint32_t filter, char *out, int out_len);
int kobserve_intr_stats(uint32_t filter, char *out, int out_len);

#endif
