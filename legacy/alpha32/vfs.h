#ifndef VFS_H
#define VFS_H

#include "types.h"

// Form access modes (HexaOS native)
#define FORM_READ  0
#define FORM_WRITE 1
#define FORM_RDWR  2
#define FORM_CREAT 4
#define FORM_TRUNC 8
#define FORM_APPEND 16

// Compat aliases
#define O_RDONLY FORM_READ
#define O_WRONLY FORM_WRITE
#define O_RDWR   FORM_RDWR
#define O_CREAT  FORM_CREAT
#define O_TRUNC  FORM_TRUNC
#define O_APPEND FORM_APPEND

#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

#define FD_STDIN  0
#define FD_STDOUT 1
#define FD_STDERR 2

struct vfs_node {
    char name[32];
    int type;     // 0=form, 1=dim
    int size;
    int owner;
    uint16_t mode;
    int ref_count;
};

int vfs_init(void);
int vfs_open(const char *path, int flags);
int vfs_close(int fd);
int vfs_read(int fd, char *buf, int count);
int vfs_write(int fd, const char *buf, int count);
int vfs_lseek(int fd, int offset, int whence);
int vfs_stat(const char *path, struct vfs_node *node);
int vfs_dup(int old_fd);

#endif
