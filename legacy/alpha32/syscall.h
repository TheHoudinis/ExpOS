#ifndef SYSCALL_H
#define SYSCALL_H

#include "types.h"
#include "interrupts.h"

#define SYS_PRINT      0
#define SYS_READ       1
#define SYS_EXIT       2
#define SYS_TICKS      3
#define SYS_OPEN       4
#define SYS_CLOSE      5
#define SYS_WRITE      6
#define SYS_GETPID     7
#define SYS_SLEEP      8
#define SYS_BRK        9
#define SYS_WAITPID    10
#define SYS_KILL       11
#define SYS_PIPE       12
#define SYS_DUP        13
#define SYS_GETPPID    14
#define SYS_SYSNAME    15
#define SYS_LSEEK      16
#define SYS_GETCWD     17
#define SYS_STAT       18
#define SYS_MMAP       19
#define SYS_MUNMAP     20

#define SYS_INTENT     21
#define SYS_FULFILL    22
#define SYS_DIFF       23
#define SYS_REPLAY     24
#define SYS_PIPE_TYPED 25
#define SYS_EVENT_SEND 26
#define SYS_EVENT_POLL 27

#define SYSCALL_MAX    28

#define SYS_OK        0
#define SYS_EINVAL   -1
#define SYS_EBADF    -2
#define SYS_ENOMEM   -3
#define SYS_EPERM    -4
#define SYS_ENOENT   -5
#define SYS_ECHILD   -6
#define SYS_EAGAIN   -7
#define SYS_ENOSYS   -8

int sys_validate_str(const char *str, int max_len);
int sys_validate_buf(void *buf, int len);

uint32_t syscall_handler(struct regs *r);

#endif
