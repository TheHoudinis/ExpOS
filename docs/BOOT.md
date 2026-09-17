# Native UEFI and service boot modes

`make uefi` builds `build/esp/EFI/BOOT/BOOTX64.EFI`. This PE32+ EFI application
contains the native kernel ELF payload, validates its bounds and non-overlapping
load segments, reserves its memory at 32 MiB, obtains the firmware memory map,
and exits boot services before entering ExpOS. The native entry establishes an
owned stack, GDT and identity mappings. It does not invoke GRUB or Multiboot.
`make run` selects this native path. `make run-bios` retains BIOS as a separate
fallback while the final boot-support policy is pending.

```sh
make run-uefi      # boot with OVMF and the existing runtime state image
make uefi-check    # firmware handoff, login and shutdown
make bootmode-check # reject Guest in single-user, test service restrictions
```

Build dependencies include Clang, GNU PE-capable ld, NASM, GCC, Rust, Make,
QEMU, and OVMF. Override `OVMF_CODE` and `OVMF_VARS` for another firmware path.
The test harness waits for kernel readiness before sending serial input.
UEFI load options accept `single`, `console`, or `desktop`. With no load option,
the development image presents a chooser: `1` is multi-user desktop, `2` is
multi-user console, and `3`/`S` is single-user maintenance.

Single-user mode still requires an Operator's password. It rejects Power/Guest
accounts and does not initialize the network driver. Desktop, Browser, games
and packet-producing shell commands are unavailable. Logging out or switching
accounts cannot enable multi-user services; restart to change the service mode.
`bootmode` reports the active service policy. Multi-user means normal account
and service policy, not concurrent processes or multiple simultaneous sessions.

The handoff ABI v1 includes the map descriptor size/version and GOP metadata.
It is an implementation interface, not a finalized CFC disk or encryption
format. The allocator currently uses its reserved kernel heap; it does not yet
consume arbitrary conventional memory from the firmware map. GOP information
is recorded, but the desktop still needs the existing Bochs/QEMU framebuffer
driver. Hardware drivers, interrupts and memory isolation retain their current
limitations. This build is unsigned and does not implement Secure Boot.

The development chooser and development accounts are not the Genesis Basic
installer. Basic's automatic boot, mandatory encryption, Operator creation and
CFC selection must be connected to Genesis after its remaining design choices
are settled. No host disks are modified by these build/test targets.
