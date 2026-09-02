#ifndef INTERRUPTS_H
#define INTERRUPTS_H

#include "types.h"

#define IDT_ENTRIES 256
#define PIC1_CMD    0x20
#define PIC1_DATA   0x21
#define PIC2_CMD    0xA0
#define PIC2_DATA   0xA1
#define PIC_EOI     0x20

#define PIT_CH0     0x40
#define PIT_CMD     0x43

// IDT entry structure
struct idt_entry {
    uint16_t base_lo;
    uint16_t sel;
    uint8_t always0;
    uint8_t flags;
    uint16_t base_hi;
} __attribute__((packed));

struct idt_ptr {
    uint16_t limit;
    uint32_t base;
} __attribute__((packed));

// Register state pushed by interrupt stubs
struct regs {
    uint32_t gs, fs, es, ds;
    uint32_t edi, esi, ebp, esp, ebx, edx, ecx, eax;
    uint32_t int_no, err_code;
    uint32_t eip, cs, eflags, user_esp, user_ss;
};

// Keyboard buffer
#define KBUF_SIZE 256
extern volatile char kbuf[KBUF_SIZE];
extern volatile int kbuf_head, kbuf_tail;
extern volatile uint32_t system_ticks;

void idt_init(void);
void pic_remap(void);
void pit_init(uint32_t freq);
void isr_handler(struct regs *r);
void irq_handler(struct regs *r);
int kb_getchar(void);
void setup_gdt(void);

#endif
