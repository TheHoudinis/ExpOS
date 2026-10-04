# Alpha-to-v9 migration map

The old implementation remains buildable so useful engineering is not lost.
Ports land only after their interfaces are expressed in Form-native terms.

| Alpha source | Useful implementation | v9 destination / rule | Status |
|---|---|---|---|
| `interrupts`, `paging` | IDT, PIC, PIT, PMM, heap | x86_64 platform layer; no semantic leakage | long-mode per-CPU TSS/IDT/GDT state, remapped PIC, BSP PIT and AP local-APIC timers, static per-Form page-table arenas and CPL3 fault containment landed; ACPI MADT CPU discovery and INIT/SIPI startup bring up to eight CPUs; PCI MSI/MSI-X vector/message preparation landed but device handlers are not armed; general physical-memory discovery/allocation and full exception coverage remain queued |
| `process`, `sync` | scheduler, task state, locking | Form execution contexts, kernel synchronization, ExpScope confinement and per-context ExpBudget enforcement | CFC/Dimension/FIN contexts now switch CR3 and complete register images, run executable capsules at CPL3, preempt on hardware timers, charge real CPU ticks to ExpBudget and can reserve independent AP jobs for concurrent Form slices; `execute-parallel` exposes a bounded foreground pair while a general user-facing background job controller and interrupt-driven device/event delivery remain queued |
| `intent`, `pipe` | typed intent and event transport | inter-Form messaging through CFC-scoped Form Handles governed by ExpSeal | CFC-scoped Handles, strictly attenuating delegation, revocation and broker-lifetime root-issuance sealing landed in the semantic core; durable per-context seals and universal boundary enforcement are queued |
| `expfs*` | ATA I/O, cache, journal, revisions | per-CFC persistent Form graph and FIN index with AEAD, wrapped random storage keys, eight rotating checkpoints and protected installation baseline | atomic ExpFS current/checkpoint/baseline records now run over NVMe, AHCI SATA or ATA-PIO; arbitrary Form content survives reboot with Dimensions, relationships, revisions, PIMP state, capabilities, accounts and settings. EXPOST03 is read-only migration input; Basic AEAD/key wrapping and Operator-only baseline restore are live |
| `driver`, `net`, `fb` | device, RTL8139, framebuffer code | capability-gated Driver Forms | ASL v1 inventory/exclusive claims; direct UEFI GOP with Bochs BIOS fallback; xHCI boot keyboard/mouse; NVMe queues and AHCI DMA; polling RTL8139 Ethernet, ARP, DHCP, IPv4, ICMP, UDP, DNS, TCP, HTTP and bounded TLS 1.3 landed. Device completion remains polling until prepared MSI/MSI-X vectors gain IDT handlers; GPU acceleration, EDID/mode negotiation, USB hubs/classes, IPv6 and physical Wi-Fi remain queued |
| `boot_policy`, `replay`, `kobserve`, `log` | recovery and observability | UEFI-first CFC selection/unlock, PIMP/DIESE diagnostics and authenticated checkpoint recovery | native UEFI default and BIOS compatibility/recovery/dev fallback, encrypted Genesis unlock, authenticated full-state checkpoint rotation/restore, protected-baseline restore, bounded Form-native typed tunables, 16-watch/16-ready signal/timer/resource events, resource accounting and denial diagnostics landed; graphical recovery selection, durable replay and interrupt integration remain queued |
| `elf`, `syscall` | loading and ring transition experience | Form implementation loader and non-POSIX call ABI | language-neutral Form ABI v1 plus Rust/Go/C/Python SDK contracts, requester-bound native call gate, bounded capsule loader, shared ABI page, `iretq` Ring 3 transition, `EXECUTION_YIELD` and `EXECUTION_EXIT` landed; general ELF/SDK binary loading and remaining buffer grants are queued |
| `vfs` | console/device plumbing | adapters only; no path-first public VFS | quarantined |
| `expos.c` | shell, users, apps, games, commands | Interface Forms after kernel primitives mature | shell, persistent account registry, Form-native surface commit/damage/frame-completion semantics, bounded pointer-motion coalescing, customizable empty-start desktop with low-cost defaults, five font faces/three real weights, all-edge taskbar, bounded off-screen/snap/focus/window-decoration controls, 480p-safe Settings viewports, graphical/console two-eye `neofetch`, graphical `windowreset`, richer Terminal and bounded HTML/CSS/JavaScript HTTP/HTTPS Browser with six tabs, per-tab history/scroll, eight bookmarks, find-in-page, Chromium-like chrome and canonical DuckDuckGo non-JavaScript HTML address-bar search landed; this is neither Wayland client compatibility nor Chromium/Blink/V8 compatibility; search follows at most three redirects without allowing an HTTPS-to-HTTP downgrade and projects up to eight result titles and links from a 14 KiB fetched body prefix under a 16 KiB parser bound; arbitrary ECMAScript, external resources, Web APIs, cookies/storage, media playback and GPU rendering are not ported; the embedded trust store is not a general CA bundle; other app ports queued |
| expodOS console | serial, VGA, locks, long-mode entry | v9 bootstrap platform layer | landed |

## Required sequence

1. Preserve native UEFI as the normal/Genesis path and BIOS/GRUB as the approved
   compatibility, recovery, and development fallback; keep both handoffs
   documented and tested.
2. Carry the landed CFC core model through Genesis and every native subsystem:
   typed CFC FIN, required nonempty name, exactly one Primary Dimension,
   exclusive ownership, and explicit rejection of cross-CFC
   Form/data/Dimension/capability/storage sharing.
3. Extend the landed static x86_64 Form page-table arenas, TSS/IDT/PIC/PIT and
   CPL3 fault containment with firmware memory discovery, a general physical
   page allocator and complete exception coverage.
4. Extend the landed interrupt-preempted, CR3/register-switching SMP Form
   scheduler from the bounded `execute-parallel` pair to a user-facing
   background job controller, then connect typed messaging and interrupt-driven
   device events.
5. Protect the landed per-CFC ExpFS current-state/checkpoint database with an independent
   random storage key per encrypted CFC, Argon2id-derived KEK wrapping and AEAD;
   and store the protected immutable installation baseline separately from the
   landed eight-slot rotating checkpoint ring.
6. Carry the landed CFC-scoped ExpSeal issuance/delegation/sealing rules into
   every driver, message, execution, and persistence entry.
7. Persist PIMP revisions and run essential DIESE evaluation during boot.
8. Connect Go `ayo` to kernel Handles and remove its JSON development bridge.
9. Create Root/system/interface Forms above the landed ASL v1 PCI/firmware
   inventory and exclusive-claim boundary. Hardware Handle grants and a
   non-x86 backend remain pending; see `ASL_V1.md`.

Architect is the official user-facing name for the configurable installation
path; “expert” is only a legacy explanation. Exact CFC disk structures, AEAD
choice, Argon2id parameters, key-envelope/nonce encoding, checkpoint rotation,
and crash recovery remain design work within the approved invariants above.

`legacy/alpha32/` remains an untouched migration reference in this feature
pass. New customization stays in the v9 `EXPOST03` preference extension; its
existing theme, accent and backdrop bytes now expose their full 256-value
procedural palette ranges without a disk-format change. Fresh or older
compatible records receive the lowest-cost defaults: 480p, 60 Hz, Efficient
presentation, contained windows, bottom 28 px taskbar, and optional effects
disabled.
