#ifndef DRIVER_H
#define DRIVER_H

#include "types.h"

#define DRV_OK       0
#define DRV_ERROR   -1
#define DRV_BUSY    -2
#define DRV_NODEV   -3

struct block_device {
    char name[16];
    int (*read)(uint32_t lba, uint16_t *buf);
    int (*write)(uint32_t lba, const uint16_t *buf);
    int (*ioctl)(int cmd, void *arg);
    uint32_t num_blocks;
    int present;
};

struct char_device {
    char name[16];
    int (*read)(char *buf, int len);
    int (*write)(const char *buf, int len);
    int present;
};

int drv_register_block(struct block_device *dev);
int drv_register_char(struct char_device *dev);
struct block_device *drv_get_block(int idx);
struct char_device *drv_get_char(int idx);
int drv_block_count(void);
int drv_char_count(void);

extern struct block_device drv_block_devices[8];
extern struct char_device drv_char_devices[8];

#endif
