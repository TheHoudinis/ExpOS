/* ExpOS native x86-64 UEFI loader. UEFI 2.10 ABI; no hosted C runtime.
 * This boot protocol is an implementation version, not the final Genesis
 * disk/encryption contract. See docs/BOOT.md for boundaries and boot options.
 */
typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;
typedef u64 usize;
typedef u64 Status;
typedef void *Handle;
#define EFI_ERROR(n) (0x8000000000000000ULL | (n))
#define PAGE 4096ULL
#define MAGIC 0x45585055U
#define INFO_MAGIC 0x4558504f53454649ULL

typedef struct { u64 signature; u32 revision, size, crc, reserved; } Header;
typedef struct { u32 a; u16 b, c; u8 d[8]; } Guid;
typedef struct {
    Header header;
    void *raise_tpl, *restore_tpl;
    Status (*allocate_pages)(u32, u32, usize, u64 *);
    Status (*free_pages)(u64, usize);
    Status (*get_memory_map)(usize *, void *, usize *, usize *, u32 *);
    void *allocate_pool, *free_pool;
    void *create_event, *set_timer, *wait_for_event, *signal_event, *close_event, *check_event;
    void *install_protocol, *reinstall_protocol, *uninstall_protocol;
    Status (*handle_protocol)(Handle, Guid *, void **);
    void *reserved, *register_protocol, *locate_handle, *locate_device_path, *install_configuration;
    void *load_image, *start_image, *exit, *unload_image;
    Status (*exit_boot_services)(Handle, usize);
    void *get_monotonic_count, *stall, *set_watchdog;
    void *connect_controller, *disconnect_controller, *open_protocol, *close_protocol;
    void *open_protocol_information, *protocols_per_handle, *locate_handle_buffer;
    Status (*locate_protocol)(Guid *, void *, void **);
} BootServices;
typedef struct {
    void *reset;
    Status (*output_string)(void *, u16 *);
} TextOutput;
typedef struct {
    Header header;
    void *vendor;
    u32 revision;
    Handle input_handle;
    void *input;
    Handle output_handle;
    TextOutput *output;
    Handle error_handle;
    void *error, *runtime;
    BootServices *boot;
    usize table_count;
    void *tables;
} SystemTable;
typedef struct {
    u32 version, width, height, format;
    u32 masks[4], stride;
} GraphicsInfo;
typedef struct {
    u32 maximum_mode, mode;
    GraphicsInfo *info;
    usize info_size;
    u64 framebuffer;
    usize framebuffer_size;
} GraphicsMode;
typedef struct { void *query, *set, *blt; GraphicsMode *mode; } Graphics;
typedef struct {
    u32 revision;
    Handle parent;
    SystemTable *system;
    Handle device;
    void *path, *reserved;
    u32 options_size;
    u16 *options;
} LoadedImage;
typedef struct {
    u64 signature;
    u32 version, size;
    u64 memory_map, memory_map_size, descriptor_size;
    u32 descriptor_version, boot_mode;
    u64 framebuffer, framebuffer_size;
    u32 width, height, stride, format;
} BootInfo;
typedef struct {
    u8 ident[16];
    u16 type, machine;
    u32 version;
    u64 entry, phoff, shoff;
    u32 flags;
    u16 ehsize, phentsize, phnum, shentsize, shnum, shstrndx;
} ElfHeader;
typedef struct { u32 type, flags; u64 offset, vaddr, paddr, filesz, memsz, align; } ProgramHeader;

extern const u8 kernel_start[], kernel_end[];

void *memcpy(void *destination, const void *source, usize length) {
    u8 *out = destination;
    const u8 *in = source;
    for (usize i = 0; i < length; i++) out[i] = in[i];
    return destination;
}
void *memset(void *destination, int value, usize length) {
    u8 *out = destination;
    for (usize i = 0; i < length; i++) out[i] = (u8)value;
    return destination;
}
static Status failure(SystemTable *system, u16 *message, Status status) {
    system->output->output_string(system->output, message);
    return status;
}
static int matches(u16 *options, usize count, const char *word) {
    for (usize start = 0; start < count; start++) {
        if (start && options[start - 1] != ' ') continue;
        usize i = 0;
        while (word[i] && start + i < count && options[start + i] == (u8)word[i]) i++;
        if (!word[i] && (start + i == count || !options[start + i] || options[start + i] == ' ')) return 1;
    }
    return 0;
}

