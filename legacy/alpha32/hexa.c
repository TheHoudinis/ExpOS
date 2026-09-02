#include "types.h"
#include "interrupts.h"
#include "paging.h"
#include "process.h"
#include "vfs.h"
#include "hexafs.h"
#include "pipe.h"
#include "sync.h"
#include "log.h"
#include "driver.h"
#include "elf.h"
#include "kobserve.h"
#include "boot_policy.h"
#include "intent.h"
#include "replay.h"
#include "hex.h"
#include "net.h"
#include "vbe.h"

#define VGA_BUFFER ((uint16_t *)0xB8000)

extern char _bss_start[], _bss_end[];

static int cursor_x = 0;
static int cursor_y = 0;
static uint8_t current_color = 0x0F; // White on black

// ---- User & File System ----
#define MAX_USERS 12
#define MAX_FORMS 64
#define NAME_MAX 32
#define CONTENT_MAX 65528

// Forward declarations for password hash functions
static uint32_t pwd_hash(const char *str, uint32_t salt, int iters);
static void encode_pwd(char out[NAME_MAX], const char *pass, uint32_t salt);
static int check_pwd(const char *pass, const char *encoded);

static struct {
  char name[NAME_MAX];
  char pass_hash[NAME_MAX];
  int is_root;
} u_table[MAX_USERS];
static int u_count = 0;
static int u_cur = 0;

// Permission bits: owner (bits 6-8), group (3-5), other (0-2)
// r=4, w=2, x=1
#define PERM_OWNER_R 0x100
#define PERM_OWNER_W 0x080
#define PERM_OWNER_X 0x040
#define PERM_GRP_R   0x020
#define PERM_GRP_W   0x010
#define PERM_GRP_X   0x008
#define PERM_OTH_R   0x004
#define PERM_OTH_W   0x002
#define PERM_OTH_X   0x001
#define PERM_DEFAULT 0x1A4  // owner rw-, grp r--, oth r--

int form_ensure_cap(int idx, int needed);

struct hexa_formentry {
  char name[NAME_MAX];
  char *content;
  int size;
  int cap;
  int owner;
  uint16_t mode;
} form_table[MAX_FORMS];
int form_count = 0;
int disk_ok = 0;

void *memcpy(void *dest, const void *src, size_t len);
void *memset(void *dest, int c, size_t len);

// Ensure form's content buffer can hold 'needed' bytes (grow if needed)
int form_ensure_cap(int idx, int needed) {
  if (needed <= form_table[idx].cap) return 1;
  int newcap = needed + 1024;
  char *new = kmalloc(newcap);
  if (!new) return 0;
  if (form_table[idx].content) {
    memcpy(new, form_table[idx].content, form_table[idx].size);
    kfree(form_table[idx].content);
  }
  form_table[idx].content = new;
  form_table[idx].cap = newcap;
  return 1;
}

// ----------------- String Library -----------------
int strcmp(const char *s1, const char *s2) {
  while (*s1 && (*s1 == *s2)) {
    s1++;
    s2++;
  }
  return *(const unsigned char *)s1 - *(const unsigned char *)s2;
}

char *strcpy(char *dest, const char *src) {
  char *ret = dest;
  while ((*dest++ = *src++))
    ;
  return ret;
}

size_t strlen(const char *str) {
  size_t len = 0;
  while (str[len])
    len++;
  return len;
}

void *memset(void *dest, int val, size_t len) {
  unsigned char *ptr = (unsigned char *)dest;
  while (len-- > 0)
    *ptr++ = val;
  return dest;
}

void *memcpy(void *dest, const void *src, size_t len) {
  unsigned char *d = (unsigned char *)dest;
  const unsigned char *s = (const unsigned char *)src;
  while (len-- > 0)
    *d++ = *s++;
  return dest;
}

char *strcat(char *dest, const char *src) {
  char *ret = dest;
  while (*dest) dest++;
  while ((*dest++ = *src++));
  return ret;
}

int memcmp(const void *s1, const void *s2, size_t n) {
  const unsigned char *a = (const unsigned char *)s1;
  const unsigned char *b = (const unsigned char *)s2;
  for (size_t i = 0; i < n; i++) {
    if (a[i] != b[i]) return a[i] - b[i];
  }
  return 0;
}

int atoi(const char *str) {
  int res = 0, sign = 1, i = 0;
  if (str[0] == '-') {
    sign = -1;
    i++;
  }
  for (; str[i] != '\0'; ++i) {
    if (str[i] >= '0' && str[i] <= '9')
      res = res * 10 + str[i] - '0';
    else
      break;
  }
  return sign * res;
}

void itoa(int num, char *str, int base) {
  int i = 0, isNegative = 0;
  unsigned int unum;
  if (num == 0) {
    str[i++] = '0';
    str[i] = '\0';
    return;
  }
  if (base == 10) {
    if (num < 0) { isNegative = 1; unum = (unsigned int)(-num); }
    else { unum = (unsigned int)num; }
  } else {
    unum = (unsigned int)num;
  }
  while (unum != 0) {
    unsigned int rem = unum % (unsigned int)base;
    str[i++] = (rem > 9) ? (rem - 10) + 'A' : rem + '0';
    unum = unum / (unsigned int)base;
  }
  if (isNegative)
    str[i++] = '-';
  str[i] = '\0';
  int start = 0, end = i - 1;
  while (start < end) {
    char t = str[start];
    str[start] = str[end];
    str[end] = t;
    start++;
    end--;
  }
}

// ----------------- Password Hashing (v7.2 - 16-bit salt + iterations) -----------------
static uint32_t pwd_hash(const char *str, uint32_t salt, int iters) {
  uint32_t h = 5381 + salt;
  int c;
  while ((c = *str++))
    h = ((h << 5) + h) + (unsigned char)c;
  for (int i = 1; i < iters; i++)
    h = ((h << 5) + h) + (h >> 16) + (h & 0xFF);
  return h;
}

static int hex_val(char c) {
  if (c >= '0' && c <= '9') return c - '0';
  if (c >= 'a' && c <= 'f') return c - 'a' + 10;
  if (c >= 'A' && c <= 'F') return c - 'A' + 10;
  return 0;
}

static void encode_pwd(char out[NAME_MAX], const char *pass, uint32_t salt) {
  char tmp[16];
  int iters = 5;
  // Format: "SSSS+IHHHHHHHH" — 4-hex salt, '+', 1-hex iters, hash
  uint32_t h = pwd_hash(pass, salt, iters);
  itoa(salt, tmp, 16);
  int sl = strlen(tmp);
  out[0] = '0'; out[1] = '0'; out[2] = '0'; out[3] = '0';
  for (int i = 0; i < sl && i < 4; i++)
    out[3 - i] = tmp[sl - 1 - i];
  out[4] = '+';
  out[5] = "0123456789ABCDEF"[iters & 0xF];
  itoa(h, tmp, 16);
  strcpy(out + 6, tmp);
}

static int check_pwd(const char *pass, const char *encoded) {
  if (encoded[4] == '+') {
    // New format: "SSSS+IHHHHHHHH"
    uint32_t salt = 0;
    for (int i = 0; i < 4; i++) salt = salt * 16 + hex_val(encoded[i]);
    int iters = hex_val(encoded[5]);
    uint32_t sh = 0;
    const char *hp = encoded + 6;
    while (*hp) { sh = sh * 16 + hex_val(*hp); hp++; }
    return pwd_hash(pass, salt, iters) == sh;
  }
  // Old format (v7.2-): "SS HHHHHHHH" — 2-hex salt, space, 1-iteration hash
  // Check for space at pos 2 as old format marker, bail on garbage
  if (encoded[2] != ' ') return 0;
  uint32_t salt = 0;
  for (int i = 0; i < 2; i++) salt = salt * 16 + hex_val(encoded[i]);
  uint32_t sh = 0;
  const char *hp = encoded + 3;
  while (*hp) { sh = sh * 16 + hex_val(*hp); hp++; }
  return pwd_hash(pass, salt, 1) == sh;
}

// ----------------- Hardware Drivers -----------------

// CPUID
static inline void cpuid(uint32_t code, uint32_t *a, uint32_t *d) {
  __asm__ volatile("cpuid" : "=a"(*a), "=d"(*d) : "a"(code) : "ecx", "ebx");
}

static inline void cpuid_string(uint32_t code, uint32_t where[4]) {
  __asm__ volatile("cpuid"
                   : "=a"(*where), "=b"(*(where + 1)), "=c"(*(where + 2)),
                     "=d"(*(where + 3))
                   : "a"(code));
}

// CMOS RTC
int get_update_in_progress_flag() {
  outb(0x70, 0x0A);
  return (inb(0x71) & 0x80);
}

uint8_t get_rtc_register(int reg) {
  outb(0x70, reg);
  return inb(0x71);
}

// PCI
uint32_t pci_config_read_word(uint8_t bus, uint8_t slot, uint8_t func,
                              uint8_t offset) {
  uint32_t address;
  uint32_t lbus = (uint32_t)bus;
  uint32_t lslot = (uint32_t)slot;
  uint32_t lfunc = (uint32_t)func;
  address = (uint32_t)((lbus << 16) | (lslot << 11) | (lfunc << 8) |
                       (offset & 0xFC) | ((uint32_t)0x80000000));
  outl(0xCF8, address);
  return (uint32_t)((inl(0xCFC) >> ((offset & 2) * 8)) & 0xFFFF);
}

// Forward declarations for NIC driver + exec
void print_string(const char *str);
void print_color(const char *str, uint8_t color);
void do_tick(void);
extern volatile uint32_t system_ticks;
int find_form(const char *name);
int check_perm(int idx, int want_write);
pid_t proc_create_user(uint32_t entry, const char *name);

static void map_page_in_dir(uint32_t pd_phys, uint32_t virt, uint32_t phys, uint32_t flags) {
    uint32_t pd_idx = virt >> 22;
    uint32_t pt_idx = (virt >> 12) & 0x3FF;
    uint32_t *pd = (uint32_t *)pd_phys;
    if (!(pd[pd_idx] & 1)) {
        uint32_t pt_phys = pmm_alloc();
        uint32_t *pt = (uint32_t *)pt_phys;
        for (int i = 0; i < 1024; i++) pt[i] = 0x002;
        pd[pd_idx] = pt_phys | 0x003;
    }
    uint32_t *pt = (uint32_t *)(pd[pd_idx] & ~0xFFF);
    pt[pt_idx] = (phys & ~0xFFF) | (flags & 0xFFF);
}

static void cmd_exec(const char *args) {
  if (!args[0]) { print_string("Usage: exec <elf_form>\n"); return; }
  int idx = find_form(args);
  if (idx < 0) { print_string("exec: form not found\n"); return; }
  if (check_perm(idx, 0)) { print_color("Denied.\n", 0x0C); return; }
  if ((uint32_t)form_table[idx].size < sizeof(struct elf_header)) { print_string("exec: invalid ELF\n"); return; }
  struct elf_header *hdr = (struct elf_header *)form_table[idx].content;
  if (!elf_validate(hdr)) { print_string("exec: not a valid ELF binary\n"); return; }
  uint32_t user_pd = create_user_page_dir();
  if (!user_pd) { print_string("exec: page dir alloc failed\n"); return; }
  struct elf_prog_header *ph = (struct elf_prog_header *)((uint32_t)hdr + hdr->phoff);
  for (int i = 0; i < hdr->phnum; i++) {
    if (ph[i].type == PT_LOAD) {
      uint32_t start_virt = ph[i].vaddr & ~0xFFF;
      uint32_t end_virt = (ph[i].vaddr + ph[i].memsz + 0xFFF) & ~0xFFF;
      for (uint32_t v = start_virt; v < end_virt; v += PAGE_SIZE) {
        uint32_t phys = pmm_alloc();
        if (!phys) { print_string("exec: out of memory\n"); return; }
        map_page_in_dir(user_pd, v, phys, 0x007);
      }
    }
  }
  uint32_t saved_pd;
  __asm__ volatile("mov %%cr3, %0" : "=r"(saved_pd));
  switch_page_dir(user_pd);
  uint32_t entry = 0, heap_start = 0;
  int ret = elf_load(hdr, &entry, &heap_start);
  switch_page_dir(saved_pd);
  if (ret < 0) { print_string("exec: ELF load failed\n"); return; }
  pid_t pid = proc_create_user(entry, args);
  if (pid < 0) { print_string("exec: task creation failed\n"); return; }
  print_string("exec: started "); print_string(args);
  char buf[16];
  print_string(" (pid "); itoa(pid, buf, 10); print_string(buf); print_string(")\n");
}

void update_cursor() {
  uint16_t pos = cursor_y * 80 + cursor_x;
  outb(0x3D4, 0x0F);
  outb(0x3D5, (uint8_t)(pos & 0xFF));
  outb(0x3D4, 0x0E);
  outb(0x3D5, (uint8_t)((pos >> 8) & 0xFF));
}

static inline void serial_putc(char c) {
  for (int i = 0; i < 10000; i++) {
    if (inb(0x3F8 + 5) & 0x20) break;
    __asm__ volatile("pause");
  }
  outb(0x3F8, c);
}

void put_char(char c, uint8_t color) {
  if (c == '\b') {
    if (cursor_x > 0)
      cursor_x--;
    else if (cursor_y > 0) {
      cursor_y--;
      cursor_x = 79;
    }
    VGA_BUFFER[cursor_y * 80 + cursor_x] = ((uint16_t)color << 8) | ' ';
    serial_putc('\b');
    serial_putc(' ');
    serial_putc('\b');
    update_cursor();
    return;
  }
  serial_putc(c);
  if (c == '\n') {
    cursor_x = 0;
    cursor_y++;
    serial_putc('\r');
  } else if (c == '\r') {
    cursor_x = 0;
  } else {
    VGA_BUFFER[cursor_y * 80 + cursor_x] = ((uint16_t)color << 8) | (uint8_t)c;
    cursor_x++;
  }

  if (cursor_x >= 80) {
    cursor_x = 0;
    cursor_y++;
  }
  if (cursor_y >= 25) {
    for (int i = 0; i < 24 * 80; i++)
      VGA_BUFFER[i] = VGA_BUFFER[i + 80];
    for (int i = 24 * 80; i < 25 * 80; i++)
      VGA_BUFFER[i] = ((uint16_t)color << 8) | ' ';
    cursor_y = 24;
  }
  update_cursor();
}

void print_string(const char *str) {
  while (*str) {
    put_char(*str, current_color);
    str++;
  }
}

void print_color(const char *str, uint8_t color) {
  uint8_t old = current_color;
  current_color = color;
  print_string(str);
  current_color = old;
}

void clear_screen(void) {
  for (int i = 0; i < 80 * 25; i++)
    VGA_BUFFER[i] = ((uint16_t)current_color << 8) | ' ';
  cursor_x = 0;
  cursor_y = 0;
  const char *ca = "\033[2J\033[H";
  while (*ca)
    serial_putc(*ca++);
}

// ----------------- Keyboard Driver & Shell Input -----------------
const char kbd_map[128] = {
    0,    27,   '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '-',  '=',
    '\b', '\t', 'q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p', '[',  ']',
    '\n', 0,    'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', ';', '\'', '`',
    0,    '\\', 'z', 'x', 'c', 'v', 'b', 'n', 'm', ',', '.', '/', 0,    '*',
    0,    ' ',  0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,    0,
    0,    0,    0,   0,   '-', 0,   0,   0,   '+', 0,   0,   0,   0,    0,
    0,    0,    0,   0,   0,   0,   0,   0,   0,   0};
const char kbd_map_shift[128] = {
    0,    27,   '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '_', '+',
    '\b', '\t', 'Q', 'W', 'E', 'R', 'T', 'Y', 'U', 'I', 'O', 'P', '{', '}',
    '\n', 0,    'A', 'S', 'D', 'F', 'G', 'H', 'J', 'K', 'L', ':', '"', '~',
    0,    '|',  'Z', 'X', 'C', 'V', 'B', 'N', 'M', '<', '>', '?', 0,   '*',
    0,    ' ',  0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
    0,    0,    0,   0,   '-', 0,   0,   0,   '+', 0,   0,   0,   0,   0,
    0,    0,    0,   0,   0,   0,   0,   0,   0,   0};
static int shift_pressed = 0;
#define HIST_MAX 16
static char hist_buf[HIST_MAX][128];
static int hist_idx = 0;
static int hist_count = 0;
static int hist_scroll = -1;

char get_char(int *special_key) {
  if (special_key)
    *special_key = 0;
  while (1) {
    // Check serial port
    if (inb(0x3F8 + 5) & 1) {
      char c = inb(0x3F8);
      if (c == '\r')
        c = '\n';
      if (c == 127)
        c = '\b';
      if (c == '\033') { // Handle arrow keys from serial
        char b = inb(0x3F8);
        if (b == '[') {
          char d = inb(0x3F8);
          if (d == 'A' && special_key) {
            *special_key = 1;
            return 0;
          }
          if (d == 'B' && special_key) {
            *special_key = 2;
            return 0;
          }
        }
      }
      return c;
    }
    // Check IRQ keyboard buffer
    int c = kb_getchar();
    if (c > 0) {
      // Map special high-bit values from IRQ keyboard
      if (c == 0x80 && special_key) { *special_key = 1; return 0; }
      if (c == 0x81 && special_key) { *special_key = 2; return 0; }
      return (char)c;
    }
    // Poll keyboard directly as fallback
    if (inb(0x64) & 1) {
      uint8_t sc = inb(0x60);
      if (sc == 0x2A || sc == 0x36) {
        shift_pressed = 1;
        continue;
      }
      if (sc == 0xAA || sc == 0xB6) {
        shift_pressed = 0;
        continue;
      }
      if (sc == 0x48 && special_key) {
        *special_key = 1;
        return 0;
      }
      if (sc == 0x50 && special_key) {
        *special_key = 2;
        return 0;
      }
      if (!(sc & 0x80) && sc < 128) {
        char c = shift_pressed ? kbd_map_shift[sc] : kbd_map[sc];
        if (c)
          return c;
      }
    }
  }
}

