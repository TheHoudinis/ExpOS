# ExpOS feature coverage

ExpOS v8.0.0-alpha.12 uses two runnable environments during migration. `make
run` boots the new x86_64 Form-native kernel. `make run-alpha` boots the original
32-bit Diamond II system with its existing storage image. Native persistence is
tested with `make persistence-check`; live authenticated HTTPS is tested with
`make https-check`, and the live Wikipedia path with `make wikipedia-check`;
both require Internet access.

## Native v8 commands

| Area | Commands / behavior |
|---|---|
| Forms | `mkform`, `forms/list`, `inspect/fin`, exact `resolve`, `view/cat`, `write`, `append`, `head`, `copy`, `move`, `delete`, `recover`, `retire`, `activate`, guarded `reclaim`, `hexdump`, `du`, `df`, `shasum`, `which`; typed `relate`/`unrelate`/`relationships`; arbitrary Form content and graph state survive reboot through ExpFS |
| Execution | `execute <Form>` admits an active capability-authorized FIN to the Form-native scheduler; `executable` Forms run their persisted payload through bounded ExpPython while the context owns the cooperative slice and retain their exit result in CPU state; `ps` reports CFC/Dimension/FIN, runtime, state, dispatches and result; contexts own address-space/CPU descriptors, bounded Handle and event sets, and an ExpBudget |
| Dimensions and policy | `dimensions`, `makedim`, `policy`, `pimp`, `journal` |
| Recovery | `checkpoint` captures the complete current CFC database; `checkpoints` lists the eight-slot rotating ring; `restorepoint <state-id>` transactionally restores a retained snapshot and reboots |
| Capabilities | `grant`, `revoke`, `handles`, and `handlecheck` with requester-bound, scoped, expiring Form Handles; non-amplifying delegation; parent-linked revocation cascades; explicit Display and Input rights |
| Kernel controls | Form-native, runtime-local `sysctl` typed tunables; `kqueue`/`kevent` fixed-capacity signal, one-shot/periodic timer and resource-denial watches with sequenced readiness, configurable dispatch batching and optional coalescing; `expbudget` accounting (`budget` and legacy `rlimit` aliases) for event watches plus enforced Form mutation attempts and live Form content bytes, with explicit runtime-local IPC-byte and scratch-page reservations with validated soft/hard/ceiling tuples and denial counters/events; DIESE checks the revocable bootstrap Handle, reserves Configure for Operator, grants Execute mutation to Power/Operator and leaves Guest read-only; inspired by FreeBSD interface concepts without source or ABI compatibility |
| Packages | `Ayo` boot Package Form and Go `ayo v3`; categorized 25-package built-in Prism catalog with honest `x86_64`/`host` architecture metadata; `slap NAME` catalog installs and `install` compatibility spelling; searchable `glance`/TUI; offline/HTTPS catalogs and explicit Ed25519 trust state; SHA-256 artifact verification; raw/tar/tar.gz extraction; owned-file receipts and collision protection; atomic dependency install, uninstall, rollback and crash recovery; no package scripts or links |
| Graphics | `ExpDisplay` v2 Service Form with feature discovery, atomically validated multi-region damage and unchanged Form ABI v1; fresh-state 480p/60 Hz and Efficient-renderer defaults; runtime 640x480 (480p), 1280x720 (720p) and 1920x1080 (1080p) Bochs/QEMU XRGB scanout with XRGB8888, ARGB8888 and RGB565 client formats; two-page virtual framebuffer with explicit boot/login presentation, verified VBE Y-offset flips, bounded allocation-free damage clipping/coalescing and direct-front recovery when a flip is rejected; submitted/copied region and pixel, collapse and flip-failure counters; rejected modes automatically retry at 480p; selectable 60/75/120/144 Hz compositor targets, optional bounded-retrace VSync, frame/miss/timeout diagnostics; responsive flat dark desktop with compact menu and all-edge taskbar, no default or pinned apps, movable/minimizable/maximizable/closeable windows, configurable bounded off-screen travel and snapping, Notes and eleven surfaces; Form-owned pending surface state, atomic commit, coalesced presentation-bound `FrameDone`, configure/focus/key/pointer events, z-order and hit testing; bounded pure-motion input coalescing that preserves key/button edges; VGA text-mode restoration on exit; `desktop`, `displayinfo`, `displaydiag`, `safevideo/displayreset` |
| Browser | Native `Browser` Interface Form; fixed-capacity HTML title/heading/paragraph/list/link/button parsing, scrolling and graphical rendering; tag/class/id/inline CSS subset; deterministic document title/text/style/visibility and click-handler JavaScript subset; local `expos://` navigation plus capability-gated native `http://` and authenticated TLS 1.3 `https://` fetches over cached DNS/TCP/HTTP; plain address-bar text searches DuckDuckGo's canonical non-JavaScript HTML endpoint, follows at most three redirects without allowing an HTTPS-to-HTTP downgrade, and projects up to eight result titles and links; DuckDuckGo Wikipedia wrappers are unwrapped and direct Wikipedia article links use the live verified HTTPS REST summary reader; verified `www.youtube.com` HTML fetch is not video playback |
| Desktop apps | Dark graphical Terminal, Browser, Form registry, Ayo package manager, System, Games, Notes and shared Ayo Apps host; twenty installable native tools include live RTC calendar/clock, network and system status, ExpFS generation and persistent package/tool state in an `AyoApps.state` Data Form; Notes creates and revises `.txt` Data Forms; Settings has eleven pages with 603 directly wired values, including 256 accent colors and 252 wallpaper variants, while the persistent schema accepts 620 selectable states (overlapping counts); compact category/row viewport scrolling keeps long pages usable at 480p; configurable fonts, window geometry/opacity/focus, all-edge taskbar, auto-hide, status clock, rendering policy and radios; Terminal has 40-line scrollback, 24-entry history, two-eye `neofetch` and `windowreset`; every app can close and reopen |
| SDK / ABI | Frozen Form ABI v1 call/status numbers and 72-byte request / 40-byte response layout; FIN + Handle boundary for identity, IPC, display, input, time, storage, networking, browser and package services; tested Rust, Go, C and Python contracts; deterministic `expos build/run/test/package` project tool; `FormABI` Interface Form with `formabi` diagnostics (`goabi` compatibility alias); native user-mode transport remains queued |
| Hardware | `date/clock`, `timers`, `cpuinfo`, `features/kernelcaps`, `lspci`, `neofetch/sysinfo`, `mem/free`, VGA and COM1 consoles, calibrated CPUID/fallback TSC timing, PS/2 keyboard/mouse, dedicated primary-master ATA PIO state transport, RTL8139 bus-master DMA, Ethernet/ARP/DHCP/IPv4/ICMP/UDP/DNS/TCP/HTTP/TLS, PCI Wi-Fi/Bluetooth class discovery with honest driver/connection state, ANSI serial input, RDRAND detection and CPUID/control-register reporting |
| System | Graphical-or-console boot chooser and mutually switchable login; bright white-on-black command deck with cyan/green identity accents, structured startup banner, grouped help and aligned status panel; Operator/Power/Guest capability authority; persistent twelve-slot accounts; salted PBKDF2-HMAC-SHA256 password verifiers; persistent desktop/connectivity preferences with fail-safe decoding; ExpFS dual-current-slot plus eight-checkpoint CRC recovery; `users`, `useradd`, `userdel`, `passwd`, `login`, `logout`, `status`, `kstat`, `ps`, `dmesg/bootlog`, `stateinfo`, combined `diag/diagnose`, `ifconfig`, `ping`, `dns`, `fetch`, `netstat`, history, `whoami`, `reboot`, `shutdown` |
| Genesis | Native-UEFI hybrid `ExpOS-0.9-x86_64.iso`; explicitly unencrypted whole-disk Architect flow with destructive confirmation; primary/backup GPT, FAT32 ESP and `EFI/BOOT/BOOTX64.EFI`; minted CFC/Primary Dimension identities and hashed initial Operator seed; automated install, disk-only reboot, login and shutdown proof. Basic is security-gated until mandatory Argon2id + AEAD storage lands |
| Utilities | `calc`, `factor`, `len`, `hex`, `reverse/rev`, `tolower`, `toupper`, `rand`, `sleep`, `true`, `false` |

