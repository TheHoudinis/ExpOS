#ifndef PROCESS_H
#define PROCESS_H

#include "types.h"
#include "interrupts.h"

#define MAX_TASKS 16
#define STACK_SIZE 8192
#define MAX_FDS 16
#define MAX_PIPES 16

#define TASK_READY   0
#define TASK_RUNNING 1
#define TASK_BLOCKED 2
#define TASK_ZOMBIE  3
#define TASK_DEAD    4

#define PROC_NAME_LEN 24

#define EVENT_TERMINATE 1
#define EVENT_PAUSE     2
#define EVENT_RESUME    3
#define EVENT_CUSTOM    4
#define EVENT_ERROR     5

#define EVENT_QUEUE_SIZE 16

struct fd_entry {
    int type;  // 0=closed, 1=form, 2=pipe, 3=console
    int ref;   // form index / pipe id
    int pos;
    int flags;
};

typedef struct {
    uint32_t event_type;
    uint32_t sender_pid;
    uint32_t sender_snap;
    uint32_t payload_hash;
    uint32_t timestamp;
} hexaos_event_t;

struct task {
    int pid;
    int state;
    int exit_code;
    int parent_pid;
    int priority;
    uint32_t esp;
    uint32_t ebp;
    uint32_t page_dir;
    uint32_t heap_brk;
    uint32_t kernel_esp;
    uint8_t stack[STACK_SIZE];
    char name[PROC_NAME_LEN];
    struct fd_entry fds[MAX_FDS];
    hexaos_event_t event_queue[EVENT_QUEUE_SIZE];
    int event_head;
    int event_tail;
    volatile int event_pending;
};

void proc_init(void);
pid_t proc_create(void (*entry)(void), const char *name, int priority);
pid_t proc_create_user(uint32_t entry, const char *name);
void proc_exit(int code);
pid_t proc_wait(int *status);
int proc_kill(pid_t pid);
void proc_reap_zombies(void);
void schedule(struct regs *r);
void yield(void);
int proc_get_fd_entry(pid_t pid, int fd, struct fd_entry **fde);
int proc_alloc_fd(pid_t pid, int type, int ref);
void proc_free_fd(pid_t pid, int fd);

int process_event_send(pid_t target_pid, uint32_t event_type, uint32_t payload_hash);
int process_event_poll(uint32_t event_type_mask, hexaos_event_t *out_event);

extern struct task tasks[MAX_TASKS];
extern int current_task;
extern int num_tasks;

#endif