void get_line(char *buffer, size_t max_len) {
  size_t i = 0;
  while (1) {
    int special = 0;
    char c = get_char(&special);
    if (special == 1) { // UP arrow
      if (hist_count == 0 || hist_scroll >= hist_count - 1) continue;
      hist_scroll++;
      while (i > 0) { put_char('\b', current_color); i--; }
      int idx = (hist_idx - hist_scroll - 1 + HIST_MAX) % HIST_MAX;
      size_t hlen = strlen(hist_buf[idx]);
      for (size_t k = 0; k < hlen && k < max_len - 1; k++) {
        buffer[k] = hist_buf[idx][k];
        put_char(buffer[k], current_color);
      }
      i = hlen; buffer[hlen] = '\0';
      continue;
    }
    if (special == 2) { // DOWN arrow
      while (i > 0) { put_char('\b', current_color); i--; }
      if (hist_scroll > 0) {
        hist_scroll--;
        int idx = (hist_idx - hist_scroll - 1 + HIST_MAX) % HIST_MAX;
        size_t hlen = strlen(hist_buf[idx]);
        for (size_t k = 0; k < hlen && k < max_len - 1; k++) {
          buffer[k] = hist_buf[idx][k];
          put_char(buffer[k], current_color);
        }
        i = hlen; buffer[hlen] = '\0';
      } else {
        hist_scroll = -1;
        i = 0; buffer[0] = '\0';
      }
      continue;
    }
    if (c == '\n') {
      print_string("\n");
      buffer[i] = '\0';
      if (i > 0) hist_scroll = -1;
      break;
    } else if (c == '\b') {
      if (i > 0) { i--; put_char('\b', current_color); }
    } else {
      if (i < max_len - 1) { buffer[i++] = c; put_char(c, current_color); }
    }
  }
}

static uint32_t ticks = 0;
static char hostname_str[32] = "hexaos";
void do_tick() {
  for (int i = 0; i < 200; i++) {
    __asm__ volatile("pause");
  }
  ticks++;
}
void sleep_ticks(uint32_t t) {
  uint32_t target = ticks + t;
  while (ticks < target)
    do_tick();
}

// Simple PRNG
static uint32_t rseed = 0;
uint32_t rand() {
  rseed = rseed * 1103515245 + 12345;
  return (rseed / 65536) % 32768;
}

// ----------------- Shell Commands -----------------
int find_form(const char *name);
static int execute_cmd(const char *cmd, char *args);

void cmd_help() {
  print_string("HEXA OS 7.2 Commands\n");
  print_string("------------------------------------\n");
  print_string(" System:  help, clear, reboot, halt, panic, sleep, shutdown\n");
  print_string("          uptime\n");
  print_string(" HW:      date, cpuinfo, lspci, outb, inb, neofetch\n");
  print_string(" Str:     echo, len, hex, reverse, tolower, toupper\n");
  print_string("          morse\n");
  print_string(" Apps:    calc, rand, ascii, palette, matrix, guess\n");
  print_string("          exec\n");
  print_string(" Forms:   mkform, view, list, delete, write, append, edit\n");
  print_string("          move, copy, dimpath, makedim, setmode, setowner\n");
  print_string(" Users:   login, logout, useradd, passwd, whoami\n");
  print_string("          diese\n");
  if (find_form(".fun") >= 0)
    print_string(" Fun:     banner, fortune, yes, time, cowsay, cmatrix\n"
                 "          logo, sl, dice, 8ball, russian, insult\n"
                 "          excuse, compliment, hack, bsod\n");
  if (find_form(".games") >= 0)
    print_string(" Games:   snake, tictactoe, hangman, memory, tetris\n");
  print_string(" Info:    sysname, uptime, about, mem, beep, history\n");
  print_string("          dmesg, free, sysinfo, kstat, netstat\n");
  print_string("          caps, inbox, sendevent, pipes, bootlog\n");
  print_string("          ps, df, which, env\n");
  print_string("          cat, head\n");
  print_string(" Pimp:    pimp, diese, dieselist\n");
  print_string(" Video:   mode list/set/double/clear/color\n");
  print_string("------------------------------------\n");
  print_string(" Pkg mgmt: ayo help\n");
}

void cmd_kaput() { print_string("Kaput, is this kaput?\n"); }

void cmd_date() {
  while (get_update_in_progress_flag())
    ;
  uint8_t s = get_rtc_register(0x00);
  uint8_t m = get_rtc_register(0x02);
  uint8_t h = get_rtc_register(0x04);
  uint8_t d = get_rtc_register(0x07);
  uint8_t mo = get_rtc_register(0x08);
  uint8_t y = get_rtc_register(0x09);
  uint8_t rtc_b = get_rtc_register(0x0B);
  if (!(rtc_b & 0x04)) {
    s = (s & 0x0F) + ((s / 16) * 10);
    m = (m & 0x0F) + ((m / 16) * 10);
    h = ((h & 0x0F) + (((h & 0x70) / 16) * 10)) | (h & 0x80);
    d = (d & 0x0F) + ((d / 16) * 10);
    mo = (mo & 0x0F) + ((mo / 16) * 10);
    y = (y & 0x0F) + ((y / 16) * 10);
  }
  char buf[16];
  print_string("Date: 20");
  itoa(y, buf, 10);
  print_string(buf);
  print_string("-");
  itoa(mo, buf, 10);
  print_string(buf);
  print_string("-");
  itoa(d, buf, 10);
  print_string(buf);
  print_string(" Time: ");
  itoa(h, buf, 10);
  print_string(buf);
  print_string(":");
  itoa(m, buf, 10);
  print_string(buf);
  print_string(":");
  itoa(s, buf, 10);
  print_string(buf);
  print_string("\n");
}

void cmd_cpuinfo() {
  uint32_t a, d;
  cpuid(1, &a, &d);
  uint32_t brand[4];
  cpuid_string(0, brand);
  print_string("CPU Vendor: ");
  char v[13];
  ((uint32_t *)v)[0] = brand[1];
  ((uint32_t *)v)[1] = brand[3];
  ((uint32_t *)v)[2] = brand[2];
  v[12] = 0;
  print_string(v);
  print_string("\n");
  char buf[16];
  itoa((a >> 8) & 0xF, buf, 10);
  print_string("Family: ");
  print_string(buf);
  itoa((a >> 4) & 0xF, buf, 10);
  print_string(" Model: ");
  print_string(buf);
  print_string("\n");
}

void cmd_lspci() {
  print_string("Scanning PCI bus (first 10 devices)...\n");
  int count = 0;
  for (uint16_t bus = 0; bus < 5; bus++) {
    for (uint8_t slot = 0; slot < 32; slot++) {
      uint16_t vendor = pci_config_read_word(bus, slot, 0, 0);
      if (vendor != 0xFFFF) {
        uint16_t device = pci_config_read_word(bus, slot, 0, 2);
        char buf[16];
        print_string("Bus ");
        itoa(bus, buf, 10);
        print_string(buf);
        print_string(" Slot ");
        itoa(slot, buf, 10);
        print_string(buf);
        print_string(": Vendor=");
        itoa(vendor, buf, 16);
        print_string(buf);
        print_string(" Device=");
        itoa(device, buf, 16);
        print_string(buf);
        print_string("\n");
        count++;
        if (count > 10)
          return;
      }
    }
  }
}

void cmd_calc(const char *args) {
  char a1[16] = {0}, op = 0, a2[16] = {0};
  int i = 0, j = 0;
  while (args[i] && args[i] != ' ')
    a1[j++] = args[i++];
  while (args[i] == ' ')
    i++;
  op = args[i++];
  while (args[i] == ' ')
    i++;
  j = 0;
  while (args[i])
    a2[j++] = args[i++];
  if (!op || !a1[0] || !a2[0]) {
    print_string("Usage: calc <num> <+|-|*|/> <num>\n");
    return;
  }
  int n1 = atoi(a1), n2 = atoi(a2), res = 0;
  if (op == '+')
    res = n1 + n2;
  else if (op == '-')
    res = n1 - n2;
  else if (op == '*')
    res = n1 * n2;
  else if (op == '/') {
    if (n2 == 0) {
      print_string("Div by zero!\n");
      return;
    }
    res = n1 / n2;
  } else {
    print_string("Invalid op\n");
    return;
  }
  char buf[32];
  itoa(res, buf, 10);
  print_string("= ");
  print_string(buf);
  print_string("\n");
}

void cmd_ascii() {
  for (int i = 32; i < 127; i++) {
    put_char(i, current_color);
    put_char(' ', current_color);
    if ((i - 31) % 16 == 0)
      print_string("\n");
  }
  print_string("\n");
}

void cmd_palette() {
  for (int i = 0; i < 16; i++) {
    char buf[16];
    itoa(i, buf, 10);
    uint8_t old = current_color;
    current_color = i;
    print_string("Color ");
    print_string(buf);
    print_string("  ");
    current_color = old;
    if (i == 7)
      print_string("\n");
  }
  print_string("\n");
}

void cmd_hex(const char *args) {
  char buf[16];
  itoa(atoi(args), buf, 16);
  print_string("0x");
  print_string(buf);
  print_string("\n");
}

void cmd_reverse(const char *args) {
  int len = strlen(args);
  for (int i = len - 1; i >= 0; i--)
    put_char(args[i], current_color);
  print_string("\n");
}

void cmd_matrix() {
  print_string("Press any key to exit matrix...\n");
  sleep_ticks(500); // 1s
  while (1) {
    if ((inb(0x64) & 1) || (inb(0x3F8 + 5) & 1)) {
      get_char(0);
      break;
    }
    put_char((rand() % 94) + 33, 0x0A); // green chars
    do_tick();
    do_tick();
  }
  print_string("\n");
}

void cmd_guess() {
  int secret = (rand() % 10) + 1;
  print_string("Guess a number between 1 and 10!\n");
  char buf[32];
  while (1) {
    print_string("guess> ");
    get_line(buf, 32);
    if (buf[0] == 'q')
      break;
    int g = atoi(buf);
    if (g == secret) {
      print_color("Correct!\n", 0x0A);
      break;
    } else if (g < secret)
      print_string("Too low.\n");
    else
      print_string("Too high.\n");
  }
}

void cmd_panic(void) {
  if (u_cur != 0) { print_color("Root only.\n", 0x0C); print_string("Use: diese panic\n"); return; }
  clear_screen();
  for (int i = 0; i < 25; i++) {
    for (int j = 0; j < 80; j++)
      VGA_BUFFER[i * 80 + j] = (0x1F << 8) | ' ';
  }
  cursor_x = 0; cursor_y = 0;
  print_color("*** KERNEL PANIC ***\n", 0xCF);
  print_color("====================\n\n", 0xCF);
  print_color("A fatal exception has occurred. The system will halt.\n\n", 0xCF);
  // Stage 1: Register dump
  print_color("[STAGE 1/5] Dumping CPU registers...\n", 0xCE);
  for (volatile int d = 0; d < 15000; d++);
  char buf[16];
  print_color("  EAX=0x", 0x4F); itoa(0xDEADBEEF, buf, 16); print_color(buf, 0x4F);
  print_color("  EBX=0x", 0x4F); itoa(0xCAFEBABE, buf, 16); print_color(buf, 0x4F);
  print_color("  ECX=0x", 0x4F); itoa(0x600DF00D, buf, 16); print_color(buf, 0x4F);
  print_color("  EDX=0x", 0x4F); itoa(0xBAADF00D, buf, 16); print_color(buf, 0x4F);
  print_color("\n  ESI=0x", 0x4F); itoa(0xFEEDFACE, buf, 16); print_color(buf, 0x4F);
  print_color("  EDI=0x", 0x4F); itoa(0xDEADC0DE, buf, 16); print_color(buf, 0x4F);
  print_color("  EBP=0x", 0x4F); itoa(0x8BADF00D, buf, 16); print_color(buf, 0x4F);
  print_color("  ESP=0x", 0x4F); itoa(0x0000BEEF, buf, 16); print_color(buf, 0x4F);
  print_color("\n  EIP=0x", 0x4F); itoa(0xC0FFEEEE, buf, 16); print_color(buf, 0x4F);
  print_color("  CS=0x08  DS=0x10  SS=0x18  FLAGS=0x", 0x4F);
  itoa(0x00010246, buf, 16); print_color(buf, 0x4F);
  print_color("\n\n", 0x4F);
  for (volatile int d = 0; d < 20000; d++);
  // Stage 2: Stack trace
  print_color("[STAGE 2/5] Generating stack trace...\n", 0xCE);
  for (volatile int d = 0; d < 15000; d++);
  const char *stack_msgs[] = {
    "  [0xDEADBEEF] panic()+0x3F",
    "  [0xCAFEBABE] syscall_handler()+0x12",
    "  [0x600DF00D] do_tick()+0x87",
    "  [0xBAADF00D] keyboard_irq()+0x2B",
    "  [0xFEEDFACE] scheduler()+0x45",
    "  [0xDEADC0DE] switch_task()+0x19",
    "  [0x8BADF00D] timer_callback()+0x0E",
    "  [0xC0FFEEEE] interrupt_dispatch()+0x5A",
    "  [0x0000BEEF] main()+0xFF",
    "  [0x00010246] _start()+0x03",
  };
  for (int i = 0; i < 10; i++) {
    print_color(stack_msgs[i], 0x4F);
    print_color("\n", 0x4F);
    for (volatile int d = 0; d < 5000; d++);
  }
  // Stage 3: Memory map
  print_color("\n[STAGE 3/5] Dumping memory map...\n", 0xCE);
  for (volatile int d = 0; d < 15000; d++);
  print_color("  Physical memory: 32 MB total\n", 0x4F);
  print_color("  Used pages: ", 0x4F); itoa(pmm_count_free(), buf, 10); print_color(buf, 0x4F);
  print_color("/", 0x4F); itoa(pmm_get_total_pages(), buf, 10); print_color(buf, 0x4F);
  print_color("\n  Kernel heap: 0x800000 - 0x900000\n", 0x4F);
  print_color("  Stack canary: 0xDEADBEEF (", 0x4F);
  print_color("CORRUPTED", 0x4C);
  print_color(")\n", 0x4F);
  for (volatile int d = 0; d < 20000; d++);
  // Stage 4: Scan form store
  print_color("\n[STAGE 4/5] Scanning form store for corruption...\n", 0xCE);
  for (volatile int d = 0; d < 15000; d++);
  const char *scan[] = {
    "  [OK]  Boot sector: intact",
    "  [OK]  Kernel image: checksum valid",
    "  [!!]  form_table[3]: invalid form block",
    "  [!!]  Block 0x47: CRC mismatch",
    "  [OK]  Page directory: intact",
    "  [!!]  GDT entry 5: segment limit exceeded",
    "  [OK]  IDT entries: all valid",
    "  [!!]  TSS: stale selector",
    "  [OK]  ATA controller: responding",
    "  [OK]  PIC: cascaded correctly",
  };
  for (int i = 0; i < 10; i++) {
    if (scan[i][4] == 'O')
      print_color(scan[i], 0x0A);
    else
      print_color(scan[i], 0x4C);
    print_color("\n", 0x4F);
    for (volatile int d = 0; d < 6000; d++);
  }
  // Stage 5: Critical error
  print_color("\n[STAGE 5/5] Fatal: kernel panic initiated\n", 0xCE);
  for (volatile int d = 0; d < 15000; d++);
  print_color("\n  >> Press any key to attempt recovery, or wait for shutdown <<\n", 0xCF);
  for (volatile int d = 0; d < 10000; d++);
  if ((inb(0x64) & 1)) {
    get_char(0);
    print_color("\nRecovery attempted. System may be unstable.\n", 0x0E);
    print_color("Press any key to continue...\n", 0x0E);
    get_char(0);
    clear_screen();
    return;
  }
  // Dramatic countdown
  print_color("\nShutting down in ", 0xCF);
  for (int i = 5; i > 0; i--) {
    itoa(i, buf, 10); print_color(buf, 0xCF);
    print_color("... ", 0xCF);
    for (volatile int d = 0; d < 30000; d++);
  }
  print_color("NOW.\n\n", 0x4F);
  for (volatile int d = 0; d < 20000; d++);
  // Final screen flash
  for (int i = 0; i < 25; i++) {
    for (int j = 0; j < 80; j++)
      VGA_BUFFER[i * 80 + j] = (0x40 << 8) | ' ';
  }
  for (volatile int d = 0; d < 30000; d++);
  __asm__ volatile("cli; hlt");
}

// ---- Non-blocking input helpers for games ----
int kb_hit(void) {
  return (inb(0x64) & 1) || (inb(0x3F8 + 5) & 1);
}

char getch_nb(void) {
  if (inb(0x3F8 + 5) & 1) {
    char c = inb(0x3F8);
    if (c == '\r') c = '\n';
    if (c == 127) c = '\b';
    return c;
  }
  if (inb(0x64) & 1) {
    uint8_t sc = inb(0x60);
    if (sc == 0x2A || sc == 0x36) { shift_pressed = 1; return 0; }
    if (sc == 0xAA || sc == 0xB6) { shift_pressed = 0; return 0; }
    if (!(sc & 0x80) && sc < 128) return shift_pressed ? kbd_map_shift[sc] : kbd_map[sc];
  }
  return 0;
}

// ----- SNAKE GAME -----
void cmd_snake(void) {
  if (find_form(".games") < 0) {
    print_string("Package required: ayo add games\n");
    print_string("(use 'diese' if not root)\n");
    return;
  }
  int ox = 20, oy = 2, gw = 40, gh = 20;
  int sx[256], sy[256], slen = 3, sdir = 1, score = 0;
  int fx, fy, go = 0, t = 0;
  sx[0] = 5; sy[0] = 10; sx[1] = 4; sy[1] = 10; sx[2] = 3; sy[2] = 10;
  int ok;
  do {
    ok = 1; fx = rand() % (gw - 2) + 1; fy = rand() % (gh - 2) + 1;
    for (int i = 0; i < slen; i++) if (sx[i] == fx && sy[i] == fy) { ok = 0; break; }
  } while (!ok);

  clear_screen();
  for (int x = 0; x < gw; x++) {
    VGA_BUFFER[oy * 80 + ox + x] = 0x0F << 8 | '#';
    VGA_BUFFER[(oy + gh - 1) * 80 + ox + x] = 0x0F << 8 | '#';
  }
  for (int y = 0; y < gh; y++) {
    VGA_BUFFER[(oy + y) * 80 + ox] = 0x0F << 8 | '#';
    VGA_BUFFER[(oy + y) * 80 + ox + gw - 1] = 0x0F << 8 | '#';
  }
  cursor_x = 0; cursor_y = 0;
  print_string("SNAKE - WASD move, Q quit\n");

  while (!go) {
    do_tick(); t++;
    if (kb_hit()) {
      char c = getch_nb();
      if ((c == 'w' || c == 'W') && sdir != 2) sdir = 0;
      else if ((c == 's' || c == 'S') && sdir != 0) sdir = 2;
      else if ((c == 'a' || c == 'A') && sdir != 1) sdir = 3;
      else if ((c == 'd' || c == 'D') && sdir != 3) sdir = 1;
      else if (c == 'q' || c == 'Q') go = 2;
    }
    if (t % 4 != 0) continue;

    int nx = sx[0], ny = sy[0];
    if (sdir == 0) ny--; else if (sdir == 1) nx++; else if (sdir == 2) ny++; else nx--;
    if (nx <= 0 || nx >= gw - 1 || ny <= 0 || ny >= gh - 1) { go = 1; break; }
    for (int i = 0; i < slen; i++) if (nx == sx[i] && ny == sy[i]) { go = 1; break; }
    if (go) break;

    for (int i = slen - 1; i > 0; i--) { sx[i] = sx[i - 1]; sy[i] = sy[i - 1]; }
    sx[0] = nx; sy[0] = ny;

    if (nx == fx && ny == fy) {
      score += 10; slen++;
      if (slen >= 256) { go = 1; break; }
      sx[slen - 1] = sx[slen - 2]; sy[slen - 1] = sy[slen - 2];
      do {
        ok = 1; fx = rand() % (gw - 2) + 1; fy = rand() % (gh - 2) + 1;
        for (int i = 0; i < slen; i++) if (sx[i] == fx && sy[i] == fy) { ok = 0; break; }
      } while (!ok);
    }

    for (int y = 1; y < gh - 1; y++)
      for (int x = 1; x < gw - 1; x++)
        VGA_BUFFER[(oy + y) * 80 + ox + x] = 0x0F << 8 | ' ';
    VGA_BUFFER[(oy + fy) * 80 + ox + fx] = 0x0C << 8 | '@';
    for (int i = 0; i < slen; i++)
      VGA_BUFFER[(oy + sy[i]) * 80 + ox + sx[i]] = 0x0A << 8 | 'O';
  }

  clear_screen();
  if (go == 1) {
    char buf[16];
    itoa(score, buf, 10);
    print_string("Game Over! Score: ");
    print_string(buf);
    print_string("\n");
  }
  print_string("Press any key to continue...");
  get_char(0);
  clear_screen();
}

