#ifndef PIPE_H
#define PIPE_H

#include "types.h"
#include "hexafs.h"

#define PIPE_BUF_SIZE 4096
#define PIPE_TYPED_CAPACITY 16

typedef struct {
    char buf[PIPE_BUF_SIZE];
    volatile int head;
    volatile int tail;
    volatile int readers;
    volatile int writers;
    volatile int closed;
} pipe_t;

typedef struct {
    uint32_t schema_hash;
    uint32_t data_hash;
    uint32_t data_size;
    uint8_t data[64];
} typed_pipe_form_t;

typedef struct {
    uint32_t schema_hash;
    uint32_t producer_pid;
    uint32_t consumer_pid;
    uint32_t cap_token;
    typed_pipe_form_t ring[PIPE_TYPED_CAPACITY];
    volatile int head;
    volatile int tail;
    volatile int active;
} typed_pipe_t;

int pipe_create(int *read_fd, int *write_fd);
int pipe_read(int pipe_id, char *buf, int len);
int pipe_write(int pipe_id, const char *buf, int len);
void pipe_close(int pipe_id, int is_writer);

int pipe_create_typed(uint32_t schema_hash, uint32_t producer_pid, uint32_t consumer_pid, uint32_t cap_token);
int pipe_typed_write(int tp_id, typed_pipe_form_t *form);
int pipe_typed_read(int tp_id, typed_pipe_form_t *form);
int pipe_typed_list(char *out, int out_len);

#endif
