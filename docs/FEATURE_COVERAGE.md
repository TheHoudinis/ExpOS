# ExpOS feature coverage

ExpOS v8.0.0-alpha.12 uses two runnable environments during migration. `make
run` boots the new x86_64 Form-native kernel. `make run-alpha` boots the original
32-bit Diamond II system with its existing storage image. Native persistence is
tested with `make persistence-check`; live authenticated HTTPS is tested with
`make https-check` and requires Internet access.

## Native v8 commands

| Area | Commands / behavior |
|---|---|
| Forms | `mkform`, `forms/list`, `inspect/fin`, exact `resolve`, `view/cat`, `write`, `append`, `head`, `copy`, `move`, `delete`, `recover`, `retire`, `activate`, guarded `reclaim`, `hexdump`, `du`, `df`, `shasum`, `which`; typed `relate`/`unrelate`/`relationships` |
| Dimensions and policy | `dimensions`, `makedim`, `policy`, `pimp`, `journal` |
| Capabilities | `grant`, `revoke`, `handles`, and `handlecheck` with requester-bound, scoped, expiring Form Handles; non-amplifying delegation; parent-linked revocation cascades; explicit Display and Input rights |
| Kernel controls | Form-native, runtime-local `sysctl` typed tunables; `kqueue`/`kevent` fixed-capacity signal, one-shot/periodic timer and resource-denial watches with sequenced readiness, configurable dispatch batching and optional coalescing; `rlimit` accounting for event watches plus explicit runtime-local IPC-byte, scratch-page and Form-operation reservations with validated soft/hard/ceiling tuples and denial counters/events; DIESE checks the revocable bootstrap Handle, reserves Configure for Operator, grants Execute mutation to Power/Operator and leaves Guest read-only; inspired by FreeBSD interface concepts without source or ABI compatibility |
| Packages | `Ayo` boot Package Form and Go `ayo v3`; 21-package built-in Prism catalog; searchable TUI; offline/HTTPS catalogs and Ed25519 catalog signatures; SHA-256 artifact verification; raw/tar/tar.gz extraction; owned-file receipts and collision protection; atomic dependency install, uninstall, rollback and crash recovery; no package scripts or links |
| Graphics | `HexaDisplay` Service Form; fresh-state 480p/60 Hz and Efficient-renderer defaults; runtime 640x480 (480p), 1280x720 (720p) and 1920x1080 (1080p) Bochs/QEMU XRGB scanout with XRGB8888, ARGB8888 and RGB565 client formats; two-page virtual framebuffer with explicit boot/login presentation, verified VBE Y-offset flips, bounded allocation-free damage clipping/coalescing and direct-front recovery when a flip is rejected; submitted/copied region and pixel, collapse and flip-failure counters; rejected modes automatically retry at 480p; selectable 60/75/120/144 Hz compositor targets, optional bounded-retrace VSync, frame/miss/timeout diagnostics; responsive flat dark desktop with compact menu and all-edge taskbar, no default or pinned apps, movable/minimizable/maximizable/closeable windows, configurable bounded off-screen travel and snapping, Notes and eleven surfaces; Form-owned pending surface state, atomic commit, coalesced presentation-bound `FrameDone`, configure/focus/key/pointer events, z-order and hit testing; bounded pure-motion input coalescing that preserves key/button edges; VGA text-mode restoration on exit; `desktop`, `displayinfo`, `displaydiag`, `safevideo/displayreset` |
| Browser | Native `Browser` Interface Form; fixed-capacity HTML title/heading/paragraph/list/link/button parsing and graphical rendering; tag/class/id/inline CSS subset; deterministic document title/text/style/visibility and click-handler JavaScript subset; local `hexa://` navigation plus capability-gated native `http://` and authenticated TLS 1.3 `https://` fetches over DNS/TCP/HTTP; plain address-bar text searches DuckDuckGo's canonical non-JavaScript HTML endpoint, follows at most three redirects without allowing an HTTPS-to-HTTP downgrade, and projects up to eight result titles and links; verified `www.youtube.com` HTML fetch is not video playback |
| Desktop apps | Dark graphical Terminal, Browser, Form registry, Ayo package catalog, System, Games and Notes; large clickable Settings control center with eleven pages including Appearance, Windows and Taskbar, plus compact category/row viewport scrolling that keeps long pages usable at 480p; 106 directly working Appearance/Windows/Taskbar selectable values and 112 schema-accepted states including 28 reserved interaction states (overlapping counts); six themes, seven procedural wallpapers, four cursor themes, four accents, five bitmap font faces and Light/Regular/Bold rasterized weights; configurable window corners, borders, titlebar height, backdrop/titlebar opacity, bounded off-screen travel, snap distance and click/sloppy/pointer focus; bottom/top/left/right taskbar, nine sizes, three app alignments, edge-reveal auto-hide, translucency, clipped horizontal labels and a live RTC clock with optional seconds; optional cursor shadow; persistent presentation controls and low-cost defaults; Terminal with 40-line scrollback, 24-entry Up/Down history, two-eye `neofetch` and `windowreset`; every app can close and reopen |
| Go | Shared ABI v1 call numbers and validation in Rust, tested `sdk/go/hexa` client and emulator, `GoABI` Interface Form and `goabi` diagnostics |
| Hardware | `date/clock`, `timers`, `cpuinfo`, `features/kernelcaps`, `lspci`, `neofetch/sysinfo`, `mem/free`, VGA and COM1 consoles, calibrated CPUID/fallback TSC timing, PS/2 keyboard/mouse, dedicated primary-master ATA PIO state transport, RTL8139 bus-master DMA, Ethernet/ARP/static IPv4/ICMP/UDP/DNS/TCP/HTTP/TLS, PCI Wi-Fi/Bluetooth class discovery with honest driver/connection state, ANSI serial input, RDRAND detection and CPUID/control-register reporting |
| System | Graphical-or-console boot chooser and mutually switchable login, Operator/Power/Guest capability authority, persistent twelve-slot accounts, salted PBKDF2-HMAC-SHA256 password verifiers, persistent desktop/connectivity preferences with fail-safe decoding, dual-slot CRC state recovery, `users`, `useradd`, `userdel`, `passwd`, `login`, `logout`, `status`, `kstat`, `ps`, `dmesg/bootlog`, `stateinfo`, combined `diag/diagnose`, `ifconfig`, `ping`, `dns`, `fetch`, `netstat`, history, `whoami`, `reboot`, `shutdown` |
| Utilities | `calc`, `factor`, `len`, `hex`, `reverse/rev`, `tolower`, `toupper`, `rand`, `sleep`, `true`, `false` |