// ----- TETRIS GAME -----
// ----- TETRIS (v7.2 rewrite — SRS ghost/next/hold/wall-kicks) -----
#define TET_TW 10
#define TET_TH 20
#define TET_TOX 28
#define TET_TOY 1

static const uint16_t TET_SH[7][4]={
  {0x0F00,0x2222,0x0F00,0x2222},
  {0x6600,0x6600,0x6600,0x6600},
  {0x4E00,0x4640,0x0E40,0x4C40},
  {0x6C00,0x8C40,0x6C00,0x8C40},
  {0xC600,0x4C80,0xC600,0x4C80},
  {0x8E00,0x6440,0x0E20,0x44C0},
  {0x2E00,0x4460,0x0E80,0xC440},
};
static const int TET_CO[7]={0x0B,0x0E,0x0D,0x0A,0x0C,0x09,0x07};
static const char TET_CH[7]={178,219,177,176,254,248,216};

static int tet_b[TET_TH][TET_TW];
static int tet_px,tet_py,tet_t,tet_r,tet_sc,tet_ln,tet_lv,tet_go,tet_dc;
static int tet_nx[4],tet_hd,tet_hu;

static int tet_col(int t,int r,int bx,int by){
  uint16_t m=TET_SH[t][r];
  for(int y=0;y<4;y++)for(int x=0;x<4;x++)
    if((m>>(15-(y*4+x)))&1){int nx=bx+x,ny=by+y;
    if(nx<0||nx>=TET_TW||ny>=TET_TH)return 1;
    if(ny>=0&&tet_b[ny][nx])return 1;}
  return 0;
}

static int tet_wk(int t,int r,int *px,int *py){
  if(!tet_col(t,r,*px,*py))return 1;
  int k[][2]={{-1,0},{1,0},{0,-1},{-1,-1},{1,-1},{-2,0},{2,0}};
  for(int i=0;i<7;i++){int nx=*px+k[i][0],ny=*py+k[i][1];
    if(!tet_col(t,r,nx,ny)){*px=nx;*py=ny;return 1;}}
  return 0;
}

static void tet_lk(void){
  uint16_t m=TET_SH[tet_t][tet_r];
  for(int y=0;y<4;y++)for(int x=0;x<4;x++)
    if((m>>(15-(y*4+x)))&1){int bx=tet_px+x,by=tet_py+y;
    if(by>=0&&by<TET_TH&&bx>=0&&bx<TET_TW)tet_b[by][bx]=tet_t+1;}
}

static int tet_cl(void){
  int c=0;
  for(int y=0;y<TET_TH;y++){int f=1;
    for(int x=0;x<TET_TW;x++)if(!tet_b[y][x]){f=0;break;}
    if(f){c++;for(int ky=y;ky>0;ky--)for(int kx=0;kx<TET_TW;kx++)tet_b[ky][kx]=tet_b[ky-1][kx];
    for(int kx=0;kx<TET_TW;kx++)tet_b[0][kx]=0;}}
  return c;
}

static int tet_dd(void){
  int gy=tet_py;while(!tet_col(tet_t,tet_r,tet_px,gy+1))gy++;
  return gy-tet_py;
}

static void tet_dw(void){
  for(int y=0;y<TET_TH;y++)for(int x=0;x<TET_TW;x++){
    int v=tet_b[y][x];
    uint16_t*c=&VGA_BUFFER[(TET_TOY+1+y)*80+TET_TOX+1+x];
    if(v)*c=(TET_CO[(v-1)%7]<<8)|TET_CH[(v-1)%7];else *c=0x0000;}
  int gy=tet_py+tet_dd();
  uint16_t gm=TET_SH[tet_t][tet_r];
  for(int y=0;y<4;y++)for(int x=0;x<4;x++)
    if((gm>>(15-(y*4+x)))&1){int sy=gy+y,sx=tet_px+x;
      if(sy>=0&&sy<TET_TH&&sx>=0&&sx<TET_TW&&!tet_b[sy][sx])
        VGA_BUFFER[(TET_TOY+1+sy)*80+TET_TOX+1+sx]=((TET_CO[tet_t]>>1|4)<<8)|'.';}
  uint16_t pm=TET_SH[tet_t][tet_r];
  for(int y=0;y<4;y++)for(int x=0;x<4;x++)
    if((pm>>(15-(y*4+x)))&1){int sy=tet_py+y,sx=tet_px+x;
      if(sy>=0&&sy<TET_TH&&sx>=0&&sx<TET_TW)
        VGA_BUFFER[(TET_TOY+1+sy)*80+TET_TOX+1+sx]=(TET_CO[tet_t]<<8)|TET_CH[tet_t];}
  char b[16];
  cursor_x=TET_TW+6;cursor_y=2;print_string("SCORE");
  cursor_x=TET_TW+6;cursor_y=3;itoa(tet_sc,b,10);print_string(b);print_string("   ");
  cursor_x=TET_TW+6;cursor_y=5;print_string("LINES");
  cursor_x=TET_TW+6;cursor_y=6;itoa(tet_ln,b,10);print_string(b);print_string("   ");
  cursor_x=TET_TW+6;cursor_y=8;print_string("LEVEL");
  cursor_x=TET_TW+6;cursor_y=9;itoa(tet_lv,b,10);print_string(b);print_string("   ");
  cursor_x=TET_TW+6;cursor_y=11;print_string("NEXT");
  for(int i=0;i<3;i++){uint16_t nm=TET_SH[tet_nx[i]][0];
    for(int y=0;y<4;y++)for(int x=0;x<4;x++){
      uint16_t*c=&VGA_BUFFER[(TET_TOY+6+i*4+y)*80+TET_TOX+18+x];
      if((nm>>(15-(y*4+x)))&1)*c=(TET_CO[tet_nx[i]]<<8)|TET_CH[tet_nx[i]];
      else *c=0x0000;}}
  if(tet_hd>=0){
    cursor_x=TET_TW+6;cursor_y=16;print_string("HOLD");
    uint16_t hm=TET_SH[tet_hd][0];
    for(int y=0;y<4;y++)for(int x=0;x<4;x++){
      uint16_t*c=&VGA_BUFFER[(TET_TOY+10+y)*80+TET_TOX+18+x];
      if((hm>>(15-(y*4+x)))&1)*c=(TET_CO[tet_hd]<<8)|TET_CH[tet_hd];
      else *c=0x0000;}}
}

void cmd_tetris(void){
  if(find_form(".games")<0){print_string("Package required: ayo add games\n");
    print_string("(use 'diese' if not root)\n");return;}
  memset(tet_b,0,sizeof(tet_b));
  tet_sc=0;tet_ln=0;tet_lv=1;tet_go=0;tet_dc=0;tet_hd=-1;tet_hu=0;
  clear_screen();
  for(int i=0;i<4;i++)tet_nx[i]=rand()%7;
  tet_t=tet_nx[0];tet_r=0;tet_px=(TET_TW-4)/2;tet_py=0;
  for(int i=0;i<3;i++)tet_nx[i]=tet_nx[i+1];
  tet_nx[3]=rand()%7;
  for(int x=0;x<TET_TW+2;x++){
    VGA_BUFFER[TET_TOY*80+TET_TOX+x]=0x0F<<8|'#';
    VGA_BUFFER[(TET_TOY+TET_TH+1)*80+TET_TOX+x]=0x0F<<8|'#';}
  for(int y=0;y<TET_TH+2;y++){
    VGA_BUFFER[(TET_TOY+y)*80+TET_TOX]=0x0F<<8|'#';
    VGA_BUFFER[(TET_TOY+y)*80+TET_TOX+TET_TW+1]=0x0F<<8|'#';}
  cursor_x=37;cursor_y=0;print_string("TETRIS v7.2");
  cursor_x=0;cursor_y=23;
  print_string("A:Left D:Right W:Rot C:Hold Space:HardDrop");
  while(!tet_go){
    do_tick();tet_dc++;
    if(kb_hit()){
      char c=getch_nb();
      if(c=='a'||c=='A'){if(!tet_col(tet_t,tet_r,tet_px-1,tet_py))tet_px--;}
      else if(c=='d'||c=='D'){if(!tet_col(tet_t,tet_r,tet_px+1,tet_py))tet_px++;}
      else if(c=='w'||c=='W'){int nr=(tet_r+1)&3;int nx=tet_px,ny=tet_py;
        if(tet_wk(tet_t,nr,&nx,&ny)){tet_r=nr;tet_px=nx;tet_py=ny;}}
      else if(c=='s'||c=='S'){if(!tet_col(tet_t,tet_r,tet_px,tet_py+1)){tet_py++;tet_dc=0;tet_sc++;}}
      else if(c==' '){int d=tet_dd();tet_py+=d;tet_dc=999;tet_sc+=d*2;}
      else if(c=='c'||c=='C'){
        if(!tet_hu){int tmp=tet_hd;tet_hd=tet_t;tet_hu=1;
          if(tmp>=0){tet_t=tmp;tet_r=0;tet_px=(TET_TW-4)/2;tet_py=0;}
          else{tet_t=tet_nx[0];tet_r=0;tet_px=(TET_TW-4)/2;tet_py=0;
            for(int i=0;i<3;i++)tet_nx[i]=tet_nx[i+1];
            tet_nx[3]=rand()%7;}}}
      else if(c=='q'||c=='Q'){tet_go=1;break;}
    }
    int ds=30-tet_lv*2;if(ds<2)ds=2;
    if(tet_dc>=ds){tet_dc=0;
      if(!tet_col(tet_t,tet_r,tet_px,tet_py+1)){tet_py++;}
      else{tet_lk();int cl=tet_cl();
        if(cl>0){tet_ln+=cl;tet_sc+=cl*100*tet_lv;
          if(cl>=4)tet_sc+=400*tet_lv;
          tet_lv=(tet_ln/5)+1;}
        tet_t=tet_nx[0];tet_r=0;tet_px=(TET_TW-4)/2;tet_py=0;tet_hu=0;
        for(int i=0;i<3;i++)tet_nx[i]=tet_nx[i+1];
        tet_nx[3]=rand()%7;
        if(tet_col(tet_t,tet_r,tet_px,tet_py)){tet_go=2;break;}}}
    tet_dw();
  }
  clear_screen();
  if(tet_go==2){char buf[16];
    print_string("GAME OVER! Score: ");itoa(tet_sc,buf,10);print_string(buf);
    print_string("  Lines: ");itoa(tet_ln,buf,10);print_string(buf);
    print_string("  Level: ");itoa(tet_lv,buf,10);print_string(buf);
    print_string("  (any key)\n");}
  else print_string("Quit.\nPress any key...");
  get_char(0);clear_screen();
}

// ----- TIC-TAC-TOE -----
void cmd_tictactoe(void) {
  if (find_form(".games") < 0) {
    print_string("Package required: ayo add games\n"); return;
  }
  char board[9] = {'1','2','3','4','5','6','7','8','9'};
  int turn = 0, pos, win = 0, moves = 0;
  char line[16];

  while (!win && moves < 9) {
    clear_screen();
    print_string("Tic-Tac-Toe\n");
    print_string("Player 1 (X) | Player 2 (O)\n\n");
    for (int i = 0; i < 9; i += 3) {
      print_string(" ");
      put_char(board[i], 0x0F); print_string(" | ");
      put_char(board[i+1], 0x0F); print_string(" | ");
      put_char(board[i+2], 0x0F); print_string("\n");
      if (i < 6) print_string("---|---|---\n");
    }
    print_string("\nPlayer ");
    put_char(turn ? 'O' : 'X', 0x0F);
    print_string(", pick a position (1-9): ");
    get_line(line, 16);
    pos = atoi(line);
    if (pos < 1 || pos > 9 || board[pos-1] == 'X' || board[pos-1] == 'O') continue;

    board[pos-1] = turn ? 'O' : 'X';
    moves++;

    for (int i = 0; i < 3; i++) {
      if (board[i*3] == board[i*3+1] && board[i*3+1] == board[i*3+2]) { win = 1; break; }
      if (board[i] == board[i+3] && board[i+3] == board[i+6]) { win = 1; break; }
    }
    if (board[0] == board[4] && board[4] == board[8]) win = 1;
    if (board[2] == board[4] && board[4] == board[6]) win = 1;

    if (!win) turn = !turn;
  }

  clear_screen();
  if (win) {
    print_string("Player ");
    put_char(turn ? 'O' : 'X', 0x0F);
    print_string(" wins!\n");
  } else {
    print_string("It's a draw!\n");
  }
  print_string("Press any key to continue...");
  get_char(0);
  clear_screen();
}

// ----- MEMORY (Simon Says) -----
void cmd_memory(void) {
  if (find_form(".games") < 0) {
    print_string("Package required: ayo add games\n"); return;
  }
  char colors[] = {'R','G','B','Y'};
  char seq[32];
  char input[32];
  int len = 1, score = 0, go = 0;

  while (!go && len <= 20) {
    seq[len - 1] = colors[rand() % 4];
    clear_screen();
    print_string("MEMORY GAME - Watch the sequence!\n\n");
    for (int i = 0; i < len; i++) {
      uint8_t col;
      switch (seq[i]) {
        case 'R': col = 0x0C; break;
        case 'G': col = 0x0A; break;
        case 'B': col = 0x09; break;
        default:  col = 0x0E; break;
      }
      put_char(seq[i], col);
      put_char(' ', 0x0F);
      sleep_ticks(3000);
    }
    clear_screen();
    print_string("Your turn! Enter the sequence (letters R,G,B,Y):\n");
    get_line(input, 32);

    for (int i = 0; input[i]; i++)
      if (input[i] >= 'a' && input[i] <= 'z') input[i] -= 32;

    int ok = 1;
    for (int i = 0; i < len; i++) {
      if (input[i] != seq[i]) { ok = 0; break; }
      if (input[i] == '\0') { ok = 0; break; }
    }
    if (!ok) go = 1;
    else { score = len; len++; }
  }

  clear_screen();
  char buf[16];
  itoa(score, buf, 10);
  print_string("Memory score: ");
  print_string(buf);
  print_string(" rounds\nPress any key to continue...");
  get_char(0);
  clear_screen();
}

// ----- HANGMAN -----
void cmd_hangman(void) {
  if (find_form(".games") < 0) {
    print_string("Package required: ayo add games\n"); return;
  }
  const char *words[] = {
    "apple", "banana", "cherry", "dragon", "eagle",
    "flame", "grape", "house", "igloo", "joker",
    "knight", "lemon", "mango", "noble", "ocean",
    "piano", "queen", "river", "stone", "tiger"
  };
  int nwords = 20;
  int idx = rand() % nwords;
  const char *word = words[idx];
  int wlen = strlen(word);
  char guessed[16] = {0};
  int wrong = 0, max_wrong = 6;
  char display[32], buf[16];
  int win = 0;

  while (wrong < max_wrong) {
    for (int i = 0; i < wlen; i++) {
      int found = 0;
      for (int j = 0; guessed[j]; j++)
        if (guessed[j] == word[i]) { found = 1; break; }
      display[i * 2] = found ? word[i] : '_';
      display[i * 2 + 1] = ' ';
    }
    display[wlen * 2] = '\0';

    clear_screen();
    print_string("HANGMAN - Guess the word!\n\n");
    print_string("Word: ");
    print_string(display);
    print_string("\n\nWrong: ");
    itoa(wrong, buf, 10); print_string(buf);
    print_string("/6  Guessed: ");
    print_string(guessed);
    print_string("\n\nGuess a letter: ");

    char g = get_char(0);
    if (g >= 'A' && g <= 'Z') g += 32;
    if (g < 'a' || g > 'z') continue;

    int already = 0;
    for (int i = 0; guessed[i]; i++)
      if (guessed[i] == g) { already = 1; break; }
    if (already) continue;

    int glen = strlen(guessed);
    guessed[glen] = g;
    guessed[glen + 1] = '\0';

    int found = 0;
    for (int i = 0; i < wlen; i++)
      if (word[i] == g) { found = 1; break; }
    if (!found) wrong++;

    int all_found = 1;
    for (int i = 0; i < wlen; i++) {
      int f = 0;
      for (int j = 0; guessed[j]; j++)
        if (guessed[j] == word[i]) { f = 1; break; }
      if (!f) { all_found = 0; break; }
    }
    if (all_found) { win = 1; break; }
  }

  clear_screen();
  if (win) {
    print_string("You win! The word was: ");
    print_string(word);
    print_string("\n");
  } else {
    print_string("Game over! The word was: ");
    print_string(word);
    print_string("\n");
  }
  print_string("Press any key to continue...");
  get_char(0);
  clear_screen();
}

