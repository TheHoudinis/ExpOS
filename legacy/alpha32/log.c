#include "types.h"
#include "log.h"

static char log_ring[LOG_RING_SIZE];
static volatile int log_head = 0;
static volatile int log_tail = 0;
static volatile int log_count = 0;

static const char *level_str[] = {"INFO", "WARN", "ERR", "FATAL"};

static void serial_putc_log(char c) {
    for (int i = 0; i < 5000; i++) {
        if (inb(0x3F8 + 5) & 0x20) break;
        __asm__ volatile("pause");
    }
    outb(0x3F8, c);
}

static void serial_puts_log(const char *s) {
    while (*s) serial_putc_log(*s++);
}

void log_init(void) {
    log_head = 0;
    log_tail = 0;
    log_count = 0;
    log_write(LOG_LEVEL_INFO, "Log system initialized");
}

void log_write(int level, const char *msg) {
    int len = 0;
    while (msg[len] && len < 120) len++;
    cli();
    for (int i = 0; i < len + 1; i++) {
        char c = (i < len) ? msg[i] : '\n';
        log_ring[log_head] = c;
        log_head = (log_head + 1) % LOG_RING_SIZE;
        if (log_head == log_tail) log_tail = (log_tail + 1) % LOG_RING_SIZE;
        if (log_count < LOG_RING_SIZE) log_count++;
    }
    sti();
    if (level <= LOG_LEVEL_WARN) {
        serial_puts_log("[");
        serial_puts_log(level_str[level]);
        serial_puts_log("] ");
        serial_puts_log(msg);
        serial_putc_log('\n');
    }
}

void log_write_hex(int level, const char *prefix, uint32_t val) {
    char buf[64];
    int i = 0;
    while (*prefix) buf[i++] = *prefix++;
    buf[i++] = '0'; buf[i++] = 'x';
    for (int s = 7; s >= 0; s--) {
        int nib = (val >> (s * 4)) & 0xF;
        buf[i++] = nib < 10 ? '0' + nib : 'A' + nib - 10;
    }
    buf[i] = 0;
    log_write(level, buf);
}

void log_dump(void) {
    int idx = log_tail;
    int cnt = 0;
    while (idx != log_head && cnt < LOG_RING_SIZE) {
        idx = (idx + 1) % LOG_RING_SIZE;
        cnt++;
    }
}

const char *log_get(void) {
    static char buf[64];
    int idx = log_tail;
    int i = 0;
    while (idx != log_head && i < 63) {
        buf[i++] = log_ring[idx];
        idx = (idx + 1) % LOG_RING_SIZE;
    }
    buf[i] = 0;
    return buf;
}

int log_get_count(void) {
    return log_count;
}