Native Form contents currently use fixed 512-byte in-memory records. Delete is
recovery-aware: it moves a Form to Recoverable; the core only permits final
reclamation after Dimension bindings are removed. Accounts and desktop
preferences are the durable exception: a dedicated ATA state image alternates
two fixed 2 KiB slots, each protected by format/version fields, generation and
CRC-32. Display rate and VSync selections are included; compatible records
without the timing extension load as 60 Hz with VSync enabled.
`runtime/expos-state.img` is created by `make run` and is preserved by `make
clean`. The customization extension is tagged inside the compatible 32-byte
preference record; invalid IDs sanitize to conservative defaults. Fresh state
still starts at 480p/60 Hz with Efficient presentation and expensive visual or
interaction options disabled.

## Runnable through Diamond II fallback

`make run-alpha` provides the original implementation of:

- ATA-backed HexaFS persistence, cache, journal and snapshots;
- IDT/PIC/PIT interrupts, paging, heap, scheduler, Ring 3 and syscalls;
- RTL8139 networking, ICMP ping and network replay/logging;
- framebuffer/VBE mode switching and double buffering;
- users, password login and the legacy `diese`/PIMP ACL layer;
- intents, typed pipes, events, replay, boot policy and observers;
- Tetris, Tic-Tac-Toe, Hangman, Memory and Guess (Snake and Pong are now also native Prism Game Forms);
- the remaining legacy shell and package commands.

These subsystems are preserved as functionality, but they are not labeled
native v8 because their interfaces still expose path, process, UID or other
pre-philosophy semantics. They will be moved only after Form Handle and
Dimension interfaces exist for them.

## Not claimed complete

Persistent accounts are fixed state records, not yet persistent Account Forms,
and the PBKDF2 login path does not claim lockout, hardware-backed keys or a
complete modern identity policy. Mouse wheel input and GPU acceleration are not
implemented. UEFI-native boot, general persistent v8 HexaFS/Form I/O,
preemptive v8 Form execution, DHCP, IPv6, physical Wi-Fi drivers, USB
host/Bluetooth data transport, concurrent sockets and the Go execution-context
loader remain active migration work. The kernel-control layer is not FreeBSD
code or compatibility: it has no FreeBSD syscall/ABI surface, vnode/socket
filters, process rlimits, scheduler integration or interrupt-driven event
delivery. Only event-watch usage is tied to an owning subsystem today; the
other fixed ledgers provide bounded console-visible reservations only.

The 60/75/120/144 Hz choices are compositor release targets driven by the TSC;
they do not program an EDID mode or change the host monitor's physical refresh
rate. The current VSync path combines a bounded VGA status poll with Bochs VBE
page flipping. It records a timeout and continues if retrace cannot be observed,
so it is not a universal hardware tear-free guarantee or GPU acceleration.
HexaDisplay borrows explicit damage, atomic surface state and presentation
callback concepts from modern compositors, but it does not implement the
Wayland wire protocol, Unix-domain transport, shared-memory file-descriptor ABI
or compatibility with existing Wayland clients and toolkits.

HTTPS authentication is deliberately bounded. The embedded trust store is not
a general CA bundle: GlobalSign Root R1 is the default anchor, and DigiCert
Global Root G2 is selected only for `duckduckgo.com` and its subdomains. The
Browser parser limits a document to 16 KiB, while native HTTP/HTTPS fetches
retain at most the first 14 KiB of the response body. A document is further
limited to 48 nodes, 32 CSS rules, 16 scripts, 24 statements per script and 12
click handlers. It does not implement arbitrary ECMAScript, external resource
loading, general Web APIs, cookies, storage, media containers/codecs,
audio/video output or GPU acceleration. YouTube playback and a
standards-complete web engine are not claimed. ASL remains intentionally
unspecified.