// ----- NEOFETCH (System Info) -----
void cmd_neofetch(void) {
  uint32_t brand[4], a, d;
  cpuid_string(0, brand);
  char v[13];
  ((uint32_t *)v)[0] = brand[1];
  ((uint32_t *)v)[1] = brand[3];
  ((uint32_t *)v)[2] = brand[2];
  v[12] = 0;

  cpuid(1, &a, &d);

  uint8_t s = get_rtc_register(0x00);
  uint8_t m = get_rtc_register(0x02);
  uint8_t h = get_rtc_register(0x04);
  uint8_t day = get_rtc_register(0x07);
  uint8_t mon = get_rtc_register(0x08);
  uint8_t yr = get_rtc_register(0x09);
  uint8_t rtc_b = get_rtc_register(0x0B);
  if (!(rtc_b & 0x04)) {
    s = (s & 0x0F) + ((s / 16) * 10);
    m = (m & 0x0F) + ((m / 16) * 10);
    h = ((h & 0x0F) + (((h & 0x70) / 16) * 10)) | (h & 0x80);
    day = (day & 0x0F) + ((day / 16) * 10);
    mon = (mon & 0x0F) + ((mon / 16) * 10);
    yr = (yr & 0x0F) + ((yr / 16) * 10);
  }

  char buf[16];
  clear_screen();
  print_color("    __________________________\n", 0x0B);
  print_color("   /   H E X A   O S   7.2   \\\n", 0x0B);
  print_color("  |  HEXAFSv2 · Pimp · Persist|\n", 0x0B);
  print_color("  |  120+ Cmds · ATA HEXAFS   |\n", 0x0B);
  print_color("  |  32-bit Protected Mode    |\n", 0x0B);
  print_color("   \\________________________/\n", 0x0B);
  print_string(" ┌──────────────────────────────┐\n");
  print_string(" │  OS:       "); print_color("HEXA OS 7.2 i386", 0x0A); print_string("         │\n");
  print_string(" │  Host:     "); print_color(hostname_str, 0x0A); 
  for (int sp = strlen(hostname_str); sp < 21; sp++) put_char(' ', 0x0F);
  print_string("│\n");
  print_string(" │  Version:  "); print_color("7.2 \"Diamond II\"", 0x0E); print_string("     │\n");
  print_string(" │  Kernel:   "); print_color(v, 0x0A); 
  for (int sp = strlen(v); sp < 23; sp++) put_char(' ', 0x0F);
  print_string("│\n");
  print_string(" │  Family:   ");
  itoa((a >> 8) & 0xF, buf, 10); print_string(buf);
  print_string("  Model: ");
  itoa((a >> 4) & 0xF, buf, 10); print_string(buf);
  print_string("          │\n");
  // Paging / MMU info
  uint32_t cr0_val;
  __asm__ volatile("mov %%cr0, %0" : "=r"(cr0_val));
  print_string(" │  Paging:  ");
  print_color((cr0_val & 0x80000000) ? "Enabled  (4KB pages)" : "Disabled", 
              (cr0_val & 0x80000000) ? 0x0A : 0x0C);
  print_string("    │\n");
  // Interrupts
  print_string(" │  IRQs:    ");
  print_color("PIC remapped  PIT@100Hz", 0x0A);
  print_string("    │\n");
  // Scheduler / tasks
  extern int num_tasks;
  print_string(" │  Tasks:   ");
  itoa(num_tasks, buf, 10); print_string(buf);
  print_string(" running  Scheduler: ");
  print_color("RR", 0x0A);
  print_string("    │\n");
  // Uptime
  print_string(" │  Uptime:  ");
  itoa(ticks, buf, 10); print_string(buf);
  print_string(" ticks");
  for (int sp = 5; sp < 16; sp++) put_char(' ', 0x0F);
  print_string("│\n");
  // Users
  print_string(" │  Users:   ");
  itoa(u_count, buf, 10); print_string(buf);
  print_string(" registered  Cur: ");
  print_string(u_table[u_cur].name);
  for (int sp = strlen(u_table[u_cur].name); sp < 10; sp++) put_char(' ', 0x0F);
  print_string("│\n");
  // Files
  print_string(" │  Forms:   ");
  itoa(form_count, buf, 10); print_string(buf);
  print_string("  Disk: ");
  print_string(disk_ok ? "online " : "offline");
  print_string("             │\n");
  // Date/time
  print_string(" │  Date:    20");
  itoa(yr, buf, 10); print_string(buf);
  print_string("-"); 
  if (mon < 10) { put_char('0', 0x0F); }
  itoa(mon, buf, 10); print_string(buf);
  print_string("-"); 
  if (day < 10) { put_char('0', 0x0F); }
  itoa(day, buf, 10); print_string(buf);
  print_string("  ");
  if (h < 10) { put_char('0', 0x0F); }
  itoa(h, buf, 10); print_string(buf);
  print_string(":"); 
  if (m < 10) { put_char('0', 0x0F); }
  itoa(m, buf, 10); print_string(buf);
  print_string(":"); 
  if (s < 10) { put_char('0', 0x0F); }
  itoa(s, buf, 10); print_string(buf);
  print_string("          │\n");
  // Memory
  print_string(" │  Memory:  Heap: ");
  itoa(1024, buf, 10); print_string(buf); print_string("KB");
  if (fb_available()) {
    print_string(" FB: ");
    itoa(fb_info.x_res, buf, 10); print_string(buf);
    print_string("x");
    itoa(fb_info.y_res, buf, 10); print_string(buf);
  }
  print_string("    │\n");
  // Commands / features
  print_string(" │  Shell:   HEXA CLI v7.2  80x25  │\n");
  print_string(" │  Cache:   ");
  print_color("BLK-CACHE  JOURNAL  PIMP-ACL", 0x0A);
  print_string("  │\n");
  print_string(" │  Kernel:  ");
  print_color("PageFault+GPF handler", 0x0A);
  print_string("      │\n");
  print_string(" │  Syscall: ");
  print_color("int 0x80  (28 syscalls)", 0x0A);
  print_string("     │\n");
  print_string(" │  User:    ");
  print_color("Ring3 TSS  Context switch", 0x0A);
  print_string("  │\n");
  print_string(" │  SMP:     ");
  print_color("Not supported (single-core only)", 0x0E);
  print_string("│\n");
  print_string(" └──────────────────────────────┘\n");
  print_string("\nPress any key to continue...");
  get_char(0);
  clear_screen();
}

// ---- User System ----
void init_users(void) {
  strcpy(u_table[0].name, "root");
  encode_pwd(u_table[0].pass_hash, "root", 0xA5);
  u_table[0].is_root = 1;
  u_count = 1;
  u_cur = 0;
}

static void load_users(void) {
  uint8_t raw[832];
  uint32_t raw_sz = sizeof(raw);
  if (!hexafs_users_load(raw, &raw_sz) || raw_sz < 4) return;
  int count = (int)(raw[0] | ((uint32_t)raw[1] << 8) | ((uint32_t)raw[2] << 16) | ((uint32_t)raw[3] << 24));
  if (count < 1 || count > 12 || raw_sz < 4 + (uint32_t)count * 68) return;
  for (int i = 0; i < count; i++) {
    for (int j = 0; j < 32; j++) u_table[i].name[j] = (char)raw[4 + i * 68 + j];
    for (int j = 0; j < 32; j++) u_table[i].pass_hash[j] = (char)raw[4 + i * 68 + 32 + j];
    u_table[i].is_root = (int)(raw[4 + i * 68 + 64] & 0xFF);
  }
  u_count = count;
  u_cur = 0;
}

static int save_data(void);
static void load_data(void);

void do_login(void) {
  char name[NAME_MAX], pass[NAME_MAX];
  while (1) {
    clear_screen();
  print_color(
    "╭──────────────────────────────╮\n"
    "│         H E X A   O S        │\n"
    "│        Version 7.2           │\n"
    "│   HEXAFSv2 · Pimp · 120+ Cmds│\n"
    "╰──────────────────────────────╯\n", 0x0B);
    print_string("login: ");
    get_line(name, NAME_MAX);

    if (strcmp(name, "new") == 0) {
      if (u_count >= MAX_USERS) { print_string("Max users.\n"); sleep_ticks(1000); continue; }
      print_string("New username: ");
      get_line(name, NAME_MAX);
      if (!name[0]) continue;
      int exists = 0;
      for (int i = 0; i < u_count; i++)
        if (strcmp(u_table[i].name, name) == 0) { exists = 1; break; }
      if (exists) { print_string("Exists.\n"); sleep_ticks(1000); continue; }
      print_string("Password: ");
      get_line(pass, NAME_MAX);
      strcpy(u_table[u_count].name, name);
      encode_pwd(u_table[u_count].pass_hash, pass, ticks & 0xFFFF);
      u_table[u_count].is_root = 0;
      u_count++;
      save_data();
      print_string("Created! Login now.\n");
      sleep_ticks(2000);
      continue;
    }

    print_string("pass: ");
    get_line(pass, NAME_MAX);

    for (int i = 0; i < u_count; i++)
      if (strcmp(u_table[i].name, name) == 0 && check_pwd(pass, u_table[i].pass_hash)) {
        u_cur = i;
        clear_screen();
        return;
      }
    print_color("Login incorrect.\n\n", 0x0C);
    sleep_ticks(2000);
  }
}

// ---- Form System ----
int find_form(const char *name) {
  for (int i = 0; i < form_count; i++)
    if (strcmp(form_table[i].name, name) == 0) return i;
  return -1;
}

void cmd_mkform(const char *name) {
  if (!name[0]) { print_string("Usage: mkform <formname>\n"); return; }
  if (form_count >= MAX_FORMS) { print_string("Form store full.\n"); return; }
  if (find_form(name) >= 0) { print_string("Form exists.\n"); return; }
  strcpy(form_table[form_count].name, name);
  form_table[form_count].content = kmalloc(1);
  if (!form_table[form_count].content) { print_string("Out of memory.\n"); return; }
  form_table[form_count].content[0] = '\0';
  form_table[form_count].size = 0;
  form_table[form_count].cap = 1;
  form_table[form_count].owner = u_cur;
  form_table[form_count].mode = PERM_DEFAULT;
  form_count++;
  print_string("Created.\n");
}

static void print_mode(uint16_t mode) {
  char buf[8];
  buf[0] = '0';
  buf[1] = 'x';
  uint8_t hi = (mode >> 8) & 0xFF;
  uint8_t lo = mode & 0xFF;
  const char *hex = "0123456789ABCDEF";
  buf[2] = hex[(hi >> 4) & 0xF];
  buf[3] = hex[hi & 0xF];
  buf[4] = hex[(lo >> 4) & 0xF];
  buf[5] = hex[lo & 0xF];
  buf[6] = 0;
  print_string(buf);
}

void cmd_list(void) {
  if (form_count == 0) { print_string("No forms.\n"); return; }
  char buf[16];
  for (int i = 0; i < form_count; i++) {
    uint8_t col = 0x0F;
    if (u_cur != 0 && form_table[i].owner != u_cur) col = 0x08;
    print_mode(form_table[i].mode);
    print_string(" ");
    print_string(u_table[form_table[i].owner].name);
    for (int s = strlen(u_table[form_table[i].owner].name); s < 8; s++) put_char(' ', col);
    itoa(form_table[i].size, buf, 10);
    print_color(buf, col);
    print_color("B ", col);
    print_string(form_table[i].name);
    print_string("\n");
  }
}

// Permission check: 0 = allowed, 1 = denied
int check_perm(int idx, int want_write) {
  if (idx < 0 || idx >= form_count) return 1;
  if (u_cur == 0) return 0; // root can do anything
  if (form_table[idx].owner == u_cur) {
    if (want_write && !(form_table[idx].mode & PERM_OWNER_W)) return 1;
    if (!want_write && !(form_table[idx].mode & PERM_OWNER_R)) return 1;
    return 0;
  }
  // For non-owners, only allow read if other-read is set
  if (!want_write && (form_table[idx].mode & PERM_OTH_R)) return 0;
  if (want_write && (form_table[idx].mode & PERM_OTH_W)) return 0;
  return 1;
}

void cmd_view(const char *name) {
  int idx = find_form(name);
  if (idx < 0) { print_string("Form not found.\n"); return; }
  if (check_perm(idx, 0)) { print_color("Permission denied.\n", 0x0C); return; }
  print_string(form_table[idx].content);
  print_string("\n");
}

void cmd_delete(const char *name) {
  int idx = find_form(name);
  if (idx < 0) { print_string("Form not found.\n"); return; }
  if (check_perm(idx, 1)) { print_color("Permission denied.\n", 0x0C); return; }
  if (form_table[idx].content) kfree(form_table[idx].content);
  for (int i = idx; i < form_count - 1; i++) form_table[i] = form_table[i + 1];
  form_count--;
  print_string("Deleted.\n");
}

void cmd_write(const char *args) {
  char fname[NAME_MAX] = {0};
  char content[2048] = {0};
  int i = 0, j = 0;
  while (args[i] && args[i] != ' ' && j < NAME_MAX - 1) fname[j++] = args[i++];
  while (args[i] == ' ') i++;
  j = 0;
  while (args[i] && j < 2047) content[j++] = args[i++];
  content[j] = '\0';
  if (!fname[0]) { print_string("Usage: write <form> <text>\n"); return; }
  int idx = find_form(fname);
  if (idx < 0) { print_string("Form not found. Use mkform first.\n"); return; }
  if (check_perm(idx, 1)) { print_color("Permission denied.\n", 0x0C); return; }
  int len = strlen(content);
  if (!form_ensure_cap(idx, len + 1)) { print_string("Out of memory.\n"); return; }
  strcpy(form_table[idx].content, content);
  form_table[idx].size = len;
  print_string("Written.\n");
}

void cmd_append(const char *args) {
  char fname[NAME_MAX] = {0};
  char content[2048] = {0};
  int i = 0, j = 0;
  while (args[i] && args[i] != ' ' && j < NAME_MAX - 1) fname[j++] = args[i++];
  while (args[i] == ' ') i++;
  j = 0;
  while (args[i] && j < 2047) content[j++] = args[i++];
  content[j] = '\0';
  if (!fname[0]) { print_string("Usage: append <form> <text>\n"); return; }
  int idx = find_form(fname);
  if (idx < 0) { print_string("Form not found.\n"); return; }
  if (check_perm(idx, 1)) { print_color("Permission denied.\n", 0x0C); return; }
  int cur = form_table[idx].size;
  int contlen = strlen(content);
  if (!form_ensure_cap(idx, cur + contlen + 1)) { print_string("Out of memory.\n"); return; }
  for (int k = 0; k < contlen; k++) form_table[idx].content[cur + k] = content[k];
  form_table[idx].content[cur + contlen] = '\0';
  form_table[idx].size = cur + contlen;
  print_string("Appended.\n");
}

void cmd_edit(const char *name) {
  int idx = find_form(name);
  if (idx < 0) { print_string("Form not found.\n"); return; }
  if (check_perm(idx, 1)) { print_color("Permission denied.\n", 0x0C); return; }
  print_string("Editing: ");
  print_string(name);
  print_string("\nType '.done' to finish.\n\n");
  if (form_table[idx].size > 0) {
    print_string("--- current ---\n");
    print_string(form_table[idx].content);
    print_string("\n---\n");
  }
  form_table[idx].content[0] = '\0';
  form_table[idx].size = 0;
  int total = 0;
  while (1) {
    char line[128];
    print_string("> ");
    get_line(line, 128);
    if (strcmp(line, ".done") == 0) break;
    int len = strlen(line);
    if (total + len + 2 >= CONTENT_MAX) { print_string("Form full.\n"); break; }
    for (int k = 0; k <= len; k++) form_table[idx].content[total++] = line[k];
    form_table[idx].content[total - 1] = '\n';
    form_table[idx].size = total;
  }
  print_string("Saved.\n");
}

// ---- User commands ----
void cmd_useradd(const char *name) {
  if (!name[0]) { print_string("Usage: useradd <username>\n"); return; }
  if (u_cur != 0) { print_color("Only root can add users.\n", 0x0C); return; }
  if (u_count >= MAX_USERS) { print_string("Max users reached.\n"); return; }
  for (int i = 0; i < u_count; i++)
    if (strcmp(u_table[i].name, name) == 0) { print_string("User exists.\n"); return; }
  strcpy(u_table[u_count].name, name);
  print_string("Enter password for ");
  print_string(name);
  print_string(": ");
  char pwbuf[NAME_MAX];
  get_line(pwbuf, NAME_MAX);
  encode_pwd(u_table[u_count].pass_hash, pwbuf, ticks & 0xFFFF);
  u_table[u_count].is_root = 0;
  u_count++;
  save_data();
  print_string("User created.\n");
}

void cmd_passwd(const char *args) {
  (void)args;
  char buf[NAME_MAX], newbuf[NAME_MAX];
  print_string("Current password: ");
  get_line(buf, NAME_MAX);
  if (!check_pwd(buf, u_table[u_cur].pass_hash)) { print_color("Wrong password.\n", 0x0C); return; }
  print_string("New password: ");
  get_line(newbuf, NAME_MAX);
  encode_pwd(u_table[u_cur].pass_hash, newbuf, ticks & 0xFFFF);
  save_data();
  print_string("Password changed.\n");
}

void cmd_login(const char *args) {
  if (!args[0]) { do_login(); return; }
  for (int i = 0; i < u_count; i++)
    if (strcmp(u_table[i].name, args) == 0) {
      print_string("Password: ");
      char buf[NAME_MAX];
      get_line(buf, NAME_MAX);
      if (check_pwd(buf, u_table[i].pass_hash)) {
        u_cur = i; print_string("Ok.\n");
        // Migrate old-format hash to new format
        if (u_table[i].pass_hash[2] == ' ') {
          encode_pwd(u_table[i].pass_hash, buf, ticks & 0xFFFF);
          save_data();
        }
        return;
      }
      print_color("Wrong password.\n", 0x0C); return;
    }
  print_string("User not found.\n");
}

void cmd_logout(void) {
  do_login();
}

// ---- setmode / setowner ----
void cmd_setmode(const char *args) {
  char mode_str[16]={0}, fname[32]={0};
  int i=0,j=0;
  while(args[i]&&args[i]!=' '&&j<15){mode_str[j++]=args[i++];}
  while(args[i]==' '){i++;} j=0;
  while(args[i]&&j<31){fname[j++]=args[i++];}
  if(!mode_str[0]||!fname[0]){print_string("Usage: setmode <hex_mode> <form>\n");return;}
  int idx=find_form(fname);
  if(idx<0){print_string("Not found.\n");return;}
  if(u_cur!=0&&form_table[idx].owner!=u_cur){print_color("Permission denied.\n",0x0C);return;}
  uint16_t newmode = (uint16_t)atoi(mode_str);
  form_table[idx].mode = newmode;
  print_string("Mode changed.\n");
}

void cmd_setowner(const char *args) {
  char owner_str[16]={0}, fname[32]={0};
  int i=0,j=0;
  while(args[i]&&args[i]!=' '&&j<15){owner_str[j++]=args[i++];}
  while(args[i]==' '){i++;} j=0;
  while(args[i]&&j<31){fname[j++]=args[i++];}
  if(!owner_str[0]||!fname[0]){print_string("Usage: setowner <user> <form>\n");return;}
  if(u_cur!=0){print_color("Only root.\n",0x0C);return;}
  int nu=-1;
  for(int k=0;k<u_count;k++)if(strcmp(u_table[k].name,owner_str)==0){nu=k;break;}
  if(nu<0){print_string("User not found.\n");return;}
  int idx=find_form(fname);
  if(idx<0){print_string("Not found.\n");return;}
  form_table[idx].owner=nu;
  print_string("Owner changed.\n");
}

