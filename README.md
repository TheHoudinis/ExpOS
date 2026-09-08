# ExpOS Prism v8 architecture rebuild

This repository is the unified HexaOS rebuild derived from the living
philosophy specification. It combines the useful foundations of both supplied
projects without making either legacy architecture the new system model:

- expodOS contributes the x86_64 Multiboot2-to-long-mode bootstrap, serial
  console, VGA console, volatile memory access and spin synchronization.
- HexaOS 7.2 Diamond II is preserved under `legacy/alpha32/` as a buildable port
  source for its drivers, scheduler, networking, persistence, shell and tools.
- the new Rust `hexa-core` implements the Form-native semantics that the old
  alpha approximated: FIN identity, Dimension bindings, Dimension-aware
  retirement, typed PIMP policy, explainable DIESE resolution, scoped and
  revocable requester-bound Form Handles, typed relationships, safe Form
  reclamation, and transactional HexaFS metadata.
- `ayo/` is an entirely Go implementation of the native Package Form manager,
  including an interactive catalog TUI, HTTPS registries, dependency plans,
  constraints, reconciliation, checkpoints and crash recovery.
- `HexaDisplay` is a native 800x600 graphical server with Form-owned surfaces,
  buffers, damage, atomic commits, focus, z-order, pointer hit testing and input
  events.
- the HexaDisplay desktop is the new Prism environment: an original dark
  Win/KDE-style capability compositor with a left application launcher and
  bottom panel, plus four workspaces, master-stack tiling, floating/fullscreen
  windows, window dragging and overview. Its graphical Terminal, Browser, Form
  registry, Ayo package center, Settings, live System Scope, and native arcade
  containing playable Snake and Pong are fully clickable. Each application owns its
  surface and receives a narrow delegated Display/Input Handle.
- a graphical Session Manager signs users in as Operator, Power or Guest and
  applies that authority to shell capability decisions. Its twelve-slot account
  registry supports creating/deleting users and changing passwords at runtime.
  PS/2 mouse packets are decoded in the kernel and routed only to the Form under
  the pointer.
- `Browser` is a native graphical Interface Form with a bounded local HTML
  parser, document renderer, links and policy-explained network restrictions.
- `sdk/go/` defines and tests the capability-gated HexaOS Go ABI v1.

The current image boots a real 64-bit kernel into a graphical login followed by
an interactive command environment. It accepts a PS/2 mouse and keyboard from
the QEMU window plus keyboard input from COM1 in the launching terminal. It is
still an architecture alpha rather than a
finished daily-use OS: scheduling, UEFI-native loading, persistent block I/O,
and ports of the alpha applications remain in development.

## Build and verify

Requirements: Rust with `x86_64-unknown-none`, Cargo, NASM, GNU binutils,
GRUB i386-pc modules, xorriso, QEMU x86_64, Go, and Make.

```sh
make test       # host tests for Form-native core
make iso        # build build/hexaos.iso
make check      # boot it headlessly and require HEXA_BOOT_OK
make ayo        # test and build the Go package manager
make all        # all of the above
```

Open the package catalog and select a Form by number or name:

```sh
./ayo/bin/ayo --authority operator
```

`Browser` and `Snake`, for example, resolve and install their complete
dependency plans in one atomic transaction. The built-in 21-package catalog
also includes PrismDE, RenderKit, MouseKit, SessionManager, InputKit, DeveloperKit,
TextLab, VirtioBlock, and AudioKit. A remote checksummed catalog can be selected with
`--registry https://.../catalog.json`; `--registry-key` additionally requires
an Ed25519 signature.

To boot interactively:

```sh
make run
```

Click the QEMU window and sign in with one of the alpha accounts below. Press
Enter to move from user to password and Enter again to sign in. You can also
click either field and the Sign In button, or type through the terminal that
launched QEMU.

| User | Password | Authority |
|---|---|---|
| `operator` | `expos` | Full Operator controls |
| `developer` | `prism` | Power user; destructive policy controls denied |
| `guest` | `guest` | Read-only session |

These are built-in demonstration credentials, not production authentication.
An Operator can manage additional runtime users with:

