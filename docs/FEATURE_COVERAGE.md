# HexaOS feature coverage

ExpOS v8.0.0-alpha.11 uses two runnable environments during migration. `make
run` boots the new x86_64 Form-native kernel. `make run-alpha` boots the original
32-bit Diamond II system with its existing storage image.

## Native v8 commands

| Area | Commands / behavior |
|---|---|
| Forms | `mkform`, `forms/list`, `inspect/fin`, exact `resolve`, `view/cat`, `write`, `append`, `head`, `copy`, `move`, `delete`, `recover`, `retire`, `activate`, guarded `reclaim`, `hexdump`, `du`, `df`, `shasum`, `which`; typed `relate`/`unrelate`/`relationships` |
| Dimensions and policy | `dimensions`, `makedim`, `policy`, `pimp`, `journal` |
| Capabilities | `grant`, `revoke`, `handles`, and `handlecheck` with requester-bound, scoped, expiring Form Handles; non-amplifying delegation; parent-linked revocation cascades; explicit Display and Input rights |
| Packages | `Ayo` boot Package Form and Go `ayo v2` with a 21-package built-in Prism catalog including RenderKit, MouseKit and SessionManager, interactive number/name/search TUI, offline/HTTPS catalogs, checksums/signatures, atomic dependency plans, all 11 commands, version constraints, dependency protection, reconciliation, capability merges, manifests and recovery |
| Graphics | `HexaDisplay` Service Form; 800x600 Bochs/QEMU framebuffer with XRGB8888, ARGB8888 and RGB565 protocol formats; gradients, alpha blending, rounded rectangles and lines; original black-by-default Win/KDE-style Prism shell with left launcher and bottom panel, four workspaces, master-stack tiling, floating/fullscreen modes, close/reopen, empty workspaces, window dragging, overview, Super-key bindings and ten surfaces; atomic geometry/commit, configure/frame/focus/key/pointer events, z-order and hit testing; VGA text-mode restoration on exit; `desktop`, `displayinfo` |
| Browser | Native `Browser` Interface Form; bounded local HTML title/heading/paragraph/list/link parsing and graphical rendering; `hexa://` navigation with DIESE/PIMP explanation for blocked external networking |
| Desktop apps | Black-by-default graphical Terminal, Browser, Form registry, Ayo package catalog, Settings, System Scope and Prism Arcade; Up/Down terminal history; clickable application launcher and panel; mouse window controls and game cards; every app can close and reopen |
| Go | Shared ABI v1 call numbers and validation in Rust, tested `sdk/go/hexa` client and emulator, `GoABI` Interface Form and `goabi` diagnostics |
| Hardware | `date/clock`, `cpuinfo`, `features/kernelcaps`, `lspci`, `mem/free`, VGA and COM1 consoles, PS/2 keyboard and three-byte mouse packet decoding including buttons and signed movement, ANSI serial arrow decoding, serial Esc timeout, and CPUID/control-register feature reporting |
| System | Dark keyboard-and-mouse graphical login with Operator, Power and Guest capability authority; twelve-slot runtime account registry; `users`, `useradd`, `userdel`, `passwd`, `login`, `logout`, `status`, `kstat`, `ps`, `dmesg/bootlog`, `ifconfig`, `netstat`, `mode`, bounded Up/Down shell history, `history`, `uptime`, `env`, `whoami`, `reboot`, `shutdown` |
| Utilities | `calc`, `factor`, `len`, `hex`, `reverse/rev`, `tolower`, `toupper`, `rand`, `dice`, `ascii`, `palette`, `morse`, `sleep`, `true`, `false` |
| Personality | `fortune`, `8ball`, `cowsay`, `banner`, `logo`, `matrix/cmatrix`, `russian`, `insult`, `excuse`, `compliment`, `hack` |

Native Form contents currently use fixed 512-byte in-memory records. Delete is
recovery-aware: it moves a Form to Recoverable; the core only permits final
reclamation after Dimension bindings are removed.

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

The three built-in alpha accounts and newly created runtime accounts are not
durable; password hashing and persistent account Forms are not yet claimed.
Mouse wheel input and GPU acceleration are also not implemented. UEFI-native boot,
persistent v8 HexaFS block I/O, preemptive v8 Form execution,
native v8 networking, the Go execution-context loader, and a standards-complete
web engine remain active migration work.
ASL remains intentionally unspecified.
