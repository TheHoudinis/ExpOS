#include "types.h"
#include "pipe.h"
#include "process.h"
#include "log.h"

static pipe_t pipes[MAX_PIPES];
static int pipe_allocated[MAX_PIPES];

static typed_pipe_t typed_pipes[MAX_PIPES];
static int typed_pipe_count = 0;

extern void *memset(void *dest, int c, size_t len);
extern void *memcpy(void *dest, const void *src, size_t len);
extern void itoa(int num, char *str, int base);
extern size_t strlen(const char *str);

int pipe_create(int *read_fd, int *write_fd) {
    cli();
    int id = -1;
    for (int i = 0; i < MAX_PIPES; i++) {
        if (!pipe_allocated[i]) { id = i; break; }
    }
    if (id < 0) { sti(); return -1; }
    pipe_allocated[id] = 1;
    pipe_t *p = &pipes[id];
    p->head = 0;
    p->tail = 0;
    p->readers = 1;
    p->writers = 1;
    p->closed = 0;
    *read_fd = proc_alloc_fd(current_task, 2, id);
    *write_fd = proc_alloc_fd(current_task, 2, id + MAX_PIPES);
    sti();
    return 0;
}

int pipe_read(int pipe_id, char *buf, int len) {
    int id = pipe_id % MAX_PIPES;
    pipe_t *p = &pipes[id];
    int total = 0;
    while (total < len) {
        cli();
        if (p->head != p->tail) {
            buf[total++] = p->buf[p->tail];
            p->tail = (p->tail + 1) % PIPE_BUF_SIZE;
            sti();
        } else {
            sti();
            if (p->writers == 0) return total;
            __asm__ volatile("pause");
        }
    }
    return total;
}

int pipe_write(int pipe_id, const char *buf, int len) {
    int id = pipe_id % MAX_PIPES;
    pipe_t *p = &pipes[id];
    int total = 0;
    while (total < len) {
        cli();
        int next = (p->head + 1) % PIPE_BUF_SIZE;
        if (next != p->tail) {
            p->buf[p->head] = buf[total++];
            p->head = next;
            sti();
        } else {
            sti();
            if (p->readers == 0) return total;
            __asm__ volatile("pause");
        }
    }
    return total;
}

void pipe_close(int pipe_id, int is_writer) {
    int id = pipe_id % MAX_PIPES;
    pipe_t *p = &pipes[id];
    cli();
    if (is_writer) p->writers--;
    else p->readers--;
    if (p->readers == 0 && p->writers == 0) {
        pipe_allocated[id] = 0;
    }
    sti();
}

int pipe_create_typed(uint32_t schema_hash, uint32_t producer_pid, uint32_t consumer_pid, uint32_t cap_token) {
    cli();
    if (typed_pipe_count >= MAX_PIPES) { sti(); return -1; }
    typed_pipe_t *tp = &typed_pipes[typed_pipe_count];
    memset(tp, 0, sizeof(typed_pipe_t));
    tp->schema_hash = schema_hash;
    tp->producer_pid = producer_pid;
    tp->consumer_pid = consumer_pid;
    tp->cap_token = cap_token;
    tp->head = 0;
    tp->tail = 0;
    tp->active = 1;
    int id = typed_pipe_count++;
    log_write(LOG_LEVEL_INFO, "pipe: typed pipe created");
    sti();
    return id;
}

int pipe_typed_write(int tp_id, typed_pipe_form_t *form) {
    if (tp_id < 0 || tp_id >= typed_pipe_count) return -1;
    typed_pipe_t *tp = &typed_pipes[tp_id];
    if (!tp->active) return -1;
    if (form->schema_hash != tp->schema_hash) {
        log_write(LOG_LEVEL_WARN, "pipe: schema mismatch on typed write");
        return -1;
    }
    cli();
    int next = (tp->head + 1) % PIPE_TYPED_CAPACITY;
    if (next == tp->tail) { sti(); return -1; }
    tp->ring[tp->head] = *form;
    tp->head = next;
    sti();
    return (int)form->data_size;
}

int pipe_typed_read(int tp_id, typed_pipe_form_t *form) {
    if (tp_id < 0 || tp_id >= typed_pipe_count) return -1;
    typed_pipe_t *tp = &typed_pipes[tp_id];
    if (!tp->active) return -1;
    cli();
    if (tp->head == tp->tail) { sti(); return -1; }
    *form = tp->ring[tp->tail];
    tp->tail = (tp->tail + 1) % PIPE_TYPED_CAPACITY;
    sti();
    return (int)form->data_size;
}

int pipe_typed_list(char *out, int out_len) {
    int pos = 0;
    char buf[16];
    for (int i = 0; i < typed_pipe_count; i++) {
        typed_pipe_t *tp = &typed_pipes[i];
        if (!tp->active) continue;
        itoa(i, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        itoa((int)tp->schema_hash, buf, 16);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        itoa((int)tp->producer_pid, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        itoa((int)tp->consumer_pid, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = '\n';
    }
    out[pos] = 0;
    return pos;
}