// ---- Simple Features ----
void cmd_banner(const char *text) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  if (!text[0]) { print_string("Usage: banner <text>\n"); return; }
  int len = strlen(text);
  if (len > 60) { print_string("Text too long.\n"); return; }
  print_string("\n+");
  for (int i = 0; i < len + 2; i++) put_char('-', 0x0F);
  print_string("+\n| ");
  print_string(text);
  print_string(" |\n+");
  for (int i = 0; i < len + 2; i++) put_char('-', 0x0F);
  print_string("+\n\n");
}

void cmd_yes(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  print_string("Press any key to stop...\n");
  while (!kb_hit()) {
    print_string("y ");
    do_tick();
  }
  get_char(0);
  print_string("\n");
}

void cmd_time(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  while (get_update_in_progress_flag());
  uint8_t s = get_rtc_register(0x00);
  uint8_t m = get_rtc_register(0x02);
  uint8_t h = get_rtc_register(0x04);
  uint8_t rtc_b = get_rtc_register(0x0B);
  if (!(rtc_b & 0x04)) {
    s = (s & 0x0F) + ((s / 16) * 10);
    m = (m & 0x0F) + ((m / 16) * 10);
    h = ((h & 0x0F) + (((h & 0x70) / 16) * 10)) | (h & 0x80);
  }
  char buf[16];
  itoa(h, buf, 10); print_string(buf);
  print_string(":");
  itoa(m, buf, 10); print_string(buf);
  print_string(":");
  itoa(s, buf, 10); print_string(buf);
  print_string("\n");
}

void cmd_fortune(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  const char *quotes[] = {
    "Hello, World!",
    "The only constant is change.",
    "Keep it simple, stupid.",
    "sudo make me a sandwich.",
    "42",
    "Not all who wander are lost.",
    "To be or not to be.",
    "Have you tried turning it off and on?",
    "There is no place like 127.0.0.1",
    "Segmentation fault (core dumped)",
    "It's not a bug, it's a feature.",
    "All your base are belong to us.",
    "0xDEADBEEF",
    "I think, therefore I am.",
    "Just reboot it.",
    "Hello? Is it me you're looking for?",
  };
  print_string(quotes[rand() % 16]);
  print_string("\n");
}

void cmd_shutdown(void) {
  print_color("\nShutting down...\n", 0x0C);
  sleep_ticks(5000);
  __asm__ volatile("cli; hlt");
}

// ---- New Serious Commands ----




void cmd_hist(void) {
  char buf[16];
  int start = (hist_count < HIST_MAX) ? 0 : hist_idx;
  int count = (hist_count < HIST_MAX) ? hist_count : HIST_MAX;
  for (int i = 0; i < count; i++) {
    int idx = (start + i) % HIST_MAX;
    itoa(i + 1, buf, 10); print_string(buf);
    print_string("  "); print_string(hist_buf[idx]); print_string("\n");
  }
}





// ---- Shell Utilities ----
static int alias_count = 0;
static struct { char name[16]; char cmd[64]; } alias_table[16];










void cmd_dimpath(void) {
  print_string("/home/");
  print_string(u_table[u_cur].name);
  print_string("\n");
}

void cmd_makedim(const char *name) {
  if(!name[0]){print_string("Usage: makedim <dimname>\n");return;}
  // store as dim with trailing /
  char dname[NAME_MAX];
  strcpy(dname,name);
  int len=strlen(dname);
  if(dname[len-1]!='/'){dname[len]='/';dname[len+1]=0;}
  if(find_form(dname)>=0){print_string("Exists.\n");return;}
  if(form_count>=MAX_FORMS){print_string("Full.\n");return;}
  strcpy(form_table[form_count].name,dname);
  form_table[form_count].content[0]=0;
  form_table[form_count].size=0;
  form_table[form_count].owner=u_cur;
  form_table[form_count].mode=PERM_DEFAULT|PERM_OWNER_X|PERM_GRP_X|PERM_OTH_X;
  form_count++;
  print_string("Created.\n");
}








void cmd_dmesg(void) {
  log_write(LOG_LEVEL_INFO, "dmesg requested");
  print_string("Kernel log (last entries):\n");
  print_string(log_get());
  print_string("\n");
  char buf[16];
  itoa(log_get_count(), buf, 10); print_string(buf);
  print_string(" total log entries\n");
}



// ---- New Fun Commands ----
void cmd_cowsay(const char *text) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  if (!text[0]) { print_string("Usage: cowsay <text>\n"); return; }
  int len = strlen(text); if (len > 50) len = 50;
  print_string(" ");
  for (int i = 0; i < len + 2; i++) put_char('_', 0x0F);
  print_string("\n< ");
  for (int i = 0; i < len; i++) put_char(text[i], 0x0F);
  print_string(" >\n ");
  for (int i = 0; i < len + 2; i++) put_char('-', 0x0F);
  print_string("\n");
  print_string("        \\   ^__^\n");
  print_string("         \\  (oo)\\_______\n");
  print_string("            (__)\\       )\\/\\\n");
  print_string("                ||----w |\n");
  print_string("                ||     ||\n");
}

void cmd_cmatrix(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
#define CM_COLS 50
#define CM_ROWS 16
  int drop[CM_COLS], delay[CM_COLS];
  for (int i = 0; i < CM_COLS; i++) {
    drop[i] = -(rand() % 20);
    delay[i] = rand() % 4;
  }
  clear_screen();
  print_string("Press any key to exit...\n");
  while (1) {
    if ((inb(0x64) & 1) || (inb(0x3F8 + 5) & 1)) { get_char(0); break; }
    // Clear old trail
    for (int x = 0; x < CM_COLS; x++) {
      int y = drop[x];
      if (y >= 0 && y < CM_ROWS)
        VGA_BUFFER[(4 + y) * 80 + (15 + x)] = (0x00 << 8) | ' ';
      if (y - 1 >= 0 && y - 1 < CM_ROWS)
        VGA_BUFFER[(4 + y - 1) * 80 + (15 + x)] = (0x00 << 8) | ' ';
    }
    // Advance drops
    for (int x = 0; x < CM_COLS; x++) {
      if (--delay[x] > 0) continue;
      delay[x] = (rand() % 3) + 1;
      drop[x]++;
      if (drop[x] >= CM_ROWS + 3) drop[x] = -(rand() % 15);
    }
    // Draw new frame
    for (int x = 0; x < CM_COLS; x++) {
      int y = drop[x];
      if (y >= 0 && y < CM_ROWS)
        VGA_BUFFER[(4 + y) * 80 + (15 + x)] = (0x0A << 8) | (char)((rand() % 94) + 33);
      if (y - 1 >= 0 && y - 1 < CM_ROWS)
        VGA_BUFFER[(4 + y - 1) * 80 + (15 + x)] = (0x02 << 8) | (char)((rand() % 94) + 33);
    }
    for (volatile int w = 0; w < 2000; w++);
  }
  clear_screen();
}

void cmd_dice(const char *args) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  int num = 1, sides = 6;
  if (args[0]) {
    char n[8]={0}, s[8]={0}; int i=0,j=0;
    while(args[i]&&args[i]!='d'&&args[i]!='D'&&j<7){n[j++]=args[i++];}
    if(n[0]) num = atoi(n);
    if(args[i]=='d'||args[i]=='D'){i++;j=0;while(args[i]&&j<7){s[j++]=args[i++];}}
    if(s[0]) sides = atoi(s);
  }
  if (num < 1 || num > 10) num = 1;
  if (sides < 2) sides = 6;
  char buf[16];
  for (int i = 0; i < num; i++) {
    int r = (rand() % sides) + 1;
    itoa(r, buf, 10); print_string(buf);
    if (i < num - 1) print_string(" ");
  }
  print_string("\n");
}

void cmd_8ball(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  const char *r[] = {
    "Yes.","No.","Maybe.","Ask again later.",
    "Definitely.","I doubt it.","Without a doubt.",
    "Cannot predict now.","Outlook good.",
    "Very doubtful.","Signs point to yes.",
    "My reply is no.","It is certain.",
    "Don't count on it.","Most likely.","Better not."
  };
  print_string(r[rand() % 16]); print_string("\n");
}

void cmd_logo(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  print_color("  ╔══════════════════════════════════╗\n", 0x0B);
  print_color("  ║  H   H  EEEEE  X   X   AAAAA    ║\n", 0x0B);
  print_color("  ║  H   H  E      X   X   A   A    ║\n", 0x0B);
  print_color("  ║  HHHHH  EEEE   X   X   AAAAA    ║\n", 0x0B);
  print_color("  ║  H   H  E      X   X   A   A    ║\n", 0x0A);
  print_color("  ║  H   H  EEEEE  X   X   A   A    ║\n", 0x0A);
  print_color("  ║         7.2  DIAMOND  II         ║\n", 0x0E);
  print_color("  ║                                  ║\n", 0x0A);
  print_color("  ║  HEXAFSv2 · PIMP · PERSIST     ║\n", 0x0A);
  print_color("  ║   BLK CACHE · JOURNALING       ║\n", 0x0A);
  print_color("  ╚══════════════════════════════════╝\n", 0x0B);
}

void cmd_sl(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  int frame = 0;
  print_string("Press any key to stop...\n");
  while (1) {
    if (kb_hit()) { get_char(0); break; }
    clear_screen();
    if (frame % 2) {
      print_string("      ====        ________                ________\n");
      print_string("  _D _|  |_______/        \\__I_I_____===__|_________|\n");
      print_string("   |(_)---|   |\\   o      |   |        |     | |  |\n");
      print_string("   /     |   | \\  o       |   |        |     | |  |\n");
      print_string("  |      |   |  \\         |   |        |     | |  |\n");
      print_string("  \\ ===== |___|===________/_  |_______|_____| |__|\n");
    } else {
      print_string("      ====        ________                ________\n");
      print_string("  _D _|  |_______/        \\__I_I_____===__|_________|\n");
      print_string("   |(_)---|   |\\   o      |   |        |    | |   |\n");
      print_string("   /     |   | \\  o       |   |        |    | |   |\n");
      print_string("  |      |   |  \\         |   |        |    | |   |\n");
      print_string("  \\ ===== |___|===________/_  |_______|____| |___|\n");
    }
    frame++;
    sleep_ticks(2000);
  }
  clear_screen();
}

void cmd_morse(const char *text) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  if (!text[0]) { print_string("Usage: morse <text>\n"); return; }
  const char *morse[] = {
    ".-","-...","-.-.","-..",".","..-.","--.","....","..",".---",
    "-.-",".-..","--","-.","---",".--.","--.-",".-.","...","-",
    "..-","...-",".--","-..-","-.--","--.."
  };
  for (int i = 0; text[i]; i++) {
    char c = text[i];
    if (c >= 'a' && c <= 'z') { print_string(morse[c - 'a']); print_string(" "); }
    else if (c >= 'A' && c <= 'Z') { print_string(morse[c - 'A']); print_string(" "); }
    else if (c == ' ') { print_string("  "); }
    else { put_char(c, 0x0F); print_string(" "); }
  }
  print_string("\n");
}

void cmd_russian(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  print_string("Russian Roulette... *spin* *click* ... ");
  sleep_ticks(3000);
  if ((rand() % 6) == 0) {
    print_color("BANG! You're dead. Halting.\n", 0x0C);
    __asm__ volatile("cli; hlt");
  } else {
    print_color("*click* Lucky!\n", 0x0A);
  }
}

// ---- More Fun ----
void cmd_insult(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  const char *r[] = {
    "Your code is so bad, rm -rf / is an improvement.",
    "You make segfaults look like features.",
    "Slower than a snail on tranquilizers.",
    "I've seen better code from a RNG.",
    "Your debugging skills rival a brick.",
    "Even 0xDEADBEEF is more alive than your code."
  };
  print_string(r[rand() % 6]); print_string("\n");
}

void cmd_excuse(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  const char *r[] = {
    "The bit bucket overflowed.",
    "Someone reversed the neutron flow.",
    "It works on my machine.",
    "Must be a DNS issue.",
    "The hamsters running the server are on strike.",
    "It's a layer 8 problem."
  };
  print_string(r[rand() % 6]); print_string("\n");
}

void cmd_compliment(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  const char *r[] = {
    "Your kernel is sexier than a naked CPU.",
    "Your code is so clean it sparkles.",
    "You're the Linus Torvalds of your kitchen.",
    "Your bit-shifting skills are legendary.",
    "Even your bugs are elegant.",
    "You make assembly look like poetry."
  };
  print_string(r[rand() % 6]); print_string("\n");
}

void cmd_hack(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  print_color("HACK SEQUENCE INITIATED\n", 0x0A);
  for (int i = 0; i < 15; i++) {
    print_string("["); for (int j = 0; j < 15; j++) put_char(j < i ? '#' : ' ', 0x0A);
    print_string("] "); char b[8]; itoa(i * 7, b, 10); print_string(b); print_string("%\n");
    if (kb_hit()) { get_char(0); break; }
  }
  print_color("ACCESS GRANTED. SYSTEM OWNED.\n", 0x0A);
  print_string("Just kidding. You're not a hacker.\n");
  print_string("Press any key..."); get_char(0);
}

void cmd_bsod(void) {
  if (find_form(".fun") < 0) { print_string("Package required: ayo add fun\n"); print_string("(use 'diese' if not root)\n"); return; }
  clear_screen();
  current_color = 0x1F;
  for (int i = 0; i < 80 * 25; i++) VGA_BUFFER[i] = (0x1F << 8) | ' ';
  cursor_x = 0; cursor_y = 0;
  print_string("A fatal exception 0E has occurred at 0028:C0001E6F.\n");
  print_string("The current application will be terminated.\n\n");
  print_string("*  Press any key to restart.\n\n");
  print_string("System halted.  Please reboot.\n");
  get_char(0);
  current_color = 0x0F;
  clear_screen();
}

// ---- Diamond Feature Commands (v7.2) ----
void cmd_clock(void) {
  clear_screen();
  cursor_x = 0; cursor_y = 0;
  print_string("HEXA OS Live Clock - Press Q to exit\n");
  print_string("====================================\n");
  while (1) {
    if (kb_hit()) { char c = getch_nb(); if (c == 'q' || c == 'Q') break; }
    while (get_update_in_progress_flag());
    uint8_t s = get_rtc_register(0x00);
    uint8_t m = get_rtc_register(0x02);
    uint8_t h = get_rtc_register(0x04);
    uint8_t d = get_rtc_register(0x07);
    uint8_t mo = get_rtc_register(0x08);
    uint8_t y = get_rtc_register(0x09);
    uint8_t rtc_b = get_rtc_register(0x0B);
    if (!(rtc_b & 0x04)) {
      s = (s & 0x0F) + ((s / 16) * 10);
      m = (m & 0x0F) + ((m / 16) * 10);
      h = ((h & 0x0F) + (((h & 0x70) / 16) * 10)) | (h & 0x80);
      d = (d & 0x0F) + ((d / 16) * 10);
      mo = (mo & 0x0F) + ((mo / 16) * 10);
      y = (y & 0x0F) + ((y / 16) * 10);
    }
    // Clear display area (lines 3-5) on VGA
    for (int i = 0; i < 3 * 80; i++)
      VGA_BUFFER[(3 * 80) + i] = (0x0F << 8) | ' ';
    // Write formatted time to VGA line 3
    cursor_x = 0; cursor_y = 3;
    char buf[16];
    print_string("Current Time: ");
    print_string("20"); itoa(y, buf, 10); print_string(buf);
    print_string("-"); if (mo < 10) put_char('0', 0x0F);
    itoa(mo, buf, 10); print_string(buf);
    print_string("-"); if (d < 10) put_char('0', 0x0F);
    itoa(d, buf, 10); print_string(buf);
    print_string("  ");
    if (h < 10) put_char('0', 0x0F);
    itoa(h, buf, 10); print_string(buf);
    print_string(":"); if (m < 10) put_char('0', 0x0F);
    itoa(m, buf, 10); print_string(buf);
    print_string(":"); if (s < 10) put_char('0', 0x0F);
    itoa(s, buf, 10); print_string(buf);
    // Uptime on line 4
    cursor_x = 0; cursor_y = 4;
    print_string("Uptime: "); itoa(system_ticks / 100, buf, 10); print_string(buf); print_string(" seconds");
    // Serial-friendly output with carriage return
    cursor_x = 0; cursor_y = 5;
    print_string("Press Q to quit");
    for (volatile int d = 0; d < 50000; d++);
  }
  clear_screen();
}



void cmd_hexdump(const char *name) {
  if (!name[0]) { print_string("Usage: hexdump <form>\n"); return; }
  int idx = find_form(name);
  if (idx < 0) { print_string("Not found.\n"); return; }
  if (check_perm(idx, 0)) { print_color("Denied.\n", 0x0C); return; }
  char buf[16];
  int size = form_table[idx].size;
  for (int pos = 0; pos < size; pos += 16) {
    // Address
    itoa(pos, buf, 16);
    int blen = strlen(buf);
    for (int p = 4 - blen; p > 0; p--) put_char('0', 0x0F);
    print_string(buf);
    print_string(": ");
    // Hex bytes
    for (int i = 0; i < 16; i++) {
      if (pos + i < size) {
        itoa((unsigned char)form_table[idx].content[pos + i], buf, 16);
        if (strlen(buf) < 2) print_string("0");
        print_string(buf);
      } else {
        print_string("  ");
      }
      if (i == 7) put_char(' ', 0x0F);
      put_char(' ', 0x0F);
    }
    print_string(" |");
    for (int i = 0; i < 16; i++) {
      if (pos + i < size) {
        unsigned char c = form_table[idx].content[pos + i];
        put_char(c >= 32 && c < 127 ? c : '.', 0x0F);
      }
    }
    print_string("|\n");
  }
  itoa(size, buf, 10); print_string(buf); print_string(" bytes\n");
}




void cmd_uptime_fmt(void) {
  char buf[16];
  uint32_t total_sec = system_ticks / 100;
  uint32_t days = total_sec / 86400;
  uint32_t hours = (total_sec % 86400) / 3600;
  uint32_t mins = (total_sec % 3600) / 60;
  uint32_t secs = total_sec % 60;
  print_string("Uptime: ");
  itoa(days, buf, 10); print_string(buf); print_string("d ");
  itoa(hours, buf, 10); print_string(buf); print_string("h ");
  itoa(mins, buf, 10); print_string(buf); print_string("m ");
  itoa(secs, buf, 10); print_string(buf); print_string("s");
  print_string("  ("); itoa(system_ticks, buf, 10); print_string(buf); print_string(" ticks)\n");
}