Status efi_main(Handle image, SystemTable *system) {
    if (!system || system->header.signature != 0x5453595320494249ULL) return EFI_ERROR(2);
    BootServices *boot = system->boot;
    system->output->output_string(system->output, L"ExpOS native UEFI handoff\r\n");
    const usize length = kernel_end - kernel_start;
    const ElfHeader *elf = (const void *)kernel_start;
    if (length < sizeof(*elf) || elf->ident[0] != 0x7f || elf->ident[1] != 'E' ||
        elf->ident[2] != 'L' || elf->ident[3] != 'F' || elf->ident[4] != 2 ||
        elf->ident[5] != 1 || elf->type != 2 || elf->machine != 62 ||
        elf->phentsize != sizeof(ProgramHeader) || !elf->phnum ||
        elf->phoff > length || elf->phnum > (length - elf->phoff) / sizeof(ProgramHeader))
        return failure(system, L"Invalid ExpOS kernel ELF\r\n", EFI_ERROR(1));
    const ProgramHeader *segments = (const void *)(kernel_start + elf->phoff);
    u64 bottom = ~0ULL, top = 0;
    int entry_valid = 0;
    for (u16 i = 0; i < elf->phnum; i++) {
        const ProgramHeader *p = &segments[i];
        if (p->type != 1) continue;
        if (p->filesz > p->memsz || p->offset > length || p->filesz > length - p->offset ||
            p->vaddr != p->paddr || p->paddr < 0x100000 || p->paddr >= 0x40000000 ||
            p->memsz > 0x40000000 - p->paddr)
            return failure(system, L"Unsafe ExpOS kernel segment\r\n", EFI_ERROR(1));
        if (!p->memsz) continue;
        for (u16 j = 0; j < i; j++) {
            const ProgramHeader *previous = &segments[j];
            if (previous->type == 1 && previous->memsz &&
                p->paddr < previous->paddr + previous->memsz &&
                previous->paddr < p->paddr + p->memsz)
                return failure(system, L"Overlapping ExpOS kernel segments\r\n", EFI_ERROR(1));
        }
        if ((p->flags & 1) && elf->entry >= p->paddr && elf->entry - p->paddr < p->filesz) entry_valid = 1;
        if ((p->paddr & ~(PAGE - 1)) < bottom) bottom = p->paddr & ~(PAGE - 1);
        u64 end = (p->paddr + p->memsz + PAGE - 1) & ~(PAGE - 1);
        if (end > top) top = end;
    }
    if (!entry_valid || bottom >= top) return failure(system, L"Invalid ExpOS entry point\r\n", EFI_ERROR(1));
    Status status = boot->allocate_pages(2, 2, (top - bottom) / PAGE, &bottom);
    if (status) return failure(system, L"Cannot reserve ExpOS kernel memory\r\n", status);
    memset((void *)bottom, 0, top - bottom);
    for (u16 i = 0; i < elf->phnum; i++) {
        const ProgramHeader *p = &segments[i];
        if (p->type == 1 && p->filesz) memcpy((void *)p->paddr, kernel_start + p->offset, p->filesz);
    }
    u64 info_address = 0x3fffffff;
    status = boot->allocate_pages(1, 2, 1, &info_address);
    if (status) return failure(system, L"Cannot reserve ExpOS handoff\r\n", status);
    BootInfo *info = (void *)info_address;
    memset(info, 0, PAGE);
    info->signature = INFO_MAGIC;
    info->version = 1;
    info->size = sizeof(*info);
    Guid loaded_guid = {0x5b1b31a1, 0x9562, 0x11d2, {0x8e,0x3f,0,0xa0,0xc9,0x69,0x72,0x3b}};
    LoadedImage *loaded = 0;
    if (!boot->handle_protocol(image, &loaded_guid, (void **)&loaded) && loaded->options && loaded->options_size <= 4096) {
        usize count = loaded->options_size / 2;
        if (matches(loaded->options, count, "single")) info->boot_mode = 1;
        else if (matches(loaded->options, count, "console")) info->boot_mode = 2;
        else if (matches(loaded->options, count, "desktop")) info->boot_mode = 3;
    }
    Guid graphics_guid = {0x9042a9de, 0x23dc, 0x4a38, {0x96,0xfb,0x7a,0xde,0xd0,0x80,0x51,0x6a}};
    Graphics *graphics = 0;
    if (!boot->locate_protocol(&graphics_guid, 0, (void **)&graphics) && graphics->mode && graphics->mode->info) {
        info->framebuffer = graphics->mode->framebuffer;
        info->framebuffer_size = graphics->mode->framebuffer_size;
        info->width = graphics->mode->info->width;
        info->height = graphics->mode->info->height;
        info->stride = graphics->mode->info->stride;
        info->format = graphics->mode->info->format;
    }
    // Size first; reserve descriptor slack because this allocation changes the map.
    usize map_size = 0, key = 0, descriptor_size = 0;
    u32 descriptor_version = 0;
    status = boot->get_memory_map(&map_size, 0, &key, &descriptor_size, &descriptor_version);
    if (status != EFI_ERROR(5) || descriptor_size < 40 || descriptor_size > 4096 || map_size > 16 * 1024 * 1024)
        return failure(system, L"Cannot size firmware memory map\r\n", EFI_ERROR(1));
    usize map_pages = (map_size + 32 * descriptor_size + PAGE - 1) / PAGE;
    u64 map_address = 0x3fffffff;
    status = boot->allocate_pages(1, 2, map_pages, &map_address);
    if (status) return failure(system, L"Cannot reserve firmware memory map\r\n", status);
    for (u32 attempt = 0; attempt < 3; attempt++) {
        map_size = map_pages * PAGE;
        status = boot->get_memory_map(&map_size, (void *)map_address, &key, &descriptor_size, &descriptor_version);
        if (status) break;
        info->memory_map = map_address;
        info->memory_map_size = map_size;
        info->descriptor_size = descriptor_size;
        info->descriptor_version = descriptor_version;
        // No allocation, console output or protocol calls between these calls.
        status = boot->exit_boot_services(image, key);
        if (!status) {
            __asm__ volatile ("cli; cld" ::: "memory");
            typedef void (__attribute__((sysv_abi)) *Entry)(u32, u64);
            ((Entry)elf->entry)(MAGIC, info_address);
            for (;;) __asm__ volatile ("hlt");
        }
        if (status != EFI_ERROR(2)) break;
    }
    // After a failed ExitBootServices only memory-map / exit calls are safe.
    for (;;) __asm__ volatile ("cli; hlt");
}
