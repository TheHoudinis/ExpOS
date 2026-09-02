#include "types.h"
#include "driver.h"

struct block_device drv_block_devices[8];
struct char_device drv_char_devices[8];
static int block_count = 0;
static int char_count = 0;

int drv_register_block(struct block_device *dev) {
    if (block_count >= 8) return DRV_ERROR;
    drv_block_devices[block_count] = *dev;
    drv_block_devices[block_count].present = 1;
    block_count++;
    return DRV_OK;
}

int drv_register_char(struct char_device *dev) {
    if (char_count >= 8) return DRV_ERROR;
    drv_char_devices[char_count] = *dev;
    drv_char_devices[char_count].present = 1;
    char_count++;
    return DRV_OK;
}

struct block_device *drv_get_block(int idx) {
    if (idx < 0 || idx >= block_count) return 0;
    return &drv_block_devices[idx];
}

struct char_device *drv_get_char(int idx) {
    if (idx < 0 || idx >= char_count) return 0;
    return &drv_char_devices[idx];
}

int drv_block_count(void) { return block_count; }
int drv_char_count(void) { return char_count; }