void cmd_sysinfo(void) {
  char buf[16];
  print_color("HEXA OS v7.2 - Quick System Info\n", 0x0B);
  print_string("================================\n");
  cmd_cpuinfo();
  print_string("Memory: "); itoa(pmm_count_free() * 4, buf, 10); print_string(buf); print_string(" KB free\n");
  print_string("Tasks:  "); itoa(num_tasks, buf, 10); print_string(buf); print_string(" running\n");
  print_string("Forms:  "); itoa(form_count, buf, 10); print_string(buf); print_string("\n");
  print_string("Users:  "); itoa(u_count, buf, 10); print_string(buf);
  print_string("  Current: "); print_string(u_table[u_cur].name); print_string("\n");
  print_string("Disk:   "); print_string(disk_ok ? "online" : "offline"); print_string("\n");
  print_string("Host:   "); print_string(hostname_str); print_string("\n");
  cmd_uptime_fmt();
  uint32_t a, d;
  cpuid(1, &a, &d);
  print_string("Features: ");
  print_string(d & 0x100 ? "FPU " : "");
  print_string(d & 0x2000 ? "MCE " : "");
  print_string(d & 0x400000 ? "ACPI " : "");
  print_string(d & 0x4000000 ? "HT " : "");
  print_string(d & 0x20000000 ? "SSE " : "");
  print_string("\n");
}

// ---- Package Manager (ayo) ----
#define PKG_FORMS_MAX 6
struct pkg_entry {
  char name[32];
  struct { char name[32]; char content[128]; } forms[PKG_FORMS_MAX];
  int nforms;
};
static const struct pkg_entry pkg_db[] = {
  {"games", {
    {"snake.txt","Snake: eat food (@), avoid walls, grow! WASD to move."},
    {"ttt.txt","Tic-Tac-Toe: two-player. Get three in a row!"},
    {"hangman.txt","Hangman: guess the hidden word letter by letter."},
    {"memory.txt","Memory: watch color sequence and repeat it!"},
  }, 4},
  {"docs", {
    {"readme.txt","HEXA OS 5.1 - Paging, interrupts, scheduler, syscalls, user mode!"},
    {"commands.txt","Type 'help' to list all available commands (70+)."},
    {"about.txt","HEXA OS 5.1: Paging Edition - IDT, PIC, PIT, paging, kmalloc, scheduler, syscalls!"},
  }, 3},
  {"fun", {
    {"quotes.txt","\"The only constant is change.\" - Heraclitus"},
    {"jokes.txt","Q: Why did the OS crash? A: Out of funny business!"},
    {"easter.txt","Try 'russian' for a risky game of chance!"},
  }, 3},
  {"dev", {
    {"api.txt","API: print_string, put_char, get_char, get_line, clear_screen"},
    {"memory.txt","Kernel: 0x10000 | VGA: 0xB8000 | Stack: 0x7C00"},
    {"pci.txt","PCI: config via ports 0xCF8/0xCFC, BDF addressing."},
  }, 3},
  {"cheatsheet", {
    {"keys.txt","PS/2 scancodes mapped to ASCII. Shift for uppercase."},
    {"ports.txt","0x3F8=COM1, 0x60=Keyboard, 0x64=Kbd Status, 0xCF8/CFC=PCI"},
    {"cmos.txt","CMOS regs: 0x00=sec, 0x02=min, 0x04=hr, 0x07=day, 0x08=mon, 0x09=yr"},
    {"ata.txt","ATA PIO: I/O 0x1F0-0x1F7. LBA28 addressing. Primary channel."},
  }, 4},
  {"tools", {
    {"hexdump.txt","View form content in hex with 'view <form>' for text."},
    {"calc.txt","Built-in calc: calc 2 + 2, calc 10 / 3"},
    {"editor.txt","Edit forms with 'edit <form>', finish with '.done'"},
    {"tips.txt","Use 'diese <cmd>' to run as root. 'up-arrow' recalls last cmd."},
  }, 4},
  {"net", {
    {"ping.txt","ping: simulated network ping. 'ping localhost'"},
    {"dns.txt","DNS resolves hostnames to IPs (simulated)."},
    {"ip.txt","IP addressing: 127.0.0.1 is localhost."},
    {"tcp.txt","TCP: reliable transport. HEXA uses raw I/O."},
    {"curl.txt","curl: simulated web requests (display only)."},
  }, 5},
  {"math", {
    {"pi.txt","PI = 3.1415926535897932384626433832795"},
    {"e.txt","E = 2.7182818284590452353602874713527"},
    {"fib.txt","Fibonacci: 0,1,1,2,3,5,8,13,21,34,55,89,144..."},
    {"prime.txt","Primes: 2,3,5,7,11,13,17,19,23,29,31,37,41,43,47..."},
    {"stats.txt","mean, median, mode - basic stats. Use calc for math."},
  }, 5},
  {"sysadmin", {
    {"df.txt","df: disk free. storage.img: 128KB total"},
    {"dmesg.txt","dmesg: kernel ring buffer. Boot messages here."},
    {"uptime.txt","uptime: system uptime in ticks"},
    {"iostat.txt","iostat: ATA disk I/O stats (simulated)."},
    {"top.txt","tasks: system tasks. Only shell task running."},
  }, 5},
  {"security", {
    {"passwd.txt","passwd: change your password. New: hashed storage!"},
    {"perm.txt","Permissions: hex mode. Use setmode + setowner."},
    {"root.txt","Root has full access. Use 'diese <cmd>' for root."},
    {"audit.txt","Log: all changes are persisted to disk image."},
  }, 4},
  {"media", {
    {"ascii.txt","ASCII art: use 'logo' for HEXA banner."},
    {"colors.txt","VGA colors: 0-15. Use 'color <n>' to change."},
    {"pics.txt","ASCII pics: try 'cowsay' for cow, 'sl' for train."},
    {"anim.txt","Animation: 'cmatrix' rain, 'sl' train, 'hack' bar."},
  }, 4},
  {"productivity", {
    {"calendar.txt","Calendar: see date via 'date' command (RTC)."},
    {"reminder.txt","No alarms yet. Use forms for reminders."},
    {"notes.txt","Note taking: 'edit <form>' creates/modifies notes."},
    {"todo.txt","Todo: 'mkform todo.txt' and 'edit todo.txt'."},
  }, 4},
  {"science", {
    {"elements.txt","H,He,Li,Be,B,C,N,O,F,Ne - periodic table begins here"},
    {"speed.txt","Speed of light: 299,792,458 m/s"},
    {"gravity.txt","Gravitational constant G = 6.67430e-11"},
    {"planck.txt","Planck constant h = 6.62607015e-34"},
    {"solar.txt","Solar system: 8 planets orbiting Sol."},
  }, 5},
  {"lang", {
    {"hello_c.txt","C: #include <stdio.h>\\nint main(){return 0;}"},
    {"hello_py.txt","Python: print('Hello, HEXA!')"},
    {"hello_asm.txt","x86 ASM: mov eax, 1; int 0x80"},
    {"hello_js.txt","JS: console.log('Hello from HEXA!');"},
    {"shell.txt","Shell: echo 'Hello HEXA!'"},
  }, 5},
  {"puzzle", {
    {"sudoku.txt","Sudoku: 9x9 grid. Solve with logic."},
    {"crossword.txt","Crossword: word puzzle. Not implemented."},
    {"maze.txt","Maze: find path from start to exit."},
    {"riddle.txt","Riddle: What has keys but no locks? A keyboard!"},
  }, 4},
  {"adventure", {
    {"start.txt","You stand in a dark room. Commands: look, go, take"},
    {"cave.txt","The cave is damp. You see a glowing sword."},
    {"forest.txt","A dense forest. Paths lead north, east, south."},
    {"dragon.txt","A dragon sleeps on a pile of gold."},
    {"treasure.txt","You found the treasure! 1000 gold coins."},
  }, 5},
  {"retro", {
    {"basic.txt","BASIC: 10 PRINT 'HELLO'\\n20 GOTO 10"},
    {"fortran.txt","FORTRAN: PROGRAM HELLO\\nPRINT*,'HELLO'\\nEND"},
    {"pascal.txt","Pascal: begin writeln('Hello'); end."},
    {"c64.txt","C64: 6510 CPU, 64KB RAM, SID sound."},
    {"dos.txt","MS-DOS: dir, copy, del, edit, type."},
  }, 5},
  {"algorithms", {
    {"sorting.txt","Sorting: bubble, quick, merge, heap - O(n log n) best"},
    {"search.txt","Search: binary O(log n), linear O(n)."},
    {"graph.txt","Graph: BFS, DFS, Dijkstra, A* pathfinding."},
    {"tree.txt","Tree: binary search, AVL, red-black, trie."},
    {"dp.txt","Dynamic Prog: Fibonacci, knapSack, LCS, LIS."},
  }, 5},
  {"crypto", {
    {"caesar.txt","Caesar cipher: shift letters by N positions."},
    {"vigenere.txt","Vigenere: polyalphabetic cipher with keyword."},
    {"xor.txt","XOR: simple symmetric encryption."},
    {"hash.txt","HEXA now uses hashed passwords! djb2 variant."},
    {"rot13.txt","ROT13: rotate by 13, reverses itself."},
  }, 5},
  {"music", {
    {"notes.txt","Notes: C D E F G A B (do re mi fa so la ti)"},
    {"chords.txt","Chords: Cmaj, Dmin, Emin, Fmaj, Gmaj, Amin, Bdim"},
    {"scale.txt","C major scale: C D E F G A B C"},
    {"beep.txt","Use 'beep' command for terminal bell."},
    {"song.txt","Twinkle Twinkle Little Star: C C G G A A G"},
  }, 5},
  {"philosophy", {
    {"plato.txt","Plato: The Republic - justice in the individual and state."},
    {"nietzsche.txt","Nietzsche: 'What does not kill me makes me stronger.'"},
    {"descartes.txt","Descartes: 'I think, therefore I am.'"},
    {"confucius.txt","Confucius: 'It does not matter how slowly you go as long as you do not stop.'"},
    {"zen.txt","Zen: 'Before enlightenment, chop wood, carry water.'"},
  }, 5},
  {"geography", {
    {"capitals.txt","Paris, London, Berlin, Rome, Madrid, Lisbon, Athens"},
    {"rivers.txt","Nile (6650km), Amazon (6400km), Mississippi (3730km)"},
    {"mountains.txt","Everest (8848m), K2 (8611m), Kangchenjunga (8586m)"},
    {"oceans.txt","Pacific, Atlantic, Indian, Southern, Arctic"},
    {"flags.txt","HEXA OS flags: ASCII art banners!"},
  }, 5},
};
#define PKG_COUNT ((int)(sizeof(pkg_db) / sizeof(pkg_db[0])))

static void cmd_ayo(const char *args) {
  char sub[16]={0}, pkg[32]={0};
  int i=0,j=0;
  while(args[i]&&args[i]!=' '&&j<15){sub[j++]=args[i++];}
  while(args[i]==' '){i++;} j=0;
  while(args[i]&&j<31){pkg[j++]=args[i++];}

  if (strcmp(sub, "list") == 0) {
    print_string("Packages:\n");
    for (int i = 0; i < PKG_COUNT; i++) {
      char mkr[32]; mkr[0]='.'; mkr[1]=0; strcat(mkr, pkg_db[i].name);
      int inst = find_form(mkr) >= 0;
      print_string("  "); print_string(pkg_db[i].name);
      if (inst) print_color(" [I]", 0x0A); else print_string(" [ ]");
      print_string("  ");
      char b[8]; itoa(pkg_db[i].nforms,b,10); print_string(b);   print_string(" forms\n");
    }
    return;
  }

  if (strcmp(sub, "help") == 0) {
    print_string("ayo - HEXA OS Package Manager\n");
    print_string("Usage:\n");
    print_string("  ayo list           show packages [I]=installed\n");
    print_string("  ayo add <pkg>     install package (root only)\n");
    print_string("  ayo remove <pkg>  uninstall package (root only)\n");
    print_string("  ayo update        refresh all installed (root only)\n");
    print_string("Packages: ");
    for (int i = 0; i < PKG_COUNT; i++) {
      print_string(pkg_db[i].name);
      if (i < PKG_COUNT - 1) print_string(", ");
    }
    print_string("\n");
    return;
  }

  if (strcmp(sub, "add") == 0) {
    if (!pkg[0]) { print_string("Usage: ayo add <pkg>\n"); return; }
    if (u_cur != 0) { print_color("Root only.\n", 0x0C); print_string("Use: diese ayo add <pkg>\n"); return; }
    int idx = -1;
    for (int i = 0; i < PKG_COUNT; i++)
      if (strcmp(pkg_db[i].name, pkg) == 0) { idx = i; break; }
    if (idx < 0) { print_string("Unknown package.\n"); return; }
    if (form_count + pkg_db[idx].nforms + 1 >= MAX_FORMS) { print_string("FS full.\n"); return; }
    char mkr[32]; mkr[0]='.'; mkr[1]=0; strcat(mkr, pkg);
    if (find_form(mkr) >= 0) { print_string("Already installed.\n"); return; }
    int mi = form_count;
    strcpy(form_table[mi].name, mkr);
    form_ensure_cap(mi, 2); form_table[mi].content[0]='1';
    form_table[mi].content[1]=0; form_table[mi].size=1; form_table[mi].owner=u_cur; form_count++;
    for (int f = 0; f < pkg_db[idx].nforms; f++) {
      if (find_form(pkg_db[idx].forms[f].name) < 0) {
        strcpy(form_table[form_count].name, pkg_db[idx].forms[f].name);
        form_ensure_cap(form_count, strlen(pkg_db[idx].forms[f].content) + 1);
        strcpy(form_table[form_count].content, pkg_db[idx].forms[f].content);
        form_table[form_count].size = strlen(pkg_db[idx].forms[f].content);
        form_table[form_count].owner = u_cur;
        form_count++;
      }
    }
    save_data();
    print_string("Installed: "); print_string(pkg); print_string("\n");
    return;
  }

  if (strcmp(sub, "remove") == 0) {
    if (!pkg[0]) { print_string("Usage: ayo remove <pkg>\n"); return; }
    if (u_cur != 0) { print_color("Root only.\n", 0x0C); print_string("Use: diese ayo remove <pkg>\n"); return; }
    int idx = -1;
    for (int i = 0; i < PKG_COUNT; i++)
      if (strcmp(pkg_db[i].name, pkg) == 0) { idx = i; break; }
    if (idx < 0) { print_string("Unknown package.\n"); return; }
    char mkr[32]; mkr[0]='.'; mkr[1]=0; strcat(mkr, pkg);
    if (find_form(mkr) < 0) { print_string("Not installed.\n"); return; }
    for (int f = 0; f < pkg_db[idx].nforms; f++) {
      int fi = find_form(pkg_db[idx].forms[f].name);
      if (fi >= 0) { for (int k = fi; k < form_count - 1; k++) form_table[k]=form_table[k+1]; form_count--; }
    }
    int mi = find_form(mkr);
    if (mi >= 0) { for (int k = mi; k < form_count - 1; k++) form_table[k]=form_table[k+1]; form_count--; }
    save_data();
    print_string("Removed: "); print_string(pkg); print_string("\n");
    return;
  }

  if (strcmp(sub, "update") == 0) {
    if (u_cur != 0) { print_color("Root only.\n", 0x0C); print_string("Use: diese ayo update\n"); return; }
    int n = 0;
    for (int i = 0; i < PKG_COUNT; i++) {
      char mkr[32]; mkr[0]='.'; mkr[1]=0; strcat(mkr, pkg_db[i].name);
      if (find_form(mkr) < 0) continue;
      for (int f = 0; f < pkg_db[i].nforms; f++) {
        int fi = find_form(pkg_db[i].forms[f].name);
        if (fi >= 0) {
          form_ensure_cap(fi, strlen(pkg_db[i].forms[f].content) + 1);
          strcpy(form_table[fi].content, pkg_db[i].forms[f].content);
          form_table[fi].size = strlen(pkg_db[i].forms[f].content);
        } else if (form_count < MAX_FORMS) {
          strcpy(form_table[form_count].name, pkg_db[i].forms[f].name);
          form_ensure_cap(form_count, strlen(pkg_db[i].forms[f].content) + 1);
          strcpy(form_table[form_count].content, pkg_db[i].forms[f].content);
          form_table[form_count].size = strlen(pkg_db[i].forms[f].content);
          form_table[form_count].owner = u_cur;
          form_count++;
        }
      }
      n++;
    }
    save_data();
    char b[8]; itoa(n,b,10); print_string(b); print_string(" packages updated.\n");
    return;
  }

  print_string("Usage: ayo <list|help|add|remove|update> [package]\n");
}

// ---- ATA PIO Disk Driver ----
#define ATA_DATA     0x1F0
#define ATA_SEC_CNT  0x1F2
#define ATA_LBA_LO   0x1F3
#define ATA_LBA_MI   0x1F4
#define ATA_LBA_HI   0x1F5
#define ATA_DRIVE    0x1F6
#define ATA_CMD      0x1F7
#define ATA_STATUS   0x1F7

static int ata_drv = 0xE0;

static int ata_poll(void) {
  int t = 50000;
  while (t--) {
    uint8_t s = inb(ATA_STATUS);
    if (!(s & 0x80)) return 1;
    __asm__ volatile("pause");
  }
  return 0;
}

int ata_read_sector(uint32_t lba, uint16_t *buf) {
  if (!ata_poll()) return 0;
  outb(ATA_DRIVE, ata_drv | ((lba >> 24) & 0x0F));
  outb(ATA_SEC_CNT, 1);
  outb(ATA_LBA_LO, lba);
  outb(ATA_LBA_MI, lba >> 8);
  outb(ATA_LBA_HI, lba >> 16);
  outb(ATA_CMD, 0x20);
  int t = 200000;
  while (t--) {
    uint8_t s = inb(ATA_STATUS);
    if (s & 0x08) goto read_data;
    if (s & 0x01) return 0;
  }
  return 0;
read_data:
  for (int i = 0; i < 256; i++) buf[i] = inw(ATA_DATA);
  return 1;
}

int ata_write_sector(uint32_t lba, const uint16_t *buf) {
  if (!ata_poll()) return 0;
  outb(ATA_DRIVE, ata_drv | ((lba >> 24) & 0x0F));
  outb(ATA_SEC_CNT, 1);
  outb(ATA_LBA_LO, lba);
  outb(ATA_LBA_MI, lba >> 8);
  outb(ATA_LBA_HI, lba >> 16);
  outb(ATA_CMD, 0x30);
  int t = 200000;
  while (t--) {
    uint8_t s = inb(ATA_STATUS);
    if (s & 0x08) goto write_data;
    if (s & 0x01) return 0;
  }
  return 0;
write_data:
  for (int i = 0; i < 256; i++) outw(ATA_DATA, buf[i]);
  outb(ATA_CMD, 0xE7);
  return ata_poll();
}

static void ata_init(void) {
  uint16_t sec[256];
  disk_ok = 0;
  int t = 10000; while (--t) inb(ATA_STATUS);
  for (int try = 0; try < 2; try++) {
    ata_drv = (try == 0) ? 0xE0 : 0xF0;
    if (ata_read_sector(0, sec)) { disk_ok = 1; return; }
  }
  disk_ok = 0;
}

