# Alpha-to-v8 migration map

The old implementation remains buildable so useful engineering is not lost.
Ports land only after their interfaces are expressed in Form-native terms.

| Alpha source | Useful implementation | v8 destination / rule | Status |
|---|---|---|---|
| `interrupts`, `paging` | IDT, PIC, PIT, PMM, heap | x86_64 platform layer; no semantic leakage | queued |
| `process`, `sync` | scheduler, task state, locking | Form execution contexts and kernel synchronization | queued |
| `intent`, `pipe` | typed intent and event transport | inter-Form messaging through Handles | queued |
| `hexafs*` | ATA I/O, cache, journal, revisions | persistent Form graph and FIN index | metadata model landed |
| `driver`, `net`, `fb` | device, RTL8139, framebuffer code | capability-gated Driver Forms | Bochs/QEMU framebuffer landed; network queued |
| `boot_policy`, `replay`, `kobserve`, `log` | recovery and observability | PIMP/DIESE diagnostics and revision journal | queued |
| `elf`, `syscall` | loading and ring transition experience | Form implementation loader and non-POSIX call ABI | Go ABI v1 landed; loader/ring transition queued |
| `vfs` | console/device plumbing | adapters only; no path-first public VFS | quarantined |
| `hexa.c` | shell, users, apps, games, commands | Interface Forms after kernel primitives mature | shell, HexaDisplay and local Browser landed; app ports queued |
| expodOS console | serial, VGA, locks, long-mode entry | v8 bootstrap platform layer | landed |

## Required sequence

1. Replace transitional GRUB loading with a UEFI loader and a documented boot
   handoff structure.
2. Add x86_64 memory discovery, page allocation, exceptions and interrupts.
3. Port scheduler and typed messaging around Form execution contexts.
4. Connect the existing HexaFS metadata transaction model to a real block
   driver, durable journal, checksums, recovery and FIN index.
5. Move capability checks into every driver, message and persistence entry.
6. Persist PIMP revisions and run essential DIESE evaluation during boot.
7. Connect Go `ayo` to kernel Handles and remove its JSON development bridge.
8. Create Root/system/interface Forms. Integrate ASL only after its actual
   specification is supplied.
