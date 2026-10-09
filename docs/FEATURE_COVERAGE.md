# ExpOS feature coverage

ExpOS v9.2-dev “Mole” uses two runnable environments during migration. `make
run` boots the new x86_64 Form-native kernel. `make run-alpha` boots the original
32-bit Diamond II system with its existing storage image. Native persistence is
tested with `make persistence-check`; live authenticated HTTPS is tested with
`make https-check`, and the live Wikipedia path with `make wikipedia-check`;
both require Internet access.

## Native v9 commands

| Area | Commands / behavior |
|---|---|
| Forms | `mkform`, `forms/list`, `inspect/fin`, exact `resolve`, `view/cat`, `write`, `append`, `head`, `copy`, `move`, `delete`, `recover`, `retire`, `activate`, guarded `reclaim`, `hexdump`, `du`, `df`, `shasum`, `which`; typed `relate`/`unrelate`/`relationships`; arbitrary Form content and graph state survive reboot through ExpFS |
| Execution | `execute <Form>` admits an active capability-authorized FIN; executable capsules get distinct CR3 roots, supervisor-only kernel mappings, read-only executable user code and NX user data/stack pages, with CPL3 entry via `iretq`; boot enables CR0.WP, NXE and supported SMEP on every CPU; a DPL3 `0x80` gate accepts only the Form ABI shared page and requester-bound Handles; PIT/local-APIC timer interrupts save complete registers, preempt every four ticks and charge per-context `CpuTicks`; ACPI MADT startup brings up to eight CPUs online with per-CPU GDT/TSS/stacks and independently reservable AP jobs; `execute-parallel <Form-A> <Form-B>` dispatches two prepared Forms before joining either and requires distinct AP slots; `native:spin` is forcibly stopped at 256 ticks; `ps` reports CFC/Dimension/FIN, runtime, state, slices, preemptions, ticks and result |
| Dimensions and policy | `dimensions`, `makedim`, `policy`, `pimp`, `journal` |
| Recovery | `checkpoint` captures the complete current CFC database; `checkpoints` lists the eight-slot rotating ring; `restorepoint <state-id>` transactionally restores a retained snapshot and reboots |
| Capabilities | `grant`, `revoke`, `handles`, and `handlecheck` with requester-bound, scoped, expiring Form Handles; non-amplifying delegation; parent-linked revocation cascades; explicit Display and Input rights |
| Kernel controls | Form-native, runtime-local `sysctl` typed tunables; `kqueue`/`kevent` fixed-capacity signal, one-shot/periodic timer and resource-denial watches with sequenced readiness, configurable dispatch batching and optional coalescing; `expbudget` accounting (`budget` and legacy `rlimit` aliases) for event watches plus enforced Form mutation attempts and live Form content bytes, with explicit runtime-local IPC-byte and scratch-page reservations with validated soft/hard/ceiling tuples and denial counters/events; DIESE checks the revocable bootstrap Handle, reserves Configure for Operator, grants Execute mutation to Power/Operator and leaves Guest read-only; inspired by FreeBSD interface concepts without source or ABI compatibility |
| Packages | `Ayo` boot Package Form and Go `ayo v3`; categorized 57-package built-in Prism catalog with honest `x86_64`/`host` architecture metadata; `slap NAME` catalog installs and `install` compatibility spelling; the native shell and graphical Terminal also install/remove the thirty desktop apps through durable `AyoApps.state`; searchable `glance`/TUI; offline catalogs and HTTPS catalogs that fail closed without a pinned Ed25519 key; explicit trust reporting; SHA-256 artifact verification; raw/tar/tar.gz extraction; owned-file receipts and collision protection; atomic dependency install, uninstall, rollback and crash recovery; no package scripts or links |
| Graphics | `ExpDisplay Portal` v3 Service Form with feature discovery, atomically validated multi-region damage and unchanged Form ABI v1; validated UEFI GOP output using firmware geometry/stride/RGB order plus post-handoff Bochs/QEMU VBE reprogramming at 640x480, 1280x720 and 1920x1080; GOP composition targets a 16 MiB shadow scanout with adaptive full-damage promotion, periodic reconciliation, readback sampling and automatic full recovery, while Bochs uses two-page VBE output where available; XRGB8888, ARGB8888 and RGB565 client formats; submitted/copied region and pixel, collapse, promotion, copy-cost, readback/recovery and flip-failure counters; selectable 60/75/120/144 Hz compositor targets with bounded pacing recovery and VSync diagnostics; responsive flat dark desktop with four menu layouts and an all-edge taskbar, no default or pinned apps, movable/minimizable/maximizable/closeable windows, recoverable off-screen travel and snapping, Notes and eleven surfaces; resolution changes persist and restart ExpDisplay without entering the shell, while command transitions reinitialize PS/2 and console scanout; Form-owned pending surface state, atomic commit, coalesced presentation-bound `FrameDone`, configure/focus/key/pointer events, z-order and hit testing; bounded pure-motion event coalescing plus critical-event slot recovery that preserves key/button edges; live `displaydebug` overlay and `displayrepair`; GOP-backed UEFI login/console and VGA text restoration on BIOS exit; `desktop`, `displayinfo`, `displaydiag`, `safevideo/displayreset` |
| Browser | Native `Browser` Interface Form; heap-backed bounded page container with transactional document replacement, last-valid-page retention, rejection counters and reduced compositor-stack pressure; six tab sessions with independent twelve-entry histories and scroll positions, eight in-session bookmarks, find-in-page with match highlighting/counter, Material-inspired rounded tonal start/capability/loading cards, Chromium-like tab/omnibox/security chrome, and explicit `TLS`/`WEB`/`FORM` status; fixed-capacity semantic HTML text/link/button/audio/video parsing with common entity decoding; tag/class/id/inline CSS including radius, max-width and line-height; deterministic document title/text/style/visibility and click-handler JavaScript subset; twelve-entry external-resource manifest with four automatic non-video fetches, a two-fetch script bridge, a 64x64 BMP cache and PCM WAV media; origin-scoped cookies and local/session storage with durable local state; capability-gated native `http://` and authenticated TLS 1.3 `https://` with two bounded transport attempts and six-second idle I/O deadlines; allocation-free Wikipedia REST search, a live Wikipedia reader, and immediate bounded YouTube compatibility handling; malformed, unreachable or over-budget pages return an in-place error without replacing the last valid document; this is not general Web API, Chromium/Blink/V8, compressed-media/video or GPU-raster compatibility |
| Desktop apps | Dark graphical Terminal, Browser, Form registry, Ayo package manager, System, Games, Notes and an internal native-app host; installed Ayo apps and a Power & session surface appear directly in the main menu; the power surface provides password-backed lock, display sleep, interrupt-driven CPU/display suspend, restart and shutdown without claiming ACPI S3; thirty optional native tools include a precedence-correct calculator, persistent task list, live clock/calendar, converters, editors, real focus/countdown/stopwatch state, developer/network utilities and a live Open-Meteo Weather app with geocoding, TLS current-condition fetches, animated condition UI, modern location/status cards and a taskbar temperature widget; package/tool state persists in an `AyoApps.state` Data Form and none of these tools is installed by a fresh Minimal system; Notes creates and revises `.txt` Data Forms; Settings has eighteen pages with 1,296 directly wired values plus versioned `DesktopUI.state` and `BrowserData.state` extensions; eight coordinated profiles; dedicated Audio, Accessibility, Terminal, Language and Date & time surfaces; persistent volume/mute, reduced transparency, focus ring and half/third/quarter tiling; edge, floating and accent-rail taskbar styles, three icon scales, optional running indicators and six optional widgets; 256 theme palettes, 256 accent colors, 252 wallpaper variants, 256 backdrop tones and persistent menu controls, while the base persistent schema accepts 1,140 selectable states (overlapping counts); compact category/row viewport scrolling keeps long pages usable at 480p; Terminal includes real `audio`/`audiotest`/`audiostop` controls alongside ExpFS, Ayo, CPU, display and window tools; every app can close and reopen |
| Audio | ASL-owned Intel/QEMU-compatible AC'97 PCM-out backend; 48 kHz stereo signed-16 bus-master DMA; persistent 0-100 volume and mute; Settings test/stop controls and graphical Terminal diagnostics; bounded RIFF/WAVE integer PCM validation, mono/stereo conversion and nearest-neighbor resampling from 8-192 kHz and 8/16/24/32-bit sources; Browser `<audio>` playback for one bounded WAV cache; no mixer, capture, HDA/USB backend, compressed codec or video path |
| Weather hardening | Open-Meteo current conditions plus five daily high/low/precipitation cards, Celsius/Fahrenheit switching, persisted forecasts and coordinates, cached repeat refreshes, exact-host HTTPS allowlisting, bounded city/coordinate/observation validation, three-second pre-network throttling and a Privacy data purge action |
| Runtime efficiency | Weather animation is capped at four damage-only frames per second; repeat forecasts skip redundant geocoding; current conditions and five daily forecasts share one bounded provider request and one damaged-window render path |
| SDK / ABI | Frozen Form ABI v1 72-byte request / 40-byte response layout plus additive `EXECUTION_YIELD`/`EXECUTION_EXIT`; FIN + Handle boundary for identity, IPC, display, input, time, storage, networking, browser and package services; native CFC/requester/target/Dimension/operation gate used by Ayo and CPL3 Forms; Rust, Go, C, and Python contracts; deterministic `expos build/run/test/package` tool; `LOG` has a validated context-page offset/length grant while remaining buffer-bearing native calls stay unsupported pending their grant formats |
| Hardware | `date/clock`, `timers`, `cpuinfo`, `features/kernelcaps`, `lspci`, `asl`, `neofetch/sysinfo`, `mem/free`; UEFI GOP and Bochs/VGA/COM1 consoles; xHCI boot keyboard/mouse plus PS/2/ANSI serial input; NVMe submission/completion queues, AHCI DMA and ATA-PIO fallback behind one state transport; ASL v1 bounded PCI/firmware inventory with exclusive ExpDisplay/ExpStorage/ExpUSB/ExpNetwork/ExpAudio claims; AC'97 PCM-out DMA; ACPI RSDP/XSDT/RSDT/MADT CPU discovery, INIT/SIPI application-processor startup and local-APIC scheduler timers; MSI/MSI-X discovery/vector/message preparation without armed device handlers; RTL8139 DMA and Ethernet/ARP/DHCP/IPv4/ICMP/UDP/DNS/TCP/HTTP/TLS; RDRAND and CPUID/control-register reporting |
| System | Graphical-or-console boot chooser and mutually switchable login; bright white-on-black command deck with cyan/green identity accents, structured startup banner, grouped help and aligned system status panel; Operator/Power/Guest capability authority; persistent twelve-slot accounts; new accounts use salted 100,000-round PBKDF2-HMAC-SHA256 login verifiers, constant-time comparison and 250 ms to 2 s failed-login backoff; persistent desktop/connectivity preferences with fail-safe decoding; ExpFS dual-current-slot plus eight checkpoints using XChaCha20-Poly1305 for encrypted CFCs and CRC for unencrypted development/Architect state; `users`, `useradd`, `userdel`, `passwd`, `login`, `logout`, `status`, `kstat`, `ps`, `dmesg/bootlog`, `stateinfo`, combined `diag/diagnose`, `ifconfig`, `ping`, `dns`, `fetch`, `netstat`, history, `whoami`, `reboot`, `shutdown` |
| Genesis | Genesis v2 native-UEFI hybrid `ExpOS-v9-x86_64.iso`; compact styled firmware-GOP console with hardware compatibility overview, eight progress stages and stage-specific errors; locale persisted in manifest v4; Easy (Argon2id 64 MiB/3-pass), Paranoid (6-pass, NX+SMEP required, network/Browser/ExpPython omitted), and Architect-only unencrypted profiles; Minimal/Essentials/Everything presets plus Custom Browser, ExpPython and per-app selection, selectable SDK contracts and RTL8139 driver, up to five secondary Dimensions and two Power/Guest users; redundant required plan sectors authenticated by encrypted key-envelope AAD and seeded once into ExpFS; explicit `ERASE` install gate; removable-media recovery with read-only diagnosis, verified metadata repair, authenticated baseline restore and double-confirmed annihilation; warm-reboot 8042 keyboard reinitialization; primary/backup GPT, FAT32 ESP, `EFI/BOOT/BOOTX64.EFI`, redundant manifests/plans and complete post-write read-back verification; safe QEMU runner and automated Basic/Architect install, scanout/input, baseline, disk-only reboot, login and shutdown proof; physical disk identity selection remains absent |
| Utilities | `calc`, `factor`, `len`, `hex`, `reverse/rev`, `tolower`, `toupper`, `rand`, `sleep`, `true`, `false` |