Native Form contents use fixed 512-byte records that are transactionally
persisted with Form metadata, Dimensions, relationships, PIMP network state,
revisions and allocator state in alternating CRC-verified ExpFS CFC snapshots.
`make persistence-check` proves `mkform MyNotes; write MyNotes hello` survives a
shutdown and fresh boot. Delete is
recovery-aware: it moves a Form to Recoverable; the core only permits final
reclamation after Dimension bindings are removed. Accounts and desktop settings
are typed records in the same ExpFS current-state transaction. The older two
fixed 2 KiB EXPOST03 slots remain read-only migration input; compatible records
without the timing extension load as 60 Hz with VSync enabled and move into
ExpFS on their next mutation.
`runtime/expos-state.img` is created by `make run` and is preserved by `make
clean`. The customization extension is tagged inside the compatible 32-byte
preference record; invalid IDs sanitize to conservative defaults. Fresh state
still starts at 480p/60 Hz with Efficient presentation and expensive visual or
interaction options disabled.

## Approved architecture and landed semantic foundations

The Genesis/CFC target is fixed, and its unencrypted Architect vertical slice
has landed, while the protected Basic path remains incomplete:

- every CFC has its own typed FIN, required nonempty name, and exactly one
  Primary Dimension;
- CFC ownership is exclusive: Forms, data, Dimensions, relationships,
  capabilities, identities/policy, storage extents, keys, and checkpoints are
  not shared across CFCs;