```text
useradd <name> <operator|power|guest> <password>
passwd <name> <new-password>
userdel <name>
```

Password-bearing commands are excluded from command history. Accounts remain
in memory until native HexaFS account persistence is implemented. Use `users`,
`login`, `logout`, or `whoami` to inspect or change the current session. The
command environment supports:

```text
help clear echo about status whoami users useradd userdel passwd login logout
forms packages dimensions inspect journal policy handles relationships
mkform retire activate reclaim resolve grant revoke handlecheck pimp
relate unrelate
desktop browser games displayinfo goabi
features kernelcaps ayo reboot shutdown
```

The alpha.11 shell also ports the practical HexaOS 7.2 command layer: Form
content (`write`, `append`, `cat`, `head`, `copy`, `move`, `delete/recover`,
`hexdump`, hashes), hardware inspection, calculator/string/math tools,
Dimensions, system diagnostics, and the original fun utilities. Both the text
shell and graphical Terminal keep bounded command history; Up and Down recall
older and newer entries through either the QEMU keyboard or ANSI serial input.

Forms, Handles, Dimensions, and PIMP changes made in the v8 shell are currently
in-memory and reset at reboot. Persistent HexaFS block integration is the next
native storage phase.

Type `browser` or `desktop` in the booted kernel to enter HexaDisplay.
`desktop` opens the graphical Terminal while `browser` focuses the Browser.
The mouse opens the application launcher, selects panel and launcher apps,
focuses windows, drags title bars, activates minimize/maximize/close controls,
and selects Arcade games. Every app can close, an empty workspace stays empty,
and selecting a closed app launches it again. Panel apps return to their
existing workspace instead of being moved.
The compositor bindings are `Super+Space` launcher, `Super+Enter` Terminal,
`Super+B` Browser, `Super+1/2/3/4` workspaces, `Super+arrows` focus/workspace
movement, `Super+F` fullscreen, `Super+V` floating, `Super+O` overview,
`Super+Q` close and `Super+Tab` cycle. Backtick, `B/T/F/P/S/I/G`, `Tab`, and
arrow-key fallbacks remain available. `Esc` returns to the text environment.
The graphical Terminal also accepts `ws1/ws2/ws3/ws4`, `games`, `float`,
`full`, `overview`, `help`, and `exit`. The shell command `games` opens Prism
Arcade directly; choose `1` for Snake or `2` for Pong, steer with the arrows,
pause with Space, reset with `R`, and return to the arcade with `M`.

Rendering remains software-based rather than GPU accelerated, but HexaDisplay
now negotiates XRGB8888, ARGB8888 and RGB565 buffers. The native renderer adds
vertical gradients, alpha-blended rectangles, rounded rectangles and line
drawing; solid fills use clipped row writes, pointer motion uses cursor
save-under, and active games repaint only their window. These changes keep the
black-by-default Prism shell responsive under QEMU at 800x600.

The complete 32-bit Diamond II environment remains runnable while its deeper
drivers and games are ported:

```sh
make run-alpha
```

See [feature coverage](docs/FEATURE_COVERAGE.md) for the native/fallback split.

To verify the preserved alpha source still builds:

```sh
make legacy-alpha-check
```

## Repository map

| Area | Purpose |
|---|---|
| `boot/`, `kernel/` | transitional Multiboot2 loader and x86_64 kernel |
| `crates/hexa-core/` | platform-independent trusted HexaOS semantics |
| `ayo/` | Go Package Form manager and development HexaFS bridge |
| `sdk/go/` | Go ABI v1 client SDK and deterministic kernel emulator |
| `docs/PHILOSOPHY.txt` | source architecture specification |
| `docs/ARCHITECTURE.md` | implemented design and trust boundaries |
| `docs/MIGRATION.md` | alpha subsystem port map and sequence |
| `docs/FEATURE_COVERAGE.md` | command and subsystem coverage |
| `legacy/alpha32/` | source snapshot of HexaOS 7.2 alpha |

ASL is intentionally not implemented or guessed because its established
specification was not included.
