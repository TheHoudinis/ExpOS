#ifndef VBE_H
#define VBE_H

#include "types.h"

#define VBE_DISPI_IOPORT_INDEX 0x01CE
#define VBE_DISPI_IOPORT_DATA  0x01CF

#define VBE_DISPI_INDEX_ID      0
#define VBE_DISPI_INDEX_XRES    1
#define VBE_DISPI_INDEX_YRES    2
#define VBE_DISPI_INDEX_BPP     3
#define VBE_DISPI_INDEX_ENABLE  4
#define VBE_DISPI_INDEX_BANK    5
#define VBE_DISPI_INDEX_VIRT_WIDTH 6
#define VBE_DISPI_INDEX_VIRT_HEIGHT 7
#define VBE_DISPI_INDEX_X_OFFSET 8
#define VBE_DISPI_INDEX_Y_OFFSET 9

#define VBE_DISPI_DISABLED      0x00
#define VBE_DISPI_ENABLED       0x01
#define VBE_DISPI_LFB_ENABLED   0x40
#define VBE_DISPI_NOCLEARMEM    0x80

#define VBE_DISPI_LFB_PHYSICAL  0xFD000000

#define VBE_DISPI_BANK_SIZE_KB  64

#define VBE_MODE_800x600x32    0x115
#define VBE_MODE_1024x768x32   0x118

typedef struct __attribute__((packed)) {
    uint32_t phys_base;
    uint16_t x_res;
    uint16_t y_res;
    uint16_t bpp;
    uint16_t pitch;
    uint8_t  available;
} fb_info_t;

extern fb_info_t fb_info;

void fb_init(void);
void fb_clear(uint32_t color);
void fb_put_pixel(int x, int y, uint32_t color);
void fb_put_rect(int x, int y, int w, int h, uint32_t color);
int  fb_set_mode(uint16_t x, uint16_t y, uint16_t bpp);
void fb_double_buf(void);
void fb_swap(void);
int  fb_available(void);

#endif
