# ExpOS feature coverage

ExpOS v8.0.0-alpha.12 uses two runnable environments during migration. `make
run` boots the new x86_64 Form-native kernel. `make run-alpha` boots the original
32-bit Diamond II system with its existing storage image.

## Native v8 commands

| Area | Commands / behavior |
|---|---|
| Forms | `mkform`, `forms/list`, `inspect/fin`, exact `resolve`, `view/cat`, `write`, `append`, `head`, `copy`, `move`, `delete`, `recover`, `retire`, `activate`, guarded `reclaim`, `hexdump`, `du`, `df`, `shasum`, `which`; typed `relate`/`unrelate`/`relationships` |
| Dimensions and policy | `dimensions`, `makedim`, `policy`, `pimp`, `journal` |
| Capabilities | `grant`, `revoke`, `handles`, and `handlecheck` with requester-bound, scoped, expiring Form Handles; non-amplifying delegation; parent-linked revocation cascades; explicit Display and Input rights |
| Packages | `Ayo` boot Package Form and Go `ayo v3`; 21-package built-in Prism catalog; searchable TUI; offline/HTTPS catalogs and Ed25519 catalog signatures; SHA-256 artifact verification; raw/tar/tar.gz extraction; owned-file receipts and collision protection; atomic dependency install, uninstall, rollback and crash recovery; no package scripts or links |
| Graphics | `HexaDisplay` Service Form; 1024x768 Bochs/QEMU framebuffer with XRGB8888, ARGB8888 and RGB565 protocol formats; flat dark desktop with compact menu and taskbar, no default or pinned apps, movable/minimizable/maximizable/closeable windows, Notes and eleven surfaces; atomic geometry/commit, configure/frame/focus/key/pointer events, z-order and hit testing; VGA text-mode restoration on exit; `desktop`, `displayinfo` |
| Browser | Native `Browser` Interface Form; bounded HTML title/heading/paragraph/list/link parsing and graphical rendering; local `hexa://` navigation plus capability-gated native `http://` fetches over DNS/TCP/HTTP |
| Desktop apps | Dark graphical Terminal, Browser, Form registry, Ayo package catalog, Settings, System, Games and Notes; Up/Down terminal history; clickable application menu/taskbar; every app can close and reopen |
| Go | Shared ABI v1 call numbers and validation in Rust, tested `sdk/go/hexa` client and emulator, `GoABI` Interface Form and `goabi` diagnostics |
| Hardware | `date/clock`, `cpuinfo`, `features/kernelcaps`, `lspci`, `mem/free`, VGA and COM1 consoles, PS/2 keyboard/mouse, RTL8139 bus-master DMA, Ethernet/ARP/static IPv4/ICMP/UDP/DNS/TCP/HTTP, ANSI serial input and CPUID/control-register reporting |
| System | Graphical-or-console boot chooser and mutually switchable login, Operator/Power/Guest capability authority, runtime accounts, `users`, `useradd`, `userdel`, `passwd`, `login`, `logout`, `status`, `kstat`, `ps`, `dmesg/bootlog`, `ifconfig`, `ping`, `dns`, `fetch`, `netstat`, history, `whoami`, `reboot`, `shutdown` |
| Utilities | `calc`, `factor`, `len`, `hex`, `reverse/rev`, `tolower`, `toupper`, `rand`, `sleep`, `true`, `false` |

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
persistent v8 HexaFS block I/O, preemptive v8 Form execution, DHCP, IPv6, TLS,
physical Wi-Fi, concurrent sockets, the Go execution-context loader, and a
standards-complete web engine remain active migration work.
ASL remains intentionally unspecified.
