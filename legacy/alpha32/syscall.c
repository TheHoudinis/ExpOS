#include "types.h"
#include "syscall.h"
#include "interrupts.h"
#include "process.h"
#include "vfs.h"
#include "pipe.h"
#include "log.h"
#include "paging.h"
#include "intent.h"
#include "replay.h"
#include "hexafs.h"

extern void print_string(const char *str);
extern void print_color(const char *str, uint8_t color);
extern int kb_getchar(void);
extern volatile uint32_t system_ticks;
extern void sleep_ticks(uint32_t t);

int sys_validate_str(const char *str, int max_len) {
    if (!str) return -1;
    for (int i = 0; i < max_len; i++) {
        if (str[i] == 0) return 0;
    }
    return -1;
}

int sys_validate_buf(void *buf, int len) {
    (void)buf;
    if (len <= 0 || len > 4096) return -1;
    if (!buf) return -1;
    return 0;
}

static void sys_print(const char *str) {
    if (sys_validate_str(str, 256) == 0) print_string(str);
}

static int sys_read(char *buf, int len) {
    if (sys_validate_buf(buf, len) < 0) return SYS_EINVAL;
    if (len < 1) return 0;
    int c;
    do { c = kb_getchar(); } while (c < 0);
    buf[0] = (char)c;
    if (len > 1 && c != '\n') { buf[1] = '\n'; return 2; }
    return 1;
}

static int sys_open(const char *path, int flags) {
    if (sys_validate_str(path, 256) < 0) return SYS_EINVAL;
    return intent_compat_open(path, flags);
}

static int sys_close(int fd) {
    return vfs_close(fd);
}

static int sys_write(int fd, const char *buf, int len) {
    if (sys_validate_buf((void *)buf, len) < 0) return SYS_EINVAL;
    return intent_compat_write(fd, buf, len, 0);
}

static int sys_getpid(void) {
    return tasks[current_task].pid;
}

static int sys_getppid(void) {
    return tasks[current_task].parent_pid;
}

static int sys_waitpid(int *status) {
    return proc_wait(status);
}

static int sys_kill(pid_t pid) {
    return process_event_send(pid, EVENT_TERMINATE, 0);
}

static int sys_pipe(int *fds) {
    if (sys_validate_buf(fds, 8) < 0) return SYS_EINVAL;
    int rfd, wfd;
    if (pipe_create(&rfd, &wfd) < 0) return SYS_EAGAIN;
    fds[0] = rfd;
    fds[1] = wfd;
    return 0;
}

static int sys_dup(int old_fd) {
    return vfs_dup(old_fd);
}

static int sys_brk(uint32_t addr) {
    if (addr == 0) return tasks[current_task].heap_brk;
    if (addr < 0x40000000 || addr > 0x80000000) return SYS_EINVAL;
    tasks[current_task].heap_brk = addr;
    return addr;
}

static int sys_lseek(int fd, int offset, int whence) {
    return vfs_lseek(fd, offset, whence);
}

static int sys_sysname(char *buf) {
    if (sys_validate_buf(buf, 64) < 0) return SYS_EINVAL;
    const char *name = "HexaOS";
    for (int i = 0; name[i] && i < 63; i++) buf[i] = name[i];
    return 0;
}

static int sys_mmap(uint32_t addr, uint32_t len) {
    if (len == 0) return SYS_EINVAL;
    len = (len + 0xFFF) & ~0xFFF;
    if (addr == 0) addr = 0x40000000;
    for (uint32_t v = addr; v < addr + len; v += 4096) {
        uint32_t phys = pmm_alloc();
        if (!phys) return SYS_ENOMEM;
        extern void map_page(uint32_t, uint32_t, uint32_t);
        map_page(v, phys, 0x007);
    }
    return addr;
}

static int sys_munmap(uint32_t addr, uint32_t len) {
    len = (len + 0xFFF) & ~0xFFF;
    for (uint32_t v = addr; v < addr + len; v += 4096) {
        extern void unmap_page(uint32_t);
        unmap_page(v);
    }
    return 0;
}

static int sys_intent(uint32_t arg1, uint32_t arg2, uint32_t arg3) {
    hexaos_intent_t *intent = (hexaos_intent_t *)arg1;
    uint32_t *handle = (uint32_t *)arg2;
    (void)arg3;
    if (!intent || !handle) return SYS_EINVAL;

    if (!hexafs_cap_check(tasks[current_task].pid, CAP_TYPE_INTENT)) {
        if (intent->intent_type != INTENT_CONSUME && tasks[current_task].pid != 0)
            return SYS_EPERM;
    }

    uint32_t h;
    if (intent_create(tasks[current_task].pid, intent, &h) < 0) return SYS_EAGAIN;
    *handle = h;
    replay_record(SYS_INTENT, arg1, arg2, arg3, 0);
    return SYS_OK;
}