static int save_data(void) {
  if (!disk_ok) return 0;
  if (!hexafs_mounted) return 0;
  hexafs_save_all();
  uint8_t raw[832];
  uint32_t off = 0;
  raw[off++] = (uint8_t)(u_count & 0xFF);
  raw[off++] = (uint8_t)((u_count >> 8) & 0xFF);
  raw[off++] = (uint8_t)((u_count >> 16) & 0xFF);
  raw[off++] = (uint8_t)((u_count >> 24) & 0xFF);
  for (int i = 0; i < u_count; i++) {
    for (int j = 0; j < 32; j++) raw[off++] = (uint8_t)u_table[i].name[j];
    for (int j = 0; j < 32; j++) raw[off++] = (uint8_t)u_table[i].pass_hash[j];
    raw[off++] = (uint8_t)(u_table[i].is_root & 0xFF);
    raw[off++] = 0; raw[off++] = 0; raw[off++] = 0;
  }
  hexafs_users_save(raw, off);
  return 1;
}

static void load_data(void) {
  disk_ok = 1;
  hexafs_load_all();
  load_users();
  pimp_load_rules();
}

// ---- Diamond Feature Commands (v7.2) ----
void cmd_kstat(const char *args) {
  if (!args[0]) { print_string("Usage: kstat </@kernel/...>\n"); return; }
  char buf[1024];
  int n = kobserve_read_kernel_path(args, buf, sizeof(buf));
  if (n > 0) {
    buf[n] = 0;
    print_string(buf);
  } else {
    print_string("No such kernel observer path.\n");
  }
}

void cmd_netstat(void) {
  char buf[1024];
  net_connection_list(buf, sizeof(buf));
  print_string(buf);
}

void cmd_ifconfig(void) {
  char buf[1024];
  net_interface_list(buf, sizeof(buf));
  print_string(buf);
}

void cmd_netlog(void) {
  print_string("Network history: snapshots track network state.\n");
}

void cmd_netrollback(const char *args) {
  if (!args[0]) { print_string("Usage: netrollback <snap_name>\n"); return; }
  uint32_t snap = hexafs_snap_find(args);
  if (snap) {
    print_string("Network rollback to snapshot: ");
    print_string(args);
    print_string("\n");
  } else {
    print_string("Snapshot not found.\n");
  }
}

void cmd_replay_shell(const char *args) {
  char snap_name[32] = {0};
  int i = 0, n = 0;
  while (args[i] && args[i] != ' ' && i < 31) { snap_name[i] = args[i]; i++; }
  if (snap_name[0]) {
    uint32_t snap = hexafs_snap_find(snap_name);
    if (snap) {
      replay_execute(snap, n > 0 ? (uint32_t)n : 100);
    } else {
      print_string("Snapshot not found.\n");
    }
  } else {
    print_string("Usage: replay <snap_name> [event_count]\n");
  }
}

void cmd_bootlog(void) {
  kobserve_read_kernel_path("/@kernel/interrupts/log", 0, 0);
  print_string("Boot log: check dmesg for boot-stage messages.\n");
}

void cmd_bootpolicy(void) {
  print_string("Boot Policy Stages (default):\n");
  print_string("  1. hardware_ok\n  2. drivers_loaded\n  3. services_ready\n  4. shell_ready\n");
}

void cmd_setfallback(const char *args) {
  if (!args[0]) { print_string("Usage: setfallback <snap_name>\n"); return; }
  boot_policy_set_fallback(args);
}

void cmd_caps(const char *args) {
  char buf[512];
  int pid = current_task >= 0 ? tasks[current_task].pid : 0;
  if (args[0]) pid = atoi(args);
  hexafs_cap_list_pid((uint32_t)pid, buf, sizeof(buf));
  print_string(buf);
}

void cmd_grantcap(const char *args) {
  char pid_s[8] = {0}, cap_s[16] = {0};
  int i = 0, j = 0;
  while (args[i] && args[i] != ' ' && i < 7) { pid_s[i] = args[i]; i++; }
  while (args[i] == ' ') i++;
  while (args[i] && j < 15) { cap_s[j++] = args[i++]; }
  if (!pid_s[0] || !cap_s[0]) {
    print_string("Usage: grantcap <pid> <cap_type>\n");
    return;
  }
  uint32_t ct = (uint32_t)atoi(cap_s);
  int ret = hexafs_cap_grant((uint32_t)atoi(pid_s), ct, 0, 1);
  if (ret >= 0) {
    print_string("Capability granted.\n");
  } else {
    print_string("Grant failed.\n");
  }
}

void cmd_revokecap(const char *args) {
  (void)args;
  print_string("revokecap: capability revocation (stub)\n");
}

void cmd_hexpack(const char *args) {
  char elf_name[32] = {0}, caps_str[64] = {0};
  int i = 0, j = 0;
  while (args[i] && args[i] != ' ' && i < 31) { elf_name[i] = args[i]; i++; }
  while (args[i] == ' ') i++;
  while (args[i] && j < 63) { caps_str[j++] = args[i++]; }
  hex_pack_elf(elf_name, caps_str, 0, 0);
}

void cmd_inbox(const char *args) {
  if (!args[0]) { print_string("Usage: inbox <pid>\n"); return; }
  pid_t pid = (pid_t)atoi(args);
  int found = 0;
  for (int i = 0; i < MAX_TASKS; i++) {
    if (tasks[i].pid == pid && tasks[i].state != TASK_DEAD) {
      int count = 0;
      int t = tasks[i].event_tail;
      while (t != tasks[i].event_head) {
        count++;
        t = (t + 1) % EVENT_QUEUE_SIZE;
      }
      char buf[16];
      print_string("Inbox for PID ");
      print_string(args);
      print_string(": ");
      itoa(count, buf, 10);
      print_string(buf);
      print_string(" pending events\n");
      found = 1;
      break;
    }
  }
  if (!found) print_string("Process not found.\n");
}

void cmd_sendevent(const char *args) {
  char pid_s[8] = {0}, type_s[8] = {0};
  int i = 0, j = 0;
  while (args[i] && args[i] != ' ' && i < 7) { pid_s[i] = args[i]; i++; }
  while (args[i] == ' ') i++;
  while (args[i] && j < 7) { type_s[j++] = args[i++]; }
  if (!pid_s[0] || !type_s[0]) { print_string("Usage: sendevent <pid> <type>\n"); return; }
  pid_t tpid = (pid_t)atoi(pid_s);
  uint32_t etype = (uint32_t)atoi(type_s);
  int ret = process_event_send(tpid, etype, 0);
  if (ret == 0) {
    print_string("Event sent to PID ");
    print_string(pid_s);
    print_string("\n");
  } else {
    print_string("Failed to send event (queue full or PID not found)\n");
  }
}

void cmd_pipes_list(void) {
  char buf[512];
  int n = pipe_typed_list(buf, sizeof(buf));
  if (n > 0) {
    print_string("Active Typed Pipes:\n  ID SCHEMA  PROD CONS\n");
    print_string(buf);
  } else {
    print_string("Active typed pipes: none\n");
  }
}

void cmd_timels(const char *args) {
  if (!args[0]) { print_string("Usage: timels <path>\n"); return; }
  print_string("Versions of ");
  print_string(args);
  print_string(" across snapshots:\n");
  uint32_t snap_block = sb_cache.root_snap_block;
  int found = 0;
  while (snap_block) {
    hexafs_snap_t snap;
    if (!hexafs_block_read(snap_block, &snap)) break;
    if (snap.magic != HEXAFS_SNAP_MAGIC) break;
    uint32_t abs_block = snap.root_object_block;
    if (abs_block) {
      uint32_t obj_block = 0;
      uint8_t otype;
      if (hexafs_abstraction_find(abs_block, args, &obj_block, &otype)) {
        char buf[16];
        print_string("  snap='");
        for (int j = 0; snap.name[j] && j < 31; j++) { put_char(snap.name[j], 0x0F); }
        print_string("' block=");
        itoa((int)snap_block, buf, 10);
        print_string(buf);
        print_string("\n");
        found = 1;
      }
    }
    snap_block = snap.parent_snap_block;
  }
  if (!found) print_string("  (no versions found)\n");
}

void cmd_timediff(const char *args) {
  (void)args;
  print_string("timediff: diff object state between two timestamps (stub)\n");
}

void cmd_timeat(const char *args) {
  if (!args[0]) { print_string("Usage: timeat <timestamp> <path>\n"); return; }
  char ts[32] = {0}, path[32] = {0};
  int i = 0, j = 0;
  while (args[i] && args[i] != ' ' && i < 31) { ts[i] = args[i]; i++; }
  while (args[i] == ' ') i++;
  while (args[i] && j < 31) { path[j++] = args[i++]; }
  print_string("Reading ");
  print_string(path);
  print_string(" at timestamp ");
  print_string(ts);
  print_string("...\n");
  int idx = find_form(path);
  if (idx >= 0) {
    print_string("Current content:\n");
    print_string(form_table[idx].content);
    print_string("\n");
  } else {
    print_string("Form not found in current snapshot.\n");
  }
}

void cmd_pimp(const char *args) {
    char sub[16] = {0}, uname[32] = {0};
    int i = 0, j = 0;
    while (args[i] && args[i] != ' ' && i < 15) { sub[i] = args[i]; i++; }
    while (args[i] == ' ') i++;
    while (args[i] && j < 31) { uname[j++] = args[i++]; }
    if (strcmp(sub, "list") == 0 || sub[0] == 0) {
        char buf[512];
        pimp_rule_list(buf, sizeof(buf));
        print_string(buf);
        return;
    }
    if (strcmp(sub, "add") == 0) {
        if (!uname[0]) { print_string("Usage: pimp add <user> [caps=nopass]\n"); return; }
        uint32_t caps = 0xFFFFFFFF;
        int no_pass = 1;
        for (int k = 0; uname[k]; k++) {
            char *eq = 0;
            for (int m = 0; uname[m]; m++) { if (uname[m] == '=') { eq = &uname[m]; break; } }
            if (eq) {
                *eq = 0;
                caps = (uint32_t)atoi(eq + 1);
                no_pass = 0;
                break;
            }
        }
        if (pimp_rule_add(uname, caps, no_pass, 0)) {
            pimp_save_rules();
            print_string("Pimp rule added for ");
            print_string(uname);
            print_string("\n");
        } else { print_string("Failed.\n"); }
        return;
    }
    if (strcmp(sub, "remove") == 0) {
        if (!uname[0]) { print_string("Usage: pimp remove <user>\n"); return; }
        if (pimp_rule_remove(uname)) {
            pimp_save_rules();
            print_string("Removed.\n");
        } else { print_string("Not found.\n"); }
        return;
    }
    print_string("Usage: pimp [list|add|remove] [args]\n");
}

void cmd_dieselist(void) {
    char buf[512];
    pimp_rule_list(buf, sizeof(buf));
    print_string(buf);
}

void cmd_mode(const char *args) {
    if (strcmp(args, "list") == 0 || args[0] == 0) {
        print_string("Available video modes:\n");
        print_string("  800x600 32bpp (default)\n");
        print_string("  1024x768 32bpp\n");
        print_string("Usage: mode set <width> <height> <bpp>\n");
        return;
    }
    if (strcmp(args, "set") == 0) {
        char w_s[8]={0}, h_s[8]={0}, b_s[8]={0};
        int i = 4, j = 0;
        while (args[i] == ' ') i++;
        j = 0; while (args[i] && args[i] != ' ' && j < 7) { w_s[j++] = args[i++]; }
        while (args[i] == ' ') i++;
        j = 0; while (args[i] && args[i] != ' ' && j < 7) { h_s[j++] = args[i++]; }
        while (args[i] == ' ') i++;
        j = 0; while (args[i] && j < 7) { b_s[j++] = args[i++]; }
        if (!w_s[0] || !h_s[0]) { print_string("Usage: mode set <width> <height> [bpp]\n"); return; }
        uint16_t w = (uint16_t)atoi(w_s);
        uint16_t h = (uint16_t)atoi(h_s);
        uint16_t b = b_s[0] ? (uint16_t)atoi(b_s) : 32;
        if (fb_set_mode(w, h, b)) {
            print_string("Mode set: ");
            print_string(w_s); print_string("x"); print_string(h_s);
            print_string(" "); itoa(b, w_s, 10); print_string(w_s);
            print_string("bpp\n");
        } else {
            print_string("Mode set failed.\n");
        }
        return;
    }
    if (strcmp(args, "double") == 0) {
        fb_double_buf();
        print_string("Double buffer enabled.\n");
        return;
    }
    if (strcmp(args, "clear") == 0) {
        fb_clear(0x00000000);
        print_string("Screen cleared.\n");
        return;
    }
    if (strcmp(args, "color") == 0) {
        char r_s[8]={0}, g_s[8]={0}, b_s[8]={0};
        int i = 6, j = 0;
        while (args[i] == ' ') i++;
        j = 0; while (args[i] && args[i] != ' ' && j < 7) { r_s[j++] = args[i++]; }
        while (args[i] == ' ') i++;
        j = 0; while (args[i] && args[i] != ' ' && j < 7) { g_s[j++] = args[i++]; }
        while (args[i] == ' ') i++;
        j = 0; while (args[i] && j < 7) { b_s[j++] = args[i++]; }
        if (!r_s[0]) { print_string("Usage: mode color <r> <g> <b>\n"); return; }
        uint32_t rv = (uint32_t)(atoi(r_s) & 0xFF);
        uint32_t gv = (uint32_t)(atoi(g_s) & 0xFF);
        uint32_t bv = (uint32_t)(atoi(b_s) & 0xFF);
        uint32_t color = (rv << 16) | (gv << 8) | bv;
        fb_clear(color);
        print_string("Screen filled.\n");
        return;
    }
    print_string("Usage: mode [list|set|double|clear|color]\n");
}

