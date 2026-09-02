#include "types.h"
#include "paging.h"
#include "interrupts.h"
#include "log.h"

#define PMM_MAX_MEM  (32 * 1024 * 1024)
#define PMM_PAGE_CNT (PMM_MAX_MEM / PAGE_SIZE)
#define PMM_BMAP_SZ  (PMM_PAGE_CNT / 8)

static uint8_t pmm_bitmap[PMM_BMAP_SZ];
static uint32_t pmm_max_page = 0;
static uint32_t pmm_last_alloc = 0;

uint32_t pmm_get_total_pages(void) { return pmm_max_page; }

#define PAGE_FREE 0
#define PAGE_USED 1

static void pmm_set(uint32_t page) {
    pmm_bitmap[page / 8] |= (1 << (page % 8));
}
static void pmm_clear(uint32_t page) {
    pmm_bitmap[page / 8] &= ~(1 << (page % 8));
}
static int pmm_test(uint32_t page) {
    return (pmm_bitmap[page / 8] >> (page % 8)) & 1;
}

void pmm_init(uint32_t mem_size, uint32_t kernel_end) {
    uint32_t top = mem_size > PMM_MAX_MEM ? PMM_MAX_MEM : mem_size;
    pmm_max_page = top / PAGE_SIZE;
    for (uint32_t i = 0; i < PMM_BMAP_SZ; i++) pmm_bitmap[i] = 0xFF;
    uint32_t kernel_pages = kernel_end / PAGE_SIZE;
    if (kernel_end % PAGE_SIZE) kernel_pages++;
    if (kernel_pages > pmm_max_page) kernel_pages = pmm_max_page;
    for (uint32_t i = kernel_pages; i < pmm_max_page; i++) pmm_clear(i);
    pmm_set(0);
    for (uint32_t i = 0xA0; i < 0x100; i++) pmm_set(i);
    pmm_last_alloc = kernel_pages;
    log_write(LOG_LEVEL_INFO, "PMM: initialized");
}

uint32_t pmm_alloc(void) {
    for (uint32_t i = pmm_last_alloc; i < pmm_max_page; i++) {
        if (!pmm_test(i)) {
            pmm_set(i);
            pmm_last_alloc = i + 1;
            return i * PAGE_SIZE;
        }
    }
    for (uint32_t i = 1; i < pmm_last_alloc; i++) {
        if (!pmm_test(i)) {
            pmm_set(i);
            pmm_last_alloc = i + 1;
            return i * PAGE_SIZE;
        }
    }
    return 0;
}

void pmm_free(uint32_t addr) {
    uint32_t page = addr / PAGE_SIZE;
    if (page < pmm_max_page) {
        pmm_clear(page);
        if (page < pmm_last_alloc) pmm_last_alloc = page;
    }
}

uint32_t pmm_count_free(void) {
    uint32_t cnt = 0;
    for (uint32_t i = 1; i < pmm_max_page; i++) if (!pmm_test(i)) cnt++;
    return cnt;
}

static uint32_t kernel_page_dir_phys = 0;
static uint32_t *kernel_page_dir = 0;

void paging_init(void) {
    kernel_page_dir_phys = pmm_alloc();
    kernel_page_dir = (uint32_t *)kernel_page_dir_phys;
    for (int i = 0; i < 1024; i++) kernel_page_dir[i] = 0x002;
    for (int tbl = 0; tbl < 2; tbl++) {
        uint32_t pt_phys = pmm_alloc();
        uint32_t *pt = (uint32_t *)pt_phys;
        for (int i = 0; i < 1024; i++) {
            pt[i] = ((tbl * 0x400000) + (i * PAGE_SIZE)) | 0x003;
        }
        kernel_page_dir[tbl] = pt_phys | 0x003;
    }
    __asm__ volatile("mov %0, %%cr3" : : "r"(kernel_page_dir_phys));
    uint32_t cr0;
    __asm__ volatile("mov %%cr0, %0" : "=r"(cr0));
    cr0 |= 0x80000000;
    __asm__ volatile("mov %0, %%cr0" : : "r"(cr0));
    log_write(LOG_LEVEL_INFO, "Paging: enabled, 8MB identity mapped");
}

uint32_t get_kernel_page_dir(void) {
    return kernel_page_dir_phys;
}

uint32_t create_user_page_dir(void) {
    uint32_t pd_phys = pmm_alloc();
    uint32_t *pd = (uint32_t *)pd_phys;
    for (int i = 0; i < 1024; i++) pd[i] = 0x002;
    // Copy kernel mappings (top 256 entries = 3GB+)
    for (int i = 0; i < 256; i++) pd[i] = kernel_page_dir[i];
    return pd_phys;
}

void switch_page_dir(uint32_t pd) {
    if (pd) __asm__ volatile("mov %0, %%cr3" : : "r"(pd));
}

void map_page(uint32_t virt, uint32_t phys, uint32_t flags) {
    uint32_t pd_idx = virt >> 22;
    uint32_t pt_idx = (virt >> 12) & 0x3FF;
    uint32_t pd_phys;
    __asm__ volatile("mov %%cr3, %0" : "=r"(pd_phys));
    uint32_t *pd = (uint32_t *)pd_phys;
    if (!(pd[pd_idx] & 1)) {
        uint32_t pt_phys = pmm_alloc();
        uint32_t *pt = (uint32_t *)pt_phys;
        for (int i = 0; i < 1024; i++) pt[i] = 0x002;
        pd[pd_idx] = pt_phys | 0x003;
        __asm__ volatile("mov %0, %%cr3" : : "r"(pd_phys));
    }
    uint32_t *pt = (uint32_t *)(pd[pd_idx] & ~0xFFF);
    pt[pt_idx] = (phys & ~0xFFF) | (flags & 0xFFF);
}

