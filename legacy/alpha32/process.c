#include "types.h"
#include "process.h"
#include "interrupts.h"
#include "paging.h"

struct task tasks[MAX_TASKS];
int current_task = 0;
int num_tasks = 0;
static int next_pid = 1;

static uint32_t tss[26] __attribute__((aligned(4)));
extern void tss_flush(void);
extern void setup_tss(uint32_t tss_addr);
extern void enter_userspace(uint32_t entry, uint32_t stack, uint32_t cs, uint32_t ds);

static int alloc_pid(void) {
    for (int i = 0; i < MAX_TASKS; i++) {
        if (tasks[i].state == TASK_DEAD) {
            tasks[i].pid = next_pid++;
            return i;
        }
    }
    return -1;
}

void proc_init(void) {
    for (int i = 0; i < MAX_TASKS; i++) {
        tasks[i].pid = -1;
        tasks[i].state = TASK_DEAD;
        tasks[i].esp = 0;
        tasks[i].parent_pid = -1;
        tasks[i].priority = 1;
        tasks[i].page_dir = 0;
        tasks[i].heap_brk = 0;
        tasks[i].name[0] = 0;
        tasks[i].event_head = 0;
        tasks[i].event_tail = 0;
        tasks[i].event_pending = 0;
        for (int f = 0; f < MAX_FDS; f++) {
            tasks[i].fds[f].type = 0;
            tasks[i].fds[f].ref = -1;
        }
        for (int e = 0; e < EVENT_QUEUE_SIZE; e++) {
            tasks[i].event_queue[e].event_type = 0;
            tasks[i].event_queue[e].sender_pid = 0;
            tasks[i].event_queue[e].payload_hash = 0;
            tasks[i].event_queue[e].timestamp = 0;
        }
        // Default form handles 0-2
        tasks[i].fds[0].type = 3; tasks[i].fds[0].ref = 0;
        tasks[i].fds[1].type = 3; tasks[i].fds[1].ref = 0;
        tasks[i].fds[2].type = 3; tasks[i].fds[2].ref = 0;
    }
    for (int i = 0; i < 26; i++) tss[i] = 0;
    tss[1] = 0x10;
    setup_tss((uint32_t)tss);
    tss_flush();
    current_task = 0;
    num_tasks = 0;
}

pid_t proc_create(void (*entry)(void), const char *name, int priority) {
    if (num_tasks >= MAX_TASKS) return -1;
    int slot = alloc_pid();
    if (slot < 0) return -1;
    struct task *t = &tasks[slot];
    t->state = TASK_READY;
    t->priority = priority > 0 ? priority : 1;
    int name_len = 0;
    while (name[name_len] && name_len < PROC_NAME_LEN - 1) {
        t->name[name_len] = name[name_len];
        name_len++;
    }
    t->name[name_len] = 0;
    t->parent_pid = current_task >= 0 && current_task < MAX_TASKS ? tasks[current_task].pid : -1;
    t->page_dir = get_kernel_page_dir();
    uint32_t *stack_top = (uint32_t *)((uint32_t)t->stack + STACK_SIZE);
    *--stack_top = 0x10;
    *--stack_top = (uint32_t)t->stack + STACK_SIZE;
    *--stack_top = 0x202;
    *--stack_top = 0x08;
    *--stack_top = (uint32_t)entry;
    *--stack_top = 0;
    *--stack_top = 0;
    *--stack_top = 0; *--stack_top = 0; *--stack_top = 0; *--stack_top = 0;
    *--stack_top = 0; *--stack_top = 0; *--stack_top = 0; *--stack_top = 0;
    *--stack_top = 0x10; *--stack_top = 0x10; *--stack_top = 0x10; *--stack_top = 0x10;
    t->esp = (uint32_t)stack_top;
    t->ebp = 0;
    t->kernel_esp = (uint32_t)t->stack + STACK_SIZE;
    t->heap_brk = 0;
    num_tasks++;
    return t->pid;
}