// ---- Command Dispatch ----
static int execute_cmd(const char *cmd, char *args) {
  if (strcmp(cmd, "help") == 0) {
    if (args[0] && strcmp(args, "refresh") == 0) {
      print_string("Help menu refreshed.\n");
    }
    cmd_help(); return 1;
  }
  if (strcmp(cmd, "echo") == 0) { print_string(args); print_string("\n"); return 1; }
  if (strcmp(cmd, "clear") == 0) { clear_screen(); return 1; }
  if (strcmp(cmd, "reboot") == 0) { outb(0x64, 0xFE); while (1); return 1; }
  if (strcmp(cmd, "uptime") == 0) { char b[16]; itoa(ticks,b,10); print_string(b); print_string(" ticks\n"); return 1; }
  if (strcmp(cmd, "color") == 0) { current_color = atoi(args) & 0x0F; return 1; }
  if (strcmp(cmd, "about") == 0) { print_string("HEXA OS 7.2 - 32-bit hobby OS with HEXAFSv2, block cache, pimp ACLs, persistence.\n"); return 1; }
  if (strcmp(cmd, "mem") == 0) { print_string("VGA: 4000B\n"); return 1; }
  if (strcmp(cmd, "beep") == 0) { serial_putc('\a'); return 1; }
  if (strcmp(cmd, "halt") == 0) { __asm__ volatile("cli; hlt"); return 1; }
  if (strcmp(cmd, "date") == 0) { cmd_date(); return 1; }
  if (strcmp(cmd, "cpuinfo") == 0) { cmd_cpuinfo(); return 1; }
  if (strcmp(cmd, "lspci") == 0) { cmd_lspci(); return 1; }
  if (strcmp(cmd, "calc") == 0) { cmd_calc(args); return 1; }
  if (strcmp(cmd, "ascii") == 0) { cmd_ascii(); return 1; }
  if (strcmp(cmd, "palette") == 0) { cmd_palette(); return 1; }
  if (strcmp(cmd, "matrix") == 0) { cmd_matrix(); return 1; }
  if (strcmp(cmd, "guess") == 0) { cmd_guess(); return 1; }
  if (strcmp(cmd, "snake") == 0) { cmd_snake(); return 1; }
  if (strcmp(cmd, "tictactoe") == 0) { cmd_tictactoe(); return 1; }
  if (strcmp(cmd, "hangman") == 0) { cmd_hangman(); return 1; }
  if (strcmp(cmd, "memory") == 0) { cmd_memory(); return 1; }
  if (strcmp(cmd, "tetris") == 0) { cmd_tetris(); return 1; }
  if (strcmp(cmd, "neofetch") == 0) { cmd_neofetch(); return 1; }
  if (strcmp(cmd, "banner") == 0) { cmd_banner(args); return 1; }
  if (strcmp(cmd, "yes") == 0) { cmd_yes(); return 1; }
  if (strcmp(cmd, "time") == 0) { cmd_time(); return 1; }
  if (strcmp(cmd, "fortune") == 0) { cmd_fortune(); return 1; }
  if (strcmp(cmd, "shutdown") == 0) { cmd_shutdown(); return 1; }
  if (strcmp(cmd, "rand") == 0) { char b[16]; itoa(rand(),b,10); print_string(b); print_string("\n"); return 1; }
  if (strcmp(cmd, "hex") == 0) { cmd_hex(args); return 1; }
  if (strcmp(cmd, "reverse") == 0) { cmd_reverse(args); return 1; }
  if (strcmp(cmd, "len") == 0) { char b[16]; itoa(strlen(args),b,10); print_string(b); print_string("\n"); return 1; }
  if (strcmp(cmd, "tolower") == 0) { for(int k=0;args[k];k++) if(args[k]>='A'&&args[k]<='Z') args[k]+=32; print_string(args); print_string("\n"); return 1; }
  if (strcmp(cmd, "toupper") == 0) { for(int k=0;args[k];k++) if(args[k]>='a'&&args[k]<='z') args[k]-=32; print_string(args); print_string("\n"); return 1; }
  if (strcmp(cmd, "sysname") == 0) { print_string("HEXA OS 7.2 i386\n"); return 1; }
  if (strcmp(cmd, "uname") == 0) { print_string("HEXA OS 7.2 i386\n"); return 1; }
  if (strcmp(cmd, "whoami") == 0) { print_string(u_table[u_cur].name); print_string("\n"); return 1; }
  if (strcmp(cmd, "mkform") == 0) { cmd_mkform(args); save_data(); return 1; }
  if (strcmp(cmd, "list") == 0) { cmd_list(); return 1; }
  if (strcmp(cmd, "view") == 0) { cmd_view(args); return 1; }
  if (strcmp(cmd, "delete") == 0) { cmd_delete(args); save_data(); return 1; }
  if (strcmp(cmd, "write") == 0) { cmd_write(args); save_data(); return 1; }
  if (strcmp(cmd, "append") == 0) { cmd_append(args); save_data(); return 1; }
  if (strcmp(cmd, "edit") == 0) { cmd_edit(args); save_data(); return 1; }
  if (strcmp(cmd, "useradd") == 0) { cmd_useradd(args); save_data(); return 1; }
  if (strcmp(cmd, "passwd") == 0) { cmd_passwd(args); save_data(); return 1; }
  if (strcmp(cmd, "login") == 0) { cmd_login(args); return 1; }
  if (strcmp(cmd, "logout") == 0) { cmd_logout(); return 1; }
  if (strcmp(cmd, "sleep") == 0) { sleep_ticks(atoi(args) * 1000); return 1; }
  if (strcmp(cmd, "panic") == 0) { cmd_panic(); return 1; }
  if (strcmp(cmd, "outb") == 0) {
    char p_s[8]={0}, v_s[8]={0}; int i=0,j=0;
    while(args[i]&&args[i]!=' '&&j<7){p_s[j++]=args[i++];}
    while(args[i]==' '){i++;}j=0;
    while(args[i]&&j<7){v_s[j++]=args[i++];}
    if(p_s[0]&&v_s[0]) outb(atoi(p_s),(uint8_t)atoi(v_s));
    print_string("Done\n"); return 1;
  }
  if (strcmp(cmd, "inb") == 0) {
    uint16_t port=atoi(args);
    char b[16]; itoa(inb(port),b,10);
    print_string(b); print_string("\n"); return 1;
  }
  if (strcmp(cmd, "history") == 0) { cmd_hist(); return 1; }
  if (strcmp(cmd, "cowsay") == 0) { cmd_cowsay(args); return 1; }
  if (strcmp(cmd, "cmatrix") == 0) { cmd_cmatrix(); return 1; }
  if (strcmp(cmd, "dice") == 0) { cmd_dice(args); return 1; }
  if (strcmp(cmd, "8ball") == 0) { cmd_8ball(); return 1; }
  if (strcmp(cmd, "logo") == 0) { cmd_logo(); return 1; }
  if (strcmp(cmd, "pimp") == 0) { cmd_pimp(args); return 1; }
  if (strcmp(cmd, "dieselist") == 0) { cmd_dieselist(); return 1; }
  if (strcmp(cmd, "mode") == 0) { cmd_mode(args); return 1; }
  if (strcmp(cmd, "sl") == 0) { cmd_sl(); return 1; }
  if (strcmp(cmd, "morse") == 0) { cmd_morse(args); return 1; }
  if (strcmp(cmd, "russian") == 0) { cmd_russian(); return 1; }
  if (strcmp(cmd, "insult") == 0) { cmd_insult(); return 1; }
  if (strcmp(cmd, "excuse") == 0) { cmd_excuse(); return 1; }
  if (strcmp(cmd, "compliment") == 0) { cmd_compliment(); return 1; }
  if (strcmp(cmd, "hack") == 0) { cmd_hack(); return 1; }
  if (strcmp(cmd, "bsod") == 0) { cmd_bsod(); return 1; }
  if (strcmp(cmd, "ayo") == 0) { cmd_ayo(args); return 1; }
  if (strcmp(cmd, "dimpath") == 0) { cmd_dimpath(); return 1; }
  if (strcmp(cmd, "makedim") == 0) { cmd_makedim(args); return 1; }
  if (strcmp(cmd, "setmode") == 0) { cmd_setmode(args); save_data(); return 1; }
  if (strcmp(cmd, "setowner") == 0) { cmd_setowner(args); save_data(); return 1; }
  if (strcmp(cmd, "dmesg") == 0) { cmd_dmesg(); return 1; }
  if (strcmp(cmd, "clock") == 0) { cmd_clock(); return 1; }
  if (strcmp(cmd, "exec") == 0) { cmd_exec(args); return 1; }
  if (strcmp(cmd, "hexdump") == 0) { cmd_hexdump(args); return 1; }
  if (strcmp(cmd, "sysinfo") == 0) { cmd_sysinfo(); return 1; }
  if (strcmp(cmd, "kstat") == 0) { cmd_kstat(args); return 1; }
  if (strcmp(cmd, "netstat") == 0) { cmd_netstat(); return 1; }
  if (strcmp(cmd, "ifconfig") == 0) { cmd_ifconfig(); return 1; }
  if (strcmp(cmd, "netlog") == 0) { cmd_netlog(); return 1; }
  if (strcmp(cmd, "netrollback") == 0) { cmd_netrollback(args); return 1; }
  if (strcmp(cmd, "replay") == 0) { cmd_replay_shell(args); return 1; }
  if (strcmp(cmd, "bootlog") == 0) { cmd_bootlog(); return 1; }
  if (strcmp(cmd, "bootpolicy") == 0) { cmd_bootpolicy(); return 1; }
  if (strcmp(cmd, "setfallback") == 0) { cmd_setfallback(args); return 1; }
  if (strcmp(cmd, "caps") == 0) { cmd_caps(args); return 1; }
  if (strcmp(cmd, "grantcap") == 0) { cmd_grantcap(args); return 1; }
  if (strcmp(cmd, "revokecap") == 0) { cmd_revokecap(args); return 1; }
  if (strcmp(cmd, "hexpack") == 0) { cmd_hexpack(args); return 1; }
  if (strcmp(cmd, "inbox") == 0) { cmd_inbox(args); return 1; }
  if (strcmp(cmd, "sendevent") == 0) { cmd_sendevent(args); return 1; }
  if (strcmp(cmd, "pipes") == 0) { cmd_pipes_list(); return 1; }
  if (strcmp(cmd, "timels") == 0) { cmd_timels(args); return 1; }
  if (strcmp(cmd, "timediff") == 0) { cmd_timediff(args); return 1; }
  if (strcmp(cmd, "timeat") == 0) { cmd_timeat(args); return 1; }
  if (strcmp(cmd, "move") == 0) {
    char src[32]={0}, dst[32]={0}; int i=0,j=0;
    while(args[i]&&args[i]!=' '&&j<31){src[j++]=args[i++];}
    while(args[i]==' '){i++;} j=0;
    while(args[i]&&j<31){dst[j++]=args[i++];}
    if(!src[0]||!dst[0]){print_string("Usage: move <src> <dst>\n");return 1;}
    int idx=find_form(src);
    if(idx<0){print_string("Not found.\n");return 1;}
    if(check_perm(idx,1)){print_color("Denied.\n",0x0C);return 1;}
    strcpy(form_table[idx].name, dst);
    save_data(); print_string("Renamed.\n"); return 1;
  }
  if (strcmp(cmd, "copy") == 0) {
    char src[32]={0}, dst[32]={0}; int i=0,j=0;
    while(args[i]&&args[i]!=' '&&j<31){src[j++]=args[i++];}
    while(args[i]==' '){i++;} j=0;
    while(args[i]&&j<31){dst[j++]=args[i++];}
    if(!src[0]||!dst[0]){print_string("Usage: copy <src> <dst>\n");return 1;}
    int si=find_form(src);
    if(si<0){print_string("Not found.\n");return 1;}
    if(check_perm(si,0)){print_color("Denied.\n",0x0C);return 1;}
    if(form_count>=MAX_FORMS){print_string("Full.\n");return 1;}
    form_ensure_cap(form_count, form_table[si].size);
    strcpy(form_table[form_count].name, dst);
    memcpy(form_table[form_count].content, form_table[si].content, form_table[si].size);
    form_table[form_count].size = form_table[si].size;
    form_table[form_count].owner = u_cur;
    form_table[form_count].mode = form_table[si].mode;
    form_count++;
    save_data(); print_string("Copied.\n"); return 1;
  }
  if (strcmp(cmd, "ps") == 0) {
    char b[16];
    print_string("PID NAME             STATE\n");
    for (int i = 0; i < MAX_TASKS; i++) {
      if (tasks[i].pid < 0) continue;
      itoa(tasks[i].pid, b, 10); print_string(b); print_string("   ");
      print_string(tasks[i].name);
      for (int k = strlen(tasks[i].name); k < 16; k++) print_string(" ");
      const char *s = "?";
      if (tasks[i].state == 0) s = "READY";
      else if (tasks[i].state == 1) s = "RUN";
      else if (tasks[i].state == 2) s = "BLKD";
      else if (tasks[i].state == 3) s = "ZOMB";
      print_string(s); print_string("\n");
    }
    return 1;
  }
  if (strcmp(cmd, "df") == 0) {
    char b[32];
    uint32_t fp = pmm_count_free();
    print_string("Forms: "); itoa(form_count, b, 10); print_string(b);
    print_string("/"); itoa(MAX_FORMS, b, 10); print_string(b);
    print_string("  Mem: "); itoa(fp * 4, b, 10); print_string(b); print_string("KB free\n");
    return 1;
  }
  if (strcmp(cmd, "which") == 0) {
    if (!args[0]) { print_string("Usage: which <name>\n"); return 1; }
    int fi = find_form(args);
    if (fi >= 0) { print_string(args); print_string(": form\n"); return 1; }
    for (int ui = 0; ui < u_count; ui++)
      if (strcmp(u_table[ui].name, args) == 0) { print_string(args); print_string(": user\n"); return 1; }
    const char *known[] = {"help","echo","clear","reboot","uptime","color","about","mem","beep","halt","date","cpuinfo","lspci","calc","ascii","palette","matrix","guess","snake","tictactoe","hangman","memory","tetris","neofetch","banner","yes","time","fortune","shutdown","rand","hex","reverse","len","tolower","toupper","sysname","uname","whoami","mkform","list","view","delete","write","append","edit","useradd","passwd","login","logout","sleep","panic","outb","inb","history","cowsay","cmatrix","dice","8ball","logo","pimp","dieselist","mode","sl","morse","russian","insult","excuse","compliment","hack","bsod","ayo","dimpath","makedim","setmode","setowner","dmesg","clock","exec","hexdump","sysinfo","kstat","netstat","ifconfig","netlog","netrollback","replay","bootlog","bootpolicy","setfallback","caps","grantcap","revokecap","hexpack","inbox","sendevent","pipes","timels","timediff","timeat","move","copy","diese","ps","df","which","cat","head","true","false","env",0};
    for (int ki = 0; known[ki]; ki++)
      if (strcmp(known[ki], args) == 0) { print_string(args); print_string(": command\n"); return 1; }
    print_string(args); print_string(": not found\n");
    return 1;
  }
  if (strcmp(cmd, "cat") == 0) {
    if (!args[0]) { print_string("Usage: cat <form>\n"); return 1; }
    int fi = find_form(args);
    if (fi < 0) { print_string("Not found.\n"); return 1; }
    if (check_perm(fi, 0)) { print_color("Denied.\n", 0x0C); return 1; }
    print_string(form_table[fi].content); print_string("\n");
    return 1;
  }
  if (strcmp(cmd, "head") == 0) {
    char fn[32]={0}, ln[8]={0}; int i=0,j=0;
    while(args[i]&&args[i]!=' '&&j<31){fn[j++]=args[i++];}
    while(args[i]==' '){i++;} j=0;
    while(args[i]&&j<7){ln[j++]=args[i++];}
    int fi=find_form(fn);
    if(fi<0){print_string("Not found.\n");return 1;}
    if(check_perm(fi,0)){print_color("Denied.\n",0x0C);return 1;}
    int n=ln[0]?atoi(ln):10;
    if(n<1)n=1;
    const char *p=form_table[fi].content;
    while(*p&&n>0){if(*p=='\n')n--;put_char(*p,current_color);p++;}
    if(*(p-1)!='\n')print_string("\n");
    return 1;
  }
  if (strcmp(cmd, "true") == 0) { return 1; }
  if (strcmp(cmd, "false") == 0) { return 1; }
  if (strcmp(cmd, "env") == 0) {
    print_string("SHELL=hexa\n");
    print_string("OS=HexaOS/7.2\n");
    print_string("ARCH=i386\n");
    print_string("USER="); print_string(u_table[u_cur].name); print_string("\n");
    char b[16];
    print_string("FORMS="); itoa(form_count, b, 10); print_string(b); print_string("\n");
    print_string("TICKS="); itoa(ticks, b, 10); print_string(b); print_string("\n");
    return 1;
  }
  if (strcmp(cmd, "diese") == 0) {
    if (!args[0]) { print_string("Usage: diese <command> [args]\n"); return 1; }
    if (u_cur == 0) { print_string("Already root.\n"); return 1; }
    char uname[32];
    strcpy(uname, u_table[u_cur].name);
    int no_pass = pimp_check(uname, 0xFFFFFFFF);
    if (!no_pass) {
      char pw[32];
      print_string("password: ");
      get_line(pw, 32);
      if (!check_pwd(pw, u_table[u_cur].pass_hash)) { print_color("Access denied.\n", 0x0C); return 1; }
      if (u_table[u_cur].pass_hash[2] == ' ') {
        encode_pwd(u_table[u_cur].pass_hash, pw, ticks & 0xFFFF);
        save_data();
      }
    }
    char c[32]={0}, a[96]={0}; int i=0,j=0;
    while(args[i]&&args[i]!=' '&&j<31){c[j++]=args[i++];}
    while(args[i]==' '){i++;} j=0;
    while(args[i]&&j<95){a[j++]=args[i++];}
    int saved=u_cur; u_cur=0;
    if (no_pass) { print_color("[diese] (pimped) ", 0x0A); }
    else { print_color("[diese] ", 0x0C); }
    execute_cmd(c, a);
    u_cur=saved;
    return 1;
  }
  return 0;
}


// ----------------- Kernel Entry -----------------
void kernel_main(void) __attribute__((section(".text.entry")));

// Alias kernel_entry to kernel_main for bootloader compatibility
__asm__(".globl kernel_entry; .set kernel_entry, kernel_main;");

// Dummy entry for shell task slot — state gets overwritten on first context switch
void shell_entry(void) {
    sti();
    while (1) hlt();
}

// User-space entry point
void _user_entry(void) {
    while (1) {
        uint32_t t;
        __asm__ volatile("mov $3, %%eax; int $0x80" : "=a"(t));
        for (volatile int i = 0; i < 50000; i++);
    }
}

void kernel_main(void) {
  for (char *p = _bss_start; p < _bss_end; p++) *p = 0;

  rseed = get_rtc_register(0x00);

  // Mask all PIC IRQs to prevent spurious interrupts during init
  outb(0x21, 0xFF);
  outb(0xA1, 0xFF);

  print_color("[BOOT] Setting up GDT...\n", 0x0A);
  setup_gdt();

  print_color("[BOOT] Setting up IDT...\n", 0x0A);
  idt_init();
  pic_remap();
  pit_init(100);

  print_color("[BOOT] Enabling paging...\n", 0x0A);
  pmm_init(32 * 1024 * 1024, (uint32_t)&_kernel_end);
  paging_init();
  kheap_init();

  print_color("[BOOT] Initializing logging...\n", 0x0A);
  log_init();

  print_color("[BOOT] Initializing kernel observers...\n", 0x0A);
  kobserve_init();

  print_color("[BOOT] Initializing intent system...\n", 0x0A);
  intent_init();

  print_color("[BOOT] Initializing replay system...\n", 0x0A);
  replay_init();

  print_color("[BOOT] Initializing network stack...\n", 0x0A);
  net_init();
  net_register_observers();

  print_color("[BOOT] Initializing framebuffer...\n", 0x0A);
  fb_init();

  print_color("[BOOT] Initializing task manager...\n", 0x0A);
  proc_init();
  vfs_init();

  // Create tasks BEFORE enabling interrupts
  int user_pid = proc_create(_user_entry, "user_task", 1);
  if (user_pid >= 0) {
      char nb[16]; int bi = 0;
      int n = user_pid; if (n == 0) { nb[bi++] = '0'; } else { while (n) { nb[bi++] = '0' + (n % 10); n /= 10; } nb[bi] = 0; for (int k=0;k<bi/2;k++){char t=nb[k];nb[k]=nb[bi-1-k];nb[bi-1-k]=t;} }
      print_color("[BOOT] User task created, PID=", 0x0A);
      print_color(nb, 0x0A);
      print_color("\n", 0x0A);
  }

  int shell_pid = proc_create(shell_entry, "shell", 1);
  if (shell_pid >= 0) {
      current_task = 1;
      tasks[1].state = TASK_RUNNING;
  }

  print_color("[BOOT] Enabling interrupts...\n", 0x0A);
  sti();

  init_users();
  ata_init();
  if (disk_ok) {
    hexafs_mount();
  } else {
    print_color("[BOOT] No disk - formatting...\n", 0x0E);
    hexafs_format();
    hexafs_mount();
  }
  load_data();

  print_color("[BOOT] Loading pimp rules...\n", 0x0A);
  pimp_load_rules();

  print_color("[BOOT] Executing boot policy...\n", 0x0A);
  boot_policy_execute();
  do_login();

  serial_putc('['); serial_putc('O'); serial_putc('K'); serial_putc(']'); serial_putc('\n');
  log_write(LOG_LEVEL_INFO, "System boot complete");

  // Fall into the shell — first timer IRQ will save our state and switch
  char input_buffer[128], cmd[32], args[96];

  while (1) {
    do_tick();
    print_color(u_table[u_cur].name, 0x0B);
    print_string("@");
    print_string(hostname_str);
    if (u_table[u_cur].is_root)
      print_color("# ", 0x0C);
    else
      print_color("$ ", 0x0A);
    get_line(input_buffer, sizeof(input_buffer));

    if (input_buffer[0]) {
      strcpy(hist_buf[hist_idx], input_buffer);
      hist_idx = (hist_idx + 1) % HIST_MAX;
      if (hist_count < HIST_MAX) hist_count++;
    }

    int i = 0, j = 0;
    while (input_buffer[i] != ' ' && input_buffer[i] != '\0' && j < 31)
      cmd[j++] = input_buffer[i++];
    cmd[j] = '\0';
    while (input_buffer[i] == ' ')
      i++;
    j = 0;
    while (input_buffer[i] != '\0' && j < 95)
      args[j++] = input_buffer[i++];
    args[j] = '\0';

    if (cmd[0] == '\0')
      continue;

    // Alias expansion
    for (int ai = 0; ai < alias_count; ai++) {
      if (strcmp(cmd, alias_table[ai].name) == 0) {
        char expanded[128];
        strcpy(expanded, alias_table[ai].cmd);
        if (args[0]) {
          strcat(expanded, " ");
          strcat(expanded, args);
        }
        int ei = 0, ej = 0;
        while (expanded[ei] != ' ' && expanded[ei] != '\0' && ej < 31)
          cmd[ej++] = expanded[ei++];
        cmd[ej] = '\0';
        while (expanded[ei] == ' ') ei++;
        ej = 0;
        while (expanded[ei] != '\0' && ej < 95)
          args[ej++] = expanded[ei++];
        args[ej] = '\0';
        break;
      }
    }

    if (!execute_cmd(cmd, args)) {
      print_color("Unknown: ", 0x0C);
      print_color(cmd, 0x0C);
      print_string("\n");
    }
  }
}


