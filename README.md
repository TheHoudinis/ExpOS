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
  revocable Form Handles, and transactional HexaFS metadata.
- `ayo/` is an entirely Go implementation of the native Package Form manager.

The current image is an architecture bootstrap, not a finished daily-use OS.
It boots a real 64-bit kernel and exercises one complete trusted path, but does
not yet provide scheduling, UEFI-native loading, persistent block I/O, a user
environment, or ports of all alpha applications.

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
| `legacy/alpha32/` | source snapshot of HexaOS 7.2 alpha |

ASL is intentionally not implemented or guessed because its established
specification was not included.