Native Form contents use fixed 512-byte records that are transactionally
persisted with Form metadata, Dimensions, relationships, PIMP network state,
revisions and allocator state in alternating verified ExpFS CFC snapshots.
Encrypted CFCs use AEAD; unencrypted state uses CRC compatibility records.
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
preference record; bounded selectors sanitize to conservative defaults, while
every one-byte theme, accent and backdrop palette ID is valid. Fresh state
still starts at 480p/60 Hz with Efficient presentation and expensive visual or
interaction options disabled.

## Approved architecture and landed semantic foundations

The Genesis/CFC target is fixed. Architect remains the unencrypted development
path, while Basic now requires the landed encrypted-storage path:

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
Dimension, FIN, address-space, Handle, event, CPU and budget state. Native
executable contexts now switch CR3/registers, enter Ring 3, cross a Form ABI
gate and are preempted/accounted from PIT interrupts. Genesis now builds
a verified UEFI installer with encrypted Easy/Paranoid Basic and unencrypted
Architect choices; the automated disk-install reboot check covers Easy and
Architect, redundant manifests/plans and the protected baseline. Recovery is a
keyboard-driven framebuffer flow, not yet a mouse-first widget UI. A general
physical-memory manager, general ELF loader and durable seal registry remain
open. SMP now starts firmware-described x86_64 application processors and
`execute-parallel` exposes bounded two-Form foreground concurrency over distinct
AP jobs; a general user-facing background job controller remains open.

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
native v9 because their interfaces still expose path, process, UID or other
pre-philosophy semantics. They will be moved only after Form Handle and
Dimension interfaces exist for them.

