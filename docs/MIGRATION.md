# Alpha-to-v8 migration map

The old implementation remains buildable so useful engineering is not lost.
Ports land only after their interfaces are expressed in Form-native terms.

| Alpha source | Useful implementation | v8 destination / rule | Status |
|---|---|---|---|
| `interrupts`, `paging` | IDT, PIC, PIT, PMM, heap | x86_64 platform layer; no semantic leakage | queued |
| `process`, `sync` | scheduler, task state, locking | Form execution contexts and kernel synchronization | queued |
| `intent`, `pipe` | typed intent and event transport | inter-Form messaging through Handles | queued |
| `hexafs*` | ATA I/O, cache, journal, revisions | persistent Form graph and FIN index | metadata model plus a separate primary-master ATA PIO account/preference state journal landed; general persistent Form storage queued |
| `driver`, `net`, `fb` | device, RTL8139, framebuffer code | capability-gated Driver Forms | runtime 640x480/1280x720/1920x1080 Bochs/QEMU framebuffer, two-page VBE presentation with bounded damage clipping/coalescing and counters, direct-front recovery, bounded-retrace VSync, persistent 60/75/120/144 Hz compositor pacing and opt-in responsive damaged commits landed; polling RTL8139 Ethernet, ARP, static IPv4, ICMP, UDP, DNS, one TCP client, HTTP/1.0 and bounded authenticated TLS 1.3 HTTPS landed; scoped DigiCert trust for DuckDuckGo search landed; GPU acceleration, physical monitor negotiation, DHCP, IPv6 and physical Wi-Fi queued |
| `boot_policy`, `replay`, `kobserve`, `log` | recovery and observability | PIMP/DIESE diagnostics and revision journal | bounded Form-native typed tunables, 16-watch/16-ready signal/timer/resource event service, resource accounting and denial diagnostics landed; durable policy/replay, scheduler and interrupt integration queued; this is not a FreeBSD ABI or source port |
| `elf`, `syscall` | loading and ring transition experience | Form implementation loader and non-POSIX call ABI | Go ABI v1 landed; loader/ring transition queued |
| `vfs` | console/device plumbing | adapters only; no path-first public VFS | quarantined |
| `hexa.c` | shell, users, apps, games, commands | Interface Forms after kernel primitives mature | shell, persistent account registry, Form-native surface commit/damage/frame-completion semantics, bounded pointer-motion coalescing, customizable empty-start desktop with low-cost defaults, five font faces/three real weights, all-edge taskbar, bounded off-screen/snap/focus/window-decoration controls, 480p-safe Settings viewports, graphical/console two-eye `neofetch`, graphical `windowreset`, richer Terminal and bounded HTML/CSS/JavaScript HTTP/HTTPS Browser with canonical DuckDuckGo non-JavaScript HTML address-bar search landed; this is not Wayland client compatibility; search follows at most three redirects without allowing an HTTPS-to-HTTP downgrade and projects up to eight result titles and links from a 14 KiB fetched body prefix under a 16 KiB parser bound; arbitrary ECMAScript, external resources, Web APIs, cookies/storage, media playback and GPU rendering are not ported; the embedded trust store is not a general CA bundle; other app ports queued |
| expodOS console | serial, VGA, locks, long-mode entry | v8 bootstrap platform layer | landed |

## Required sequence

1. Replace transitional GRUB loading with a UEFI loader and a documented boot
   handoff structure.
2. Add x86_64 memory discovery, page allocation, exceptions and interrupts.
3. Port scheduler and typed messaging around Form execution contexts.
   Bind the current runtime-local tunable, readiness and resource-accounting
   primitives to those contexts without importing the FreeBSD ABI.
4. Extend the landed ATA PIO dual-slot account/preference journal into a
   general HexaFS block format with a persistent Form graph, durable
   transactions, recovery and FIN index. The compact state image is not that
   filesystem.
5. Move capability checks into every driver, message and persistence entry.
6. Persist PIMP revisions and run essential DIESE evaluation during boot.
7. Connect Go `ayo` to kernel Handles and remove its JSON development bridge.
8. Create Root/system/interface Forms. Integrate ASL only after its actual
   specification is supplied.

`legacy/alpha32/` remains an untouched migration reference in this feature
pass. New customization stays in the v8 `EXPOST03` preference extension; fresh
or older compatible records receive the lowest-cost defaults: 480p, 60 Hz,
Efficient presentation, contained windows, bottom 28 px taskbar, and optional
effects disabled.
