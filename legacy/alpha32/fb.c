#include "types.h"
#include "vbe.h"
#include "paging.h"
#include "log.h"

extern void *memset(void *dest, int c, size_t len);
extern void *memcpy(void *dest, const void *src, size_t len);

#define VBE_DISPI_IOPORT_INDEX 0x01CE
#define VBE_DISPI_IOPORT_DATA  0x01CF

static void vbe_write(uint16_t index, uint16_t value) {
    outw(VBE_DISPI_IOPORT_INDEX, index);
    outw(VBE_DISPI_IOPORT_DATA, value);
}

static uint16_t vbe_read(uint16_t index) {
    outw(VBE_DISPI_IOPORT_INDEX, index);
    return inw(VBE_DISPI_IOPORT_DATA);
}

fb_info_t fb_info;
static uint32_t *back_buf = 0;
static uint32_t back_buf_pixels = 0;
static int double_buf_enabled = 0;

void fb_init(void) {
    memset(&fb_info, 0, sizeof(fb_info));
    uint16_t id = vbe_read(VBE_DISPI_INDEX_ID);
    if (id == 0 || id == 0xFFFF) {
        fb_info.available = 0;
        log_write(LOG_LEVEL_INFO, "FB: no VBE2/Bochs VBE detected");
        return;
    }
    fb_info.available = 0;
    fb_info.x_res = 800;
    fb_info.y_res = 600;
    fb_info.bpp = 32;
    fb_info.pitch = 800 * 4;
    fb_info.phys_base = VBE_DISPI_LFB_PHYSICAL;
    log_write(LOG_LEVEL_INFO, "FB: Bochs VBE detected");
}

int fb_set_mode(uint16_t x, uint16_t y, uint16_t bpp) {
    vbe_write(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_DISABLED);
    vbe_write(VBE_DISPI_INDEX_XRES, x);
    vbe_write(VBE_DISPI_INDEX_YRES, y);
    vbe_write(VBE_DISPI_INDEX_BPP, bpp);
    vbe_write(VBE_DISPI_INDEX_ENABLE, VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED);
    vbe_write(VBE_DISPI_INDEX_VIRT_WIDTH, x);
    vbe_write(VBE_DISPI_INDEX_VIRT_HEIGHT, y);

    fb_info.x_res = x;
    fb_info.y_res = y;
    fb_info.bpp = bpp;
    fb_info.pitch = x * (bpp / 8);
    fb_info.available = 1;

    if (back_buf) {
        kfree(back_buf);
        back_buf = 0;
    }
    back_buf_pixels = (uint32_t)x * y;
    double_buf_enabled = 0;
    return 1;
}

void fb_clear(uint32_t color) {
    if (!fb_info.available) return;
    uint32_t *fb = double_buf_enabled && back_buf ? back_buf : (uint32_t *)(uint32_t)fb_info.phys_base;
    uint32_t pixels = back_buf_pixels;
    for (uint32_t i = 0; i < pixels; i++) fb[i] = color;
}

void fb_put_pixel(int x, int y, uint32_t color) {
    if (!fb_info.available) return;
    if (x < 0 || x >= (int)fb_info.x_res || y < 0 || y >= (int)fb_info.y_res) return;
    uint32_t *fb = double_buf_enabled && back_buf ? back_buf : (uint32_t *)(uint32_t)fb_info.phys_base;
    fb[y * (fb_info.pitch / 4) + x] = color;
}

void fb_put_rect(int x, int y, int w, int h, uint32_t color) {
    if (!fb_info.available) return;
    uint32_t *fb = double_buf_enabled && back_buf ? back_buf : (uint32_t *)(uint32_t)fb_info.phys_base;
    uint32_t pitch4 = fb_info.pitch / 4;
    for (int j = y; j < y + h && j < (int)fb_info.y_res; j++)
        for (int i = x; i < x + w && i < (int)fb_info.x_res; i++)
            fb[j * pitch4 + i] = color;
}

void fb_double_buf(void) {
    if (!fb_info.available) return;
    if (back_buf) return;
    back_buf = (uint32_t *)kmalloc(back_buf_pixels * 4);
    if (!back_buf) {
        log_write(LOG_LEVEL_WARN, "FB: double buffer alloc failed");
        return;
    }
    memcpy(back_buf, (void *)(uint32_t)fb_info.phys_base, back_buf_pixels * 4);
    double_buf_enabled = 1;
    log_write(LOG_LEVEL_INFO, "FB: double buffer enabled");
}

void fb_swap(void) {
    if (!fb_info.available || !back_buf) return;
    memcpy((void *)(uint32_t)fb_info.phys_base, back_buf, back_buf_pixels * 4);
}

int fb_available(void) {
    return fb_info.available;
}