## Not claimed complete

Persistent accounts are typed ExpFS records, not yet executable Account Forms.
The PBKDF2 login path has bounded exponential backoff but does not claim durable
lockout, hardware-backed keys or a complete modern identity policy. Mouse wheel input and GPU acceleration are not
implemented. The PBKDF2 account verifier is separate from the implemented
Argon2id storage-key wrapper. ExpFS current-state snapshots and eight rotating
full-state checkpoints are CFC-bound, transactional and AEAD-protected for
encrypted CFCs, and include a protected on-disk installation baseline outside
checkpoint rotation. The old plaintext account/settings slots are accepted
only as migration input and are no longer the native commit destination.
The new native UEFI path, retained BIOS fallback, Operator-only single-user
mode, CPL3 Form execution, per-Form CR3 switching, timer quota enforcement and
two-Form SMP dispatch-before-join are QEMU-tested. General binary loading and
background job management,
IPv6,
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
ExpDisplay Portal v3 borrows explicit multi-region damage, atomic surface state,
feature discovery, bounded event-pressure handling and presentation callback
concepts from modern compositors, but it does not implement the
Wayland wire protocol, Unix-domain transport, shared-memory file-descriptor ABI
or compatibility with existing Wayland clients and toolkits.

HTTPS authentication is deliberately bounded. The embedded trust store is not
a general CA bundle: GlobalSign Root R1 is the default anchor, DigiCert Global
Root G2 is selected only for `duckduckgo.com`, and ISRG Root X1 only for
`wikipedia.org` and their subdomains. The
Browser parser limits a document to 16 KiB, while native HTTP/HTTPS fetches
retain at most the first 14 KiB of the response body. A document is further
limited to 48 nodes, 32 CSS rules, 16 scripts, 24 statements per script, 12
click handlers, 12 external resources and eight Web-bridge requests. External
loading is limited to CSS, the deterministic script subset, one 64x64 BMP cache
and one PCM WAV cache. Fetch is GET-only; cookies and local/session storage are
origin-partitioned, with only local storage persisted. It does not implement
arbitrary ECMAScript, general Web APIs, compressed media, video output or GPU
acceleration. YouTube playback and a
standards-complete web engine are not claimed. ASL v1 now defines and implements
the x86_64 inventory, exclusive ownership and interrupt-preparation boundary;
Handle-backed driver Forms and a non-x86 backend are not implemented.

## Python and native firmware

ExpPython embeds MicroPython 1.26 with a private bounded heap and non-catchable
execution/output budget aborts; Python runs inline or from a shell Form. The
Python host SDK has a tested ABI codec and surface emulator, without a live
kernel transport. See `PYTHON.md` for language, authority and runtime limits.
The UEFI loader exits firmware boot services and passes the validated memory
map; it retains the current kernel hardware limitations described in `BOOT.md`.