- every encrypted CFC has an independent random storage key, wrapped/unlocked
  by a key-encryption key derived from its Operator password with Argon2id;
- persistent state of an encrypted CFC uses authenticated encryption (AEAD),
  and every CFC retains eight rotating checkpoints plus a protected immutable
  installation baseline;
- native UEFI is the default Genesis/normal boot path, with BIOS retained for
  compatibility, recovery, and development;
- Architect is the official configurable-installation label; “expert” is only
  a legacy explanation; and
- ExpScope names CFC/Dimension-aware confinement, ExpSeal names monotonic
  capability reduction, and ExpBudget names per-context resource enforcement.

The `expos-core` model now provides a distinct `CfcFin`; a required name and
non-replaceable Primary Dimension; fixed-capacity CFC ownership/catalog checks;
CFC-owned typed ExpFS system records and disk snapshots; CFC-bound recovery descriptors with an
up-to-eight-entry metadata ring and the baseline descriptor held outside
rotation; immutable-ownership ExpScope reachability; a broker-lifetime ExpSeal
root-issuance cutoff with strict attenuation; and fixed-capacity context-keyed
ExpBudget accounting; and Form-native scheduler contexts carrying CFC,
Dimension, FIN, address-space, Handle, event, CPU and budget state. Cooperative
admission/dispatch and per-context budget charging are live. Genesis now builds
and verifies a bootable UEFI Architect installer; it is not yet an encrypted
Basic installer, protected-baseline restore engine,
page-table sandbox, durable seal registry, interrupt scheduler or user-mode
context switcher.

## Runnable through Diamond II fallback

`make run-alpha` provides the original implementation of:

- ATA-backed ExpFS persistence, cache, journal and snapshots;
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

Persistent accounts are typed ExpFS records, not yet executable Account Forms,
and the PBKDF2 login path does not claim lockout, hardware-backed keys or a
complete modern identity policy. Mouse wheel input and GPU acceleration are not
implemented. The PBKDF2 account verifier is not the approved Argon2id storage-key
wrapper. ExpFS current-state snapshots and eight rotating full-state
checkpoints are CFC-bound and transactional, but still lack AEAD and a
protected on-disk installation baseline. The old plaintext account/settings slots are accepted
only as migration input and are no longer the native commit destination.
The new native UEFI path, retained BIOS fallback, and Operator-only single-user
mode are QEMU-tested;
preemptive v8 Form execution, real address-space switching, IPv6,
physical Wi-Fi drivers, USB
host/Bluetooth data transport, concurrent sockets and the Go execution-context
loader remain active migration work. The kernel-control layer is not FreeBSD
code or compatibility: it has no FreeBSD syscall/ABI surface, vnode/socket
filters, process rlimits, scheduler integration or interrupt-driven event
delivery. Event watches, Form mutation attempts and live content bytes are tied to
owning subsystems; IPC-byte and scratch-page ledgers remain explicit
reservations. Content growth is validated before publication and never
silently truncated.

The 60/75/120/144 Hz choices are compositor release targets driven by the TSC;
they do not program an EDID mode or change the host monitor's physical refresh
rate. The current VSync path combines a bounded VGA status poll with Bochs VBE
page flipping. It records a timeout and continues if retrace cannot be observed,
so it is not a universal hardware tear-free guarantee or GPU acceleration.
ExpDisplay v2 borrows explicit multi-region damage, atomic surface state,
feature discovery and presentation callback concepts from modern compositors,
but it does not implement the
Wayland wire protocol, Unix-domain transport, shared-memory file-descriptor ABI
or compatibility with existing Wayland clients and toolkits.

HTTPS authentication is deliberately bounded. The embedded trust store is not
a general CA bundle: GlobalSign Root R1 is the default anchor, DigiCert Global
Root G2 is selected only for `duckduckgo.com`, and ISRG Root X1 only for
`wikipedia.org` and their subdomains. The
Browser parser limits a document to 16 KiB, while native HTTP/HTTPS fetches
retain at most the first 14 KiB of the response body. A document is further
limited to 48 nodes, 32 CSS rules, 16 scripts, 24 statements per script and 12
click handlers. It does not implement arbitrary ECMAScript, external resource
loading, general Web APIs, cookies, storage, media containers/codecs,
audio/video output or GPU acceleration. YouTube playback and a
standards-complete web engine are not claimed. ASL's high-level role is defined,
but its exact interfaces, bootstrap ordering, and cross-architecture
implementation remain unspecified and unimplemented.

## Python and native firmware

ExpPython embeds MicroPython 1.26 with a private bounded heap and non-catchable
execution/output budget aborts; Python runs inline or from a shell Form. The
Python host SDK has a tested ABI codec and surface emulator, without a live
kernel transport. See `PYTHON.md` for language, authority and runtime limits.
The UEFI loader exits firmware boot services and passes the validated memory
map; it retains the current kernel hardware limitations described in `BOOT.md`.
