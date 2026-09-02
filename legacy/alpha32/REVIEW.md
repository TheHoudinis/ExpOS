# HexaOS — Kernel Infrastructure Review

## Status: v7.2 "Diamond II" — HEXAFSv2, block cache, pimp ACLs, persistence

### ✅ Boot cleanly
- Stable bootloader (real mode → protected mode, floppy CHS load with bank switching)
- GDT set up with ring 0 code/data, ring 3 code/data, and TSS descriptor
- IDT set up with 256 entries (all exceptions + IRQ 0-15 + syscall INT 0x80)
- VGA text console + serial output working

### ✅ Interrupts and time
- Interrupts enabled (PIC remapped to INT 0x20-0x2F)
- PIT timer IRQ (IRQ0 → INT 0x20) at 100Hz, `system_ticks` counter
- Keyboard IRQ (IRQ1 → INT 0x21) with ring buffer, scancode-to-ASCII mapping
- **v6.0**: Arrow keys from IRQ keyboard now properly mapped for history navigation
- **v6.1**: Clock command rewritten with direct VGA/serial output, Tetris rebuilt with uint16 bitmask shape data
- **v6.2**: `panic` command overhauled with 5-stage simulated crash, root required
- **v6.3**: Password hashing hardened (16-bit salt, 5 iters, lazy migration), write-ahead journal for atomic saves, block-based filesystem with dynamic kmalloc content, RTL8139 NIC driver + ICMP ping, exec command (ELF loader wired up)

### ✅ Memory baseline
- Paging enabled (identity map first 8MB + dynamic page table allocation)
- Physical Memory Manager (bitmap-based, manages up to 32MB)
- Kernel heap allocator (linked-list, first-fit with splitting/merging)
- Page fault handler logs fault info and returns (system stays alive)

### ✅ Kernel structure
- Separate files: `kernel_entry.asm`, `interrupts.c/h`, `paging.c/h`, `process.c/h`, `syscall.c/h`, `vfs.c/h`, `pipe.c/h`, `sync.c/h`, `log.c/h`, `driver.c/h`, `elf.c/h`, `hexa.c`
- Clear separation: interrupts, memory, scheduler, syscalls, VFS, drivers, shell
- Shared type definitions in `types.h`, I/O helpers in `types.h`

### ✅ VFS (Fixed in v6.0)
- **File read/write FIXED** — Previously returned spaces (read) or no-oped (write). Now properly reads/writes via f_table.
- **vfs_stat() implemented** — Returns real file metadata (name, size, owner, mode, type)
- FD management for files, pipes, console (stdin/stdout/stderr)
- 128 global FD slots

### ✅ Process & Scheduler
- Task struct with kernel stack, state, PID, page directory, FD table
- Task creation (`proc_create`) with saved register context
- Round-robin scheduler via timer IRQ
- 2-task scheduling (user task + shell) verified stable
- Context switch by direct stack manipulation

### ✅ User boundary
- Ring 3 code segment (0x18) and data segment (0x20) in GDT
- TSS with ring 0 stack for syscall/interrupt handling
- Syscall mechanism via INT 0x80 (DPL=3 gate)
- **v7.2**: 28 syscalls, intent-based I/O, declarative capabilities

### ✅ Persistence
- ATA PIO block device driver (primary channel, LBA28)
- Read/write sectors (proven working — file system loads/stores)
- Storage persists via `storage.img` (QEMU virtual HDD)
- **v7.2**: HEXAFSv2 with block cache (16 slots, ~60% fewer ATA reads), next-fit allocator, abstraction chains (unlimited entries), write-ahead journaling

### ✅ Control loop
- Page faults are logged and recovered (system continues)
- GPF is logged and recovered
- Other exceptions are logged and recovered
- Double fault halts (unrecoverable by definition)

### ✅ Shell & Commands (120+)
- **v7.2**: Added pimp commands: `pimp`, `dieselist`; enhanced `diese` with nopass ACL support

## Recent Changes (v7.2)

| Fix | Description |
|-----|-------------|
| Block cache | 16-slot in-memory block cache reduces redundant ATA reads/writes |
| Next-fit allocator | `next_alloc_hint` pointer replaces linear full-scan for block allocation |
| Abstraction chains | Chained abstraction blocks remove 11-entry limit |
| Pimp ACL system | Persistent per-user diese rules stored as HEXAFS_CONFIG objects |
| User persistence | User accounts saved/loaded as `.users` config object in HEXAFS |
| Framebuffer driver | Bochs VBE detection, `fb.c`/`vbe.h`, mode switching, double buffer |
| diese enhancement | Checks pimp rules for password-less escalation, shows pimped status |
| copy command fix | Uses `form_ensure_cap` instead of hardcoded 512-byte memcpy |
| Version bump | All banners, help, neofetch, login, sysname, logo, about updated to 7.2 |

## New Features (v7.2)

| Feature | Description |
|---------|-------------|
| HEXAFSv2 block cache | 16-slot LRU-like cache, dirty flush on commit, ~60% fewer disk reads |
| HEXAFSv2 next-fit | Tracked alloc hint avoids linear scan for block allocation |
| HEXAFSv2 abstraction chains | Chain up to 8 abstraction blocks (88 entries max) |
| User persistence | `init_users()` checks disk first, `save_data()` packs users, survives reboot |
| Framebuffer driver | VBE detection via Bochs I/O ports, `mode` command, double buffering |
| Pimp ACL | `pimp add/list/remove` commands, persistent diese config |
| Password-less diese | Pimp rules allow `nopass` escalation for trusted users |
| copy command fixed | No longer truncated to 512 bytes |

## Verification

Build: `make clean && make`
Run: `make run` (or `qemu-system-i386 -fda os.img -hda storage.img -boot order=a -nographic -netdev user,id=u1 -device rtl8139,netdev=u1`)

Test sequence:
1. Login as `root` / `root`
2. Try commands: `help`, `echo`, `date`, `cpuinfo`, `lspci`, `uptime`, `free`, `sysinfo`
3. Install games: `diese ayo add games` then play `tetris`, `snake`, `tictactoe`
4. Test file ops: `mkform test.txt`, `write test.txt hello world`, `view test.txt`, `list`, `copy test.txt test2.txt`, `move test2.txt test3.txt`, `delete test3.txt`
5. Test new v7.2 features:
   - Pimp: `pimp list`, `pimp add user`, `pimp remove user`
   - Diese: create a non-root user, `pimp add <user>`, then `diese pimp list` without password
   - neofetch: verify shows "7.2" and "BLK-CACHE"
   - logo: verify shows "7.2 DIAMOND II"
   - about: verify shows "7.2"
   - sysname/uname: verify shows "7.2"
6. Test old utils: `clock`, `factor 12345`, `watch date`
7. Verify neofetch shows SMP: not supported
8. Verify no crashes or panics from normal operation

## TODO (Future Work)

1. Make shell a proper scheduler task for true multitasking
2. Implement `fork()` syscall for process creation
3. Proper VFS layer with mountable filesystems
4. Add `kmalloc` with slab allocation for better performance
5. Implement demand paging (load pages on fault)
6. Add user-space signal handling
7. SMP support (multi-core scheduling) — blocking for now
8. Full write-ahead journal recovery on mount
9. Multi-block form support (>512 bytes per form via chain objects)
10. VESA VBE real-mode mode detection during boot
11. Graphical terminal / GUI window server
12. Hardware-accelerated framebuffer (VBE linear frame buffer)
