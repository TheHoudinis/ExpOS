# Native UEFI and service boot modes

`make uefi` builds `build/esp/EFI/BOOT/BOOTX64.EFI`. This PE32+ EFI application
contains the native kernel ELF payload, validates its bounds and non-overlapping
load segments, reserves its memory at 32 MiB, obtains the firmware memory map,
and exits boot services before entering ExpOS. The native entry establishes an
owned stack, GDT and identity mappings below 4 GiB, including firmware-assigned
PCI framebuffer addresses. It does not invoke GRUB or Multiboot.
`make run` selects this native path. `make run-bios` retains BIOS as a separate
compatibility, recovery, and development fallback. Native UEFI is the approved
default for Genesis and normal boot; BIOS support is deliberately retained but
does not define the primary boot architecture.

```sh
make run-uefi      # boot with OVMF and the existing runtime state image
make uefi-check    # firmware handoff, login and shutdown
make bootmode-check # reject Guest in single-user, test service restrictions
make startup-check # capture visible UEFI/BIOS screens and exercise PS/2 login
```

## Approved Genesis boot and unlock model

An installed CFC has its own typed FIN, required nonempty name, exactly one
Primary Dimension, and exclusive ownership of all Forms, data, capabilities,
storage, keys, and checkpoints. A multi-CFC Architect installation may present
a CFC chooser, but Basic boots its single CFC directly.

Before encrypted CFC state is decoded, boot must unlock that CFC's independently
generated random storage key. The Operator password is processed with Argon2id
to derive a key-encryption key (KEK), which unwraps the storage key. Persistent
state of an encrypted CFC is authenticated-encrypted (AEAD), and recovery
exposes eight rotating checkpoints plus a protected immutable installation
baseline. Even if an
Architect installation reuses one entered password, CFC keys, key envelopes,
AEAD domains, and storage remain independent.

This is an approved target flow, not the behavior of the current development
image. Exact loader/key-envelope exchange, AEAD algorithm, Argon2id parameters,
nonce format, checkpoint format, and recovery selection UI remain implementation
work.

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
driver. The driver discovers its PCI BAR and checks the mapped address and
reported video memory before drawing; it does not assume the BIOS BAR address.
Hardware drivers, interrupts and memory isolation retain their current
limitations. This build is unsigned and does not implement Secure Boot.

The development chooser and development accounts are not the Genesis Basic
installer. The current state image is still a plaintext dual-slot CRC journal;
it does not implement the approved CFC key envelope, AEAD, eight rotating
checkpoints, protected installation baseline, or password-driven unlock flow.
Basic's automatic boot, mandatory encryption, Operator creation, and CFC
selection remain to be connected to Genesis. No host disks are modified by
these build/test targets.
