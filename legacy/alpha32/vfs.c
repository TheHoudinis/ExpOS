#include "types.h"
#include "vfs.h"
#include "process.h"
#include "pipe.h"

// Hook into hexa.c's form table
extern int form_count;
extern int find_form(const char *name);
extern int check_perm(int idx, int want_write);
extern int form_ensure_cap(int idx, int needed);

// String functions from hexa.c
extern char *strcpy(char *dest, const char *src);
extern size_t strlen(const char *str);

// hexa.c form table structure (dynamic content)
struct hexa_form {
  char name[32];
  char *content;
  int size;
  int cap;
  int owner;
  uint16_t mode;
};
extern struct hexa_form form_table[];

// FD table for the whole system
#define VFS_MAX_FDS 128

static struct {
    int pid;
    int type;   // 0=free, 1=form, 2=pipe, 3=console
    int ref;
    int pos;
    int flags;
} vfs_fds[VFS_MAX_FDS];

static int vfs_fd_alloc(int pid) {
    for (int i = 3; i < VFS_MAX_FDS; i++) {
        if (vfs_fds[i].type == 0) {
            vfs_fds[i].pid = pid;
            vfs_fds[i].type = 1;
            vfs_fds[i].pos = 0;
            vfs_fds[i].flags = 0;
            return i;
        }
    }
    return -1;
}

int vfs_init(void) {
    for (int i = 0; i < VFS_MAX_FDS; i++) vfs_fds[i].type = 0;
    return 0;
}

int vfs_open(const char *path, int flags) {
    int idx = find_form(path);
    if (idx < 0) {
        if (!(flags & FORM_CREAT)) return -1;
        // Create form via mkform
        extern void cmd_mkform(const char *name);
        cmd_mkform(path);
        idx = find_form(path);
        if (idx < 0) return -1;
    }
    if (check_perm(idx, flags & FORM_WRITE)) return -1;
    int fd = vfs_fd_alloc(current_task);
    if (fd < 0) return -1;
    vfs_fds[fd].type = 1;
    vfs_fds[fd].ref = idx;
    vfs_fds[fd].pos = (flags & FORM_APPEND) ? 0x7FFFFFFF : 0;
    vfs_fds[fd].flags = flags;
    return fd;
}

int vfs_close(int fd) {
    if (fd < 0 || fd >= VFS_MAX_FDS || vfs_fds[fd].type == 0) return -1;
    vfs_fds[fd].type = 0;
    return 0;
}

int vfs_read(int fd, char *buf, int count) {
    if (fd < 0 || fd >= VFS_MAX_FDS || vfs_fds[fd].type == 0) return -1;
    if (vfs_fds[fd].type == 2) {
        return pipe_read(vfs_fds[fd].ref, buf, count);
    }
    if (vfs_fds[fd].type == 3) {
        extern int kb_getchar(void);
        for (int i = 0; i < count; i++) {
            int c;
            do { c = kb_getchar(); } while (c < 0);
            buf[i] = (char)c;
            if (c == '\n') return i + 1;
        }
        return count;
    }
    if (vfs_fds[fd].type != 1) return -1;
    int idx = vfs_fds[fd].ref;
    if (idx < 0 || idx >= form_count) return -1;
    int pos = vfs_fds[fd].pos;
    int n = 0;
    while (n < count && pos + n < form_table[idx].size) {
        buf[n] = form_table[idx].content[pos + n];
        n++;
    }
    vfs_fds[fd].pos += n;
    return n;
}

int vfs_write(int fd, const char *buf, int count) {
    if (fd < 0 || fd >= VFS_MAX_FDS || vfs_fds[fd].type == 0) return -1;
    if (vfs_fds[fd].type == 2) {
        return pipe_write(vfs_fds[fd].ref, buf, count);
    }
    if (vfs_fds[fd].type == 3) {
        extern void print_string(const char *str);
        for (int i = 0; i < count; i++) {
            extern void put_char(char c, uint8_t color);
            put_char(buf[i], 0x0F);
        }
        return count;
    }
    if (vfs_fds[fd].type != 1) return -1;
    int idx = vfs_fds[fd].ref;
    if (idx < 0 || idx >= form_count) return -1;
    int pos = vfs_fds[fd].pos;
    if (!form_ensure_cap(idx, pos + count + 1)) return -1;
    int n = 0;
    while (n < count && pos + n < 65528) {
        form_table[idx].content[pos + n] = buf[n];
        n++;
    }
    form_table[idx].content[pos + n] = '\0';
    if (pos + n > form_table[idx].size) form_table[idx].size = pos + n;
    vfs_fds[fd].pos += n;
    return n;
}

int vfs_lseek(int fd, int offset, int whence) {
    if (fd < 0 || fd >= VFS_MAX_FDS || vfs_fds[fd].type != 1) return -1;
    switch (whence) {
        case SEEK_SET: vfs_fds[fd].pos = offset; break;
        case SEEK_CUR: vfs_fds[fd].pos += offset; break;
        case SEEK_END:
        default: vfs_fds[fd].pos = 0; break;
    }
    if (vfs_fds[fd].pos < 0) vfs_fds[fd].pos = 0;
    return vfs_fds[fd].pos;
}

int vfs_stat(const char *path, struct vfs_node *node) {
    if (!path || !node) return -1;
    int idx = find_form(path);
    if (idx < 0) return -1;
    strcpy(node->name, form_table[idx].name);
    node->type = 0;
    int len = strlen(node->name);
    if (len > 0 && node->name[len-1] == '/') node->type = 1;
    node->size = form_table[idx].size;
    node->owner = form_table[idx].owner;
    node->mode = form_table[idx].mode;
    node->ref_count = 1;
    return 0;
}

int vfs_dup(int old_fd) {
    if (old_fd < 0 || old_fd >= VFS_MAX_FDS || vfs_fds[old_fd].type == 0) return -1;
    int new_fd = vfs_fd_alloc(current_task);
    if (new_fd < 0) return -1;
    vfs_fds[new_fd] = vfs_fds[old_fd];
    return new_fd;
}
