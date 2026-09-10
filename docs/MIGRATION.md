# Alpha-to-v8 migration map

The old implementation remains buildable so useful engineering is not lost.
Ports land only after their interfaces are expressed in Form-native terms.

| Alpha source | Useful implementation | v8 destination / rule | Status |
|---|---|---|---|
| `interrupts`, `paging` | IDT, PIC, PIT, PMM, heap | x86_64 platform layer; no semantic leakage | queued |
| `process`, `sync` | scheduler, task state, locking | Form execution contexts and kernel synchronization | queued |
| `intent`, `pipe` | typed intent and event transport | inter-Form messaging through Handles | queued |
| `hexafs*` | ATA I/O, cache, journal, revisions | persistent Form graph and FIN index | metadata model plus a separate primary-master ATA PIO account/preference state journal landed; general persistent Form storage queued |
| `driver`, `net`, `fb` | device, RTL8139, framebuffer code | capability-gated Driver Forms | runtime 640x480/1280x720/1920x1080 Bochs/QEMU framebuffer; polling RTL8139 Ethernet, ARP, static IPv4, ICMP, UDP, DNS, one TCP client, HTTP/1.0 and bounded authenticated TLS 1.3 HTTPS landed; DHCP, IPv6 and physical Wi-Fi queued |
| `boot_policy`, `replay`, `kobserve`, `log` | recovery and observability | PIMP/DIESE diagnostics and revision journal | queued |
| `elf`, `syscall` | loading and ring transition experience | Form implementation loader and non-POSIX call ABI | Go ABI v1 landed; loader/ring transition queued |
| `vfs` | console/device plumbing | adapters only; no path-first public VFS | quarantined |
| `hexa.c` | shell, users, apps, games, commands | Interface Forms after kernel primitives mature | shell, persistent account registry, responsive HexaDisplay, customizable empty-start desktop, richer graphical Terminal and bounded HTTP/HTTPS Browser landed; app ports queued |
| expodOS console | serial, VGA, locks, long-mode entry | v8 bootstrap platform layer | landed |

## Required sequence

1. Replace transitional GRUB loading with a UEFI loader and a documented boot
   handoff structure.
2. Add x86_64 memory discovery, page allocation, exceptions and interrupts.
3. Port scheduler and typed messaging around Form execution contexts.
4. Extend the landed ATA PIO dual-slot account/preference journal into a
   general HexaFS block format with a persistent Form graph, durable
   transactions, recovery and FIN index. The compact state image is not that
   filesystem.
5. Move capability checks into every driver, message and persistence entry.
6. Persist PIMP revisions and run essential DIESE evaluation during boot.
7. Connect Go `ayo` to kernel Handles and remove its JSON development bridge.
8. Create Root/system/interface Forms. Integrate ASL only after its actual
   specification is supplied.