void unmap_page(uint32_t virt) {
    uint32_t pd_idx = virt >> 22;
    uint32_t pt_idx = (virt >> 12) & 0x3FF;
    uint32_t pd_phys;
    __asm__ volatile("mov %%cr3, %0" : "=r"(pd_phys));
    uint32_t *pd = (uint32_t *)pd_phys;
    if (!(pd[pd_idx] & 1)) return;
    uint32_t *pt = (uint32_t *)(pd[pd_idx] & ~0xFFF);
    pt[pt_idx] = 0x002;
    __asm__ volatile("invlpg (%0)" : : "r"(virt));
}

int page_fault_handler(uint32_t cr2, uint32_t err, uint32_t eip) {
    int user = (err & 4) != 0;
    (void)eip;
    if (user || cr2 >= 0x40000000) {
        log_write_hex(LOG_LEVEL_WARN, "PF killing task: addr=", cr2);
        extern void proc_exit(int code);
        proc_exit(-1);
        return 1;
    }
    // Kernel PF — log but recover
    log_write_hex(LOG_LEVEL_WARN, "Kernel PF at ", cr2);
    log_write_hex(LOG_LEVEL_WARN, "EIP=", eip);
    return 1;
}

#define HEAP_START 0x00800000
#define HEAP_INIT_SIZE (1024 * 1024)

typedef struct heap_block {
    uint32_t size;
    int free;
    struct heap_block *next;
} heap_block_t;

static heap_block_t *heap_first = 0;
static uint32_t heap_phys_start = 0;
static uint32_t heap_phys_end = 0;

static void map_heap_page(uint32_t addr) {
    uint32_t pd_phys;
    __asm__ volatile("mov %%cr3, %0" : "=r"(pd_phys));
    uint32_t *pd = (uint32_t *)pd_phys;
    uint32_t pd_idx = addr >> 22;
    uint32_t pt_idx = (addr >> 12) & 0x3FF;
    if (!(pd[pd_idx] & 1)) {
        uint32_t pt_phys = pmm_alloc();
        uint32_t *pt = (uint32_t *)pt_phys;
        for (int i = 0; i < 1024; i++) pt[i] = 0x002;
        pd[pd_idx] = pt_phys | 0x003;
        __asm__ volatile("mov %0, %%cr3" : : "r"(pd_phys));
    }
    uint32_t *pt = (uint32_t *)(pd[pd_idx] & ~0xFFF);
    if (!(pt[pt_idx] & 1)) {
        uint32_t phys = pmm_alloc();
        if (phys) pt[pt_idx] = phys | 0x003;
    }
}

void kheap_init(void) {
    heap_phys_start = HEAP_START;
    heap_phys_end = HEAP_START + HEAP_INIT_SIZE;
    for (uint32_t addr = HEAP_START; addr < heap_phys_end; addr += PAGE_SIZE) {
        map_heap_page(addr);
    }
    __asm__ volatile("mov %0, %%cr3" : : "r"(get_kernel_page_dir()));
    heap_first = (heap_block_t *)HEAP_START;
    heap_first->size = HEAP_INIT_SIZE - sizeof(heap_block_t);
    heap_first->free = 1;
    heap_first->next = 0;
    log_write(LOG_LEVEL_INFO, "Kheap: initialized at 8MB");
}

void *kmalloc(size_t size) {
    if (!heap_first || size == 0) return 0;
    size = (size + 3) & ~3;
    heap_block_t *curr = heap_first;
    while (curr) {
        if (curr->free && curr->size >= size) {
            if (curr->size > size + sizeof(heap_block_t) + 4) {
                heap_block_t *new_block = (heap_block_t *)((uint32_t)curr + sizeof(heap_block_t) + size);
                new_block->size = curr->size - size - sizeof(heap_block_t);
                new_block->free = 1;
                new_block->next = curr->next;
                curr->size = size;
                curr->next = new_block;
            }
            curr->free = 0;
            return (void *)((uint32_t)curr + sizeof(heap_block_t));
        }
        curr = curr->next;
    }
    return 0;
}

void kfree(void *ptr) {
    if (!ptr) return;
    heap_block_t *block = (heap_block_t *)((uint32_t)ptr - sizeof(heap_block_t));
    block->free = 1;
    if (block->next && block->next->free) {
        block->size += sizeof(heap_block_t) + block->next->size;
        block->next = block->next->next;
    }
    heap_block_t *curr = heap_first;
    while (curr && curr->next != block) curr = curr->next;
    if (curr && curr->free) {
        curr->size += sizeof(heap_block_t) + block->size;
        curr->next = block->next;
    }
}

void *krealloc(void *ptr, size_t size) {
    if (!ptr) return kmalloc(size);
    heap_block_t *block = (heap_block_t *)((uint32_t)ptr - sizeof(heap_block_t));
    if (block->size >= size) return ptr;
    void *new_ptr = kmalloc(size);
    if (new_ptr) {
        uint8_t *d = (uint8_t *)new_ptr;
        uint8_t *s = (uint8_t *)ptr;
        for (uint32_t i = 0; i < block->size; i++) d[i] = s[i];
        kfree(ptr);
    }
    return new_ptr;
}