static int sys_fulfill(uint32_t handle, void *buffer, int len) {
    replay_record(SYS_FULFILL, handle, (uint32_t)buffer, (uint32_t)len, 0);
    return intent_fulfill(handle, buffer, len);
}

static int sys_diff(uint32_t arg1, uint32_t arg2, uint32_t arg3) {
    (void)arg1;
    (void)arg2;
    (void)arg3;
    return SYS_ENOSYS;
}

static int sys_replay(uint32_t snap_id, uint32_t event_count, uint32_t flags) {
    (void)flags;
    return replay_execute(snap_id, event_count);
}

static int sys_pipe_typed(uint32_t schema_hash, uint32_t producer_pid, uint32_t consumer_pid) {
    uint32_t cap_token = 0;
    return pipe_create_typed(schema_hash, producer_pid, consumer_pid, cap_token);
}

static int sys_event_send(uint32_t target_pid, uint32_t event_type, uint32_t payload_hash) {
    if (target_pid == 0) return SYS_EINVAL;
    int ret = process_event_send((pid_t)target_pid, event_type, payload_hash);
    if (ret < 0) return SYS_EAGAIN;
    return SYS_OK;
}

static int sys_event_poll(uint32_t event_type_mask, uint32_t timeout_ticks) {
    hexaos_event_t ev;
    uint32_t start = system_ticks;
    while (1) {
        int ret = process_event_poll(event_type_mask, &ev);
        if (ret >= 0) {
            if (ev.event_type == EVENT_TERMINATE) {
                proc_exit(0);
                return 0;
            }
            return ret;
        }
        if (timeout_ticks > 0 && (system_ticks - start) >= timeout_ticks)
            return 0;
        __asm__ volatile("pause");
    }
}

uint32_t syscall_handler(struct regs *r) {
    uint32_t num = r->eax;
    uint32_t arg1 = r->ebx;
    uint32_t arg2 = r->ecx;
    uint32_t arg3 = r->edx;

    if (num >= SYSCALL_MAX) return SYS_ENOSYS;

    switch (num) {
        case SYS_PRINT:
            sys_print((const char *)arg1);
            return SYS_OK;
        case SYS_READ:
            return sys_read((char *)arg1, (int)arg2);
        case SYS_EXIT:
            proc_exit((int)arg1);
            return SYS_OK;
        case SYS_TICKS:
            return system_ticks;
        case SYS_OPEN:
            return sys_open((const char *)arg1, (int)arg2);
        case SYS_CLOSE:
            return sys_close((int)arg1);
        case SYS_WRITE:
            return sys_write((int)arg1, (const char *)arg2, (int)arg3);
        case SYS_GETPID:
            return sys_getpid();
        case SYS_SLEEP:
            sleep_ticks(arg1);
            return SYS_OK;
        case SYS_BRK:
            return sys_brk(arg1);
        case SYS_WAITPID:
            return sys_waitpid((int *)arg1);
        case SYS_KILL:
            return sys_kill((pid_t)arg1);
        case SYS_PIPE:
            return sys_pipe((int *)arg1);
        case SYS_DUP:
            return sys_dup((int)arg1);
        case SYS_GETPPID:
            return sys_getppid();
        case SYS_SYSNAME:
            return sys_sysname((char *)arg1);
        case SYS_LSEEK:
            return sys_lseek((int)arg1, (int)arg2, (int)arg3);
        case SYS_MMAP:
            return sys_mmap(arg1, arg2);
        case SYS_GETCWD: {
            char *buf = (char *)arg1;
            int len = (int)arg2;
            if (!buf || len <= 0) return SYS_EINVAL;
            const char *path = "/home/";
            int plen = 0;
            while (path[plen] && plen < len - 1) { buf[plen] = path[plen]; plen++; }
            buf[plen] = 0;
            return plen;
        }
        case SYS_STAT: {
            struct vfs_node node;
            int ret = intent_compat_stat((const char *)arg1, &node);
            if (ret < 0) return ret;
            if (arg2) {
                struct vfs_node *out = (struct vfs_node *)arg2;
                *out = node;
            }
            return 0;
        }
        case SYS_MUNMAP:
            return sys_munmap(arg1, arg2);
        case SYS_INTENT:
            return sys_intent(arg1, arg2, arg3);
        case SYS_FULFILL:
            return sys_fulfill(arg1, (void *)arg2, (int)arg3);
        case SYS_DIFF:
            return sys_diff(arg1, arg2, arg3);
        case SYS_REPLAY:
            return sys_replay(arg1, arg2, arg3);
        case SYS_PIPE_TYPED:
            return sys_pipe_typed(arg1, arg2, arg3);
        case SYS_EVENT_SEND:
            return sys_event_send(arg1, arg2, arg3);
        case SYS_EVENT_POLL:
            return sys_event_poll(arg1, arg2);
        default:
            return SYS_ENOSYS;
    }
}
