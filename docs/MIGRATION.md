# Alpha-to-v8 migration map

The old implementation remains buildable so useful engineering is not lost.
Ports land only after their interfaces are expressed in Form-native terms.

| Alpha source | Useful implementation | v8 destination / rule | Status |
|---|---|---|---|
| `interrupts`, `paging` | IDT, PIC, PIT, PMM, heap | x86_64 platform layer; no semantic leakage | queued |
| `process`, `sync` | scheduler, task state, locking | Form execution contexts, kernel synchronization, ExpScope confinement and per-context ExpBudget enforcement | deny-by-default CFC/Dimension/Form reachability and fixed context-keyed accounting landed in the semantic core; the live kernel ledger remains shell-global and address-space/scheduler enforcement is queued |
| `intent`, `pipe` | typed intent and event transport | inter-Form messaging through CFC-scoped Form Handles governed by ExpSeal | CFC-scoped Handles, strictly attenuating delegation, revocation and broker-lifetime root-issuance sealing landed in the semantic core; durable per-context seals and universal boundary enforcement are queued |
| `expfs*` | ATA I/O, cache, journal, revisions | per-CFC persistent Form graph and FIN index with AEAD, wrapped random storage keys, eight rotating checkpoints and protected installation baseline | CFC-owned in-memory transactions and a metadata-only recovery model landed, with a CFC-bound baseline descriptor excluded from an up-to-eight-entry rotating ring; disk capture, authentication, encryption, persistence, protection and restore remain queued, and the separate native ATA account/preference journal remains plaintext CRC |
| `driver`, `net`, `fb` | device, RTL8139, framebuffer code | capability-gated Driver Forms | runtime 640x480/1280x720/1920x1080 Bochs/QEMU framebuffer, two-page VBE presentation with bounded damage clipping/coalescing and counters, direct-front recovery, bounded-retrace VSync, persistent 60/75/120/144 Hz compositor pacing and opt-in responsive damaged commits landed; polling RTL8139 Ethernet, ARP, static IPv4, ICMP, UDP, DNS, one TCP client, HTTP/1.0 and bounded authenticated TLS 1.3 HTTPS landed; scoped DigiCert trust for DuckDuckGo search landed; GPU acceleration, physical monitor negotiation, DHCP, IPv6 and physical Wi-Fi queued |
| `boot_policy`, `replay`, `kobserve`, `log` | recovery and observability | UEFI-first CFC selection/unlock, PIMP/DIESE diagnostics and authenticated checkpoint recovery | native UEFI default and BIOS compatibility/recovery/dev fallback landed; bounded Form-native typed tunables, 16-watch/16-ready signal/timer/resource event service, resource accounting and denial diagnostics landed; Genesis unlock, eight-checkpoint/baseline recovery, durable policy/replay, scheduler and interrupt integration queued; this is not a FreeBSD ABI or source port |
| `elf`, `syscall` | loading and ring transition experience | Form implementation loader and non-POSIX call ABI | Go ABI v1 landed; loader/ring transition queued |
| `vfs` | console/device plumbing | adapters only; no path-first public VFS | quarantined |
| `expos.c` | shell, users, apps, games, commands | Interface Forms after kernel primitives mature | shell, persistent account registry, Form-native surface commit/damage/frame-completion semantics, bounded pointer-motion coalescing, customizable empty-start desktop with low-cost defaults, five font faces/three real weights, all-edge taskbar, bounded off-screen/snap/focus/window-decoration controls, 480p-safe Settings viewports, graphical/console two-eye `neofetch`, graphical `windowreset`, richer Terminal and bounded HTML/CSS/JavaScript HTTP/HTTPS Browser with canonical DuckDuckGo non-JavaScript HTML address-bar search landed; this is not Wayland client compatibility; search follows at most three redirects without allowing an HTTPS-to-HTTP downgrade and projects up to eight result titles and links from a 14 KiB fetched body prefix under a 16 KiB parser bound; arbitrary ECMAScript, external resources, Web APIs, cookies/storage, media playback and GPU rendering are not ported; the embedded trust store is not a general CA bundle; other app ports queued |
| expodOS console | serial, VGA, locks, long-mode entry | v8 bootstrap platform layer | landed |

## Required sequence

1. Preserve native UEFI as the normal/Genesis path and BIOS/GRUB as the approved
   compatibility, recovery, and development fallback; keep both handoffs
   documented and tested.
2. Carry the landed CFC core model through Genesis and every native subsystem:
   typed CFC FIN, required nonempty name, exactly one Primary Dimension,
   exclusive ownership, and explicit rejection of cross-CFC
   Form/data/Dimension/capability/storage sharing.
3. Add x86_64 memory discovery, page allocation, exceptions and interrupts.
4. Port scheduler and typed messaging around Form execution contexts. Integrate
   ExpScope confinement and ExpBudget accounting/enforcement with real contexts
   without importing the FreeBSD ABI.
5. Extend the landed ATA PIO dual-slot account/preference journal into a
   per-CFC ExpFS block format with a persistent Form graph and FIN index. Use an
   independent random storage key per encrypted CFC, Argon2id-derived KEK wrapping,
   authenticated encryption, eight rotating checkpoints, and a protected
   immutable installation baseline. The compact plaintext CRC state image is
   not that filesystem.
6. Carry the landed CFC-scoped ExpSeal issuance/delegation/sealing rules into
   every driver, message, execution, and persistence entry.
7. Persist PIMP revisions and run essential DIESE evaluation during boot.
8. Connect Go `ayo` to kernel Handles and remove its JSON development bridge.
9. Create Root/system/interface Forms. Use the Abstracted Silicon Layer (ASL) role described in
   `GENESIS_DESIGN.txt`; its cross-architecture implementation is still pending.

Architect is the official user-facing name for the configurable installation
path; “expert” is only a legacy explanation. Exact CFC disk structures, AEAD
choice, Argon2id parameters, key-envelope/nonce encoding, checkpoint rotation,
and crash recovery remain design work within the approved invariants above.

`legacy/alpha32/` remains an untouched migration reference in this feature
pass. New customization stays in the v8 `EXPOST03` preference extension; fresh
or older compatible records receive the lowest-cost defaults: 480p, 60 Hz,
Efficient presentation, contained windows, bottom 28 px taskbar, and optional
effects disabled.
