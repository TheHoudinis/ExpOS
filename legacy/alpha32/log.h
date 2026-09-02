#ifndef LOG_H
#define LOG_H

#include "types.h"

#define LOG_RING_SIZE 4096
#define LOG_LEVEL_INFO  0
#define LOG_LEVEL_WARN  1
#define LOG_LEVEL_ERROR 2
#define LOG_LEVEL_FATAL 3

void log_init(void);
void log_write(int level, const char *msg);
void log_write_hex(int level, const char *prefix, uint32_t val);
void log_dump(void);
const char *log_get(void);
int log_get_count(void);

#endif
