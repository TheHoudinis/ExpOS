# HexaOS v8 architecture rebuild

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
  including constraints, reconciliation, checkpoints and crash recovery.

The current image boots a real 64-bit kernel into an interactive command
environment. It accepts input from both the QEMU window keyboard and COM1 in
the launching terminal. It is still an architecture alpha rather than a
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

To boot interactively:

```sh
make run
```

Click the QEMU window and type `help`, or type commands directly in the terminal
that launched QEMU. The current command environment supports:

```text
help clear echo about status whoami
forms packages dimensions inspect journal policy handles relationships
mkform retire activate grant revoke pimp relate
ayo reboot shutdown
```

The alpha.4 shell also ports the practical HexaOS 7.2 command layer: Form
content (`write`, `append`, `cat`, `head`, `copy`, `move`, `delete/recover`,
`hexdump`, hashes), hardware inspection, calculator/string/math tools,
Dimensions, history, system diagnostics, and the original fun utilities.

Forms, Handles, Dimensions, and PIMP changes made in the v8 shell are currently
in-memory and reset at reboot. Persistent HexaFS block integration is the next
native storage phase.

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
| `docs/PHILOSOPHY.txt` | source architecture specification |
| `docs/ARCHITECTURE.md` | implemented design and trust boundaries |
| `docs/MIGRATION.md` | alpha subsystem port map and sequence |
| `docs/FEATURE_COVERAGE.md` | command and subsystem coverage |
| `legacy/alpha32/` | source snapshot of HexaOS 7.2 alpha |

ASL is intentionally not implemented or guessed because its established
specification was not included.
