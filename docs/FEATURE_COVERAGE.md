# HexaOS feature coverage

HexaOS v8.0.0-alpha.5 uses two runnable environments during migration. `make
run` boots the new x86_64 Form-native kernel. `make run-alpha` boots the original
32-bit Diamond II system with its existing storage image.

## Native v8 commands

| Area | Commands / behavior |
|---|---|
| Forms | `mkform`, `forms/list`, `inspect/fin`, exact `resolve`, `view/cat`, `write`, `append`, `head`, `copy`, `move`, `delete`, `recover`, `retire`, `activate`, guarded `reclaim`, `hexdump`, `du`, `df`, `shasum`, `which`; typed `relate`/`unrelate`/`relationships` |
| Dimensions and policy | `dimensions`, `makedim`, `policy`, `pimp`, `journal` |
| Capabilities | `grant`, `revoke`, `handles`, and `handlecheck` with requester-bound, scoped, expiring Form Handles |
| Packages | `Ayo` boot Package Form, `packages`, and Go `ayo v2` with interactive TUI, offline/HTTPS catalogs, checksums/signatures, atomic dependency plans, all 11 commands, version constraints, dependency protection, reconciliation, capability merges, manifests and recovery |
| Hardware | `date/clock`, `cpuinfo`, `lspci`, `mem/free`, VGA and COM1 consoles, PS/2 and serial input |
| System | `status`, `kstat`, `ps`, `dmesg/bootlog`, `ifconfig`, `netstat`, `mode`, `history`, `uptime`, `env`, `whoami`, `reboot`, `shutdown` |
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
- Snake, Tetris, Tic-Tac-Toe, Hangman, Memory and Guess;
- the remaining legacy shell and package commands.

These subsystems are preserved as functionality, but they are not labeled
native v8 because their interfaces still expose path, process, UID or other
pre-philosophy semantics. They will be moved only after Form Handle and
Dimension interfaces exist for them.

## Not claimed complete

UEFI-native boot, persistent v8 HexaFS block I/O, preemptive v8 Form execution,
native v8 networking, and GUI Interface Forms remain active migration work.
ASL remains intentionally unspecified.