pid_t proc_create_user(uint32_t entry, const char *name) {
    if (num_tasks >= MAX_TASKS) return -1;
    int slot = alloc_pid();
    if (slot < 0) return -1;
    struct task *t = &tasks[slot];
    t->state = TASK_READY;
    int name_len = 0;
    while (name[name_len] && name_len < PROC_NAME_LEN - 1) {
        t->name[name_len] = name[name_len];
        name_len++;
    }
    t->name[name_len] = 0;
    t->parent_pid = current_task >= 0 && current_task < MAX_TASKS ? tasks[current_task].pid : -1;
    t->priority = 1;
    uint32_t user_stack = pmm_alloc();
    if (!user_stack) return -1;
    map_page(user_stack, user_stack, 0x007);
    uint32_t user_stack_top = user_stack + PAGE_SIZE;
    uint32_t *sp = (uint32_t *)user_stack_top;
    *--sp = 0x20;
    *--sp = user_stack_top;
    *--sp = 0x202;
    *--sp = 0x18;
    *--sp = entry;
    *--sp = 0; *--sp = 0;
    *--sp = 0; *--sp = 0; *--sp = 0; *--sp = 0;
    *--sp = 0; *--sp = 0; *--sp = 0; *--sp = 0;
    *--sp = 0x20; *--sp = 0x20; *--sp = 0x20; *--sp = 0x20;
    t->esp = (uint32_t)sp;
    t->ebp = 0;
    t->kernel_esp = (uint32_t)t->stack + STACK_SIZE;
    t->page_dir = create_user_page_dir();
    t->heap_brk = 0;
    num_tasks++;
    return t->pid;
}

void proc_exit(int code) {
    cli();
    struct task *t = &tasks[current_task];
    t->exit_code = code;
    t->state = TASK_ZOMBIE;
    t->pid = -1;
    num_tasks--;
    // Reparent children to init
    for (int i = 0; i < MAX_TASKS; i++) {
        if (tasks[i].state != TASK_DEAD && tasks[i].parent_pid == t->pid) {
            tasks[i].parent_pid = -1;
        }
    }
    sti();
    // Switch to next available task
    for (int i = 0; i < MAX_TASKS * 2; i++) {
        int next = (current_task + 1 + i) % MAX_TASKS;
        if (tasks[next].state == TASK_READY || tasks[next].state == TASK_RUNNING) {
            tasks[next].state = TASK_RUNNING;
            current_task = next;
            tss[0] = tasks[next].kernel_esp;
            __asm__ volatile(
                "mov %0, %%esp\n"
                "add $16, %%esp\n"
                "popa\n"
                "add $8, %%esp\n"
                "iret"
                : : "r"(tasks[next].esp) : "memory"
            );
        }
    }
    cli();
    while (1) hlt();
}

pid_t proc_wait(int *status) {
    for (int i = 0; i < MAX_TASKS; i++) {
        if (tasks[i].parent_pid == tasks[current_task].pid && tasks[i].state == TASK_ZOMBIE) {
            if (status) *status = tasks[i].exit_code;
            tasks[i].state = TASK_DEAD;
            return tasks[i].pid;
        }
    }
    return -1;
}

int proc_kill(pid_t pid) {
    if (pid <= 0) return -1;
    for (int i = 0; i < MAX_TASKS; i++) {
        if (tasks[i].pid == pid && tasks[i].state != TASK_DEAD) {
            if (i == current_task) { proc_exit(-1); return 0; }
            tasks[i].state = TASK_ZOMBIE;
            tasks[i].exit_code = -1;
            num_tasks--;
            return 0;
        }
    }
    return -1;
}

void proc_reap_zombies(void) {
    for (int i = 0; i < MAX_TASKS; i++) {
        if (tasks[i].state == TASK_ZOMBIE) {
            tasks[i].state = TASK_DEAD;
        }
    }
}

int proc_get_fd_entry(pid_t pid, int fd, struct fd_entry **fde) {
    for (int i = 0; i < MAX_TASKS; i++) {
        if (tasks[i].pid == pid || pid == -1) {
            if (fd >= 0 && fd < MAX_FDS && tasks[i].fds[fd].type != 0) {
                *fde = &tasks[i].fds[fd];
                return 0;
            }
            return -1;
        }
    }
    return -1;
}

int proc_alloc_fd(pid_t pid, int type, int ref) {
    for (int i = 0; i < MAX_TASKS; i++) {
        if (tasks[i].pid == pid) {
            for (int f = 3; f < MAX_FDS; f++) {
                if (tasks[i].fds[f].type == 0) {
                    tasks[i].fds[f].type = type;
                    tasks[i].fds[f].ref = ref;
                    tasks[i].fds[f].pos = 0;
                    tasks[i].fds[f].flags = 0;
                    return f;
                }
            }
            return -1;
        }
    }
    return -1;
}

void proc_free_fd(pid_t pid, int fd) {
    for (int i = 0; i < MAX_TASKS; i++) {
        if (tasks[i].pid == pid) {
            if (fd >= 0 && fd < MAX_FDS) {
                tasks[i].fds[fd].type = 0;
                tasks[i].fds[fd].ref = -1;
            }
            return;
        }
    }
}

void schedule(struct regs *r) {
    if (num_tasks <= 1) return;
    int cur = current_task;
    tasks[cur].esp = (uint32_t)r;
    if (tasks[cur].state == TASK_RUNNING) tasks[cur].state = TASK_READY;
    int next = cur;
    for (int pass = 0; pass < 2; pass++) {
        for (int i = 1; i < MAX_TASKS; i++) {
            int candidate = (cur + i) % MAX_TASKS;
            if (tasks[candidate].state == TASK_READY || tasks[candidate].state == TASK_RUNNING) {
                if (pass == 0 && tasks[candidate].priority < tasks[next].priority) continue;
                if (pass == 1) { next = candidate; break; }
                if (next == cur || tasks[candidate].priority > tasks[next].priority) next = candidate;
            }
        }
        if (next != cur) break;
    }
    if (next == cur) return;
    current_task = next;
    tasks[next].state = TASK_RUNNING;
    tss[0] = tasks[next].kernel_esp;
    struct task *nt = &tasks[next];
    if (nt->page_dir) switch_page_dir(nt->page_dir);
    __asm__ volatile(
        "mov %0, %%esp\n"
        "add $16, %%esp\n"
        "popa\n"
        "add $8, %%esp\n"
        "iret"
        : : "r"(nt->esp) : "memory"
    );
}

void yield(void) {
    __asm__ volatile("int $0x20");
}

int process_event_send(pid_t target_pid, uint32_t event_type, uint32_t payload_hash) {
    for (int i = 0; i < MAX_TASKS; i++) {
        if (tasks[i].pid == target_pid && tasks[i].state != TASK_DEAD) {
            cli();
            int next = (tasks[i].event_head + 1) % EVENT_QUEUE_SIZE;
            if (next == tasks[i].event_tail) { sti(); return -1; }
            hexaos_event_t *ev = &tasks[i].event_queue[tasks[i].event_head];
            ev->event_type = event_type;
            ev->sender_pid = current_task >= 0 ? tasks[current_task].pid : 0;
            ev->sender_snap = 0;
            ev->payload_hash = payload_hash;
            ev->timestamp = system_ticks;
            tasks[i].event_head = next;
            tasks[i].event_pending = 1;
            sti();
            return 0;
        }
    }
    return -1;
}

int process_event_poll(uint32_t event_type_mask, hexaos_event_t *out_event) {
    cli();
    if (tasks[current_task].event_tail == tasks[current_task].event_head) {
        tasks[current_task].event_pending = 0;
        sti();
        return -1;
    }
    hexaos_event_t *ev = &tasks[current_task].event_queue[tasks[current_task].event_tail];
    if (event_type_mask && !(ev->event_type & event_type_mask)) {
        sti();
        return -1;
    }
    if (out_event) *out_event = *ev;
    tasks[current_task].event_tail = (tasks[current_task].event_tail + 1) % EVENT_QUEUE_SIZE;
    if (tasks[current_task].event_tail == tasks[current_task].event_head)
        tasks[current_task].event_pending = 0;
    sti();
    return (int)ev->event_type;
}
