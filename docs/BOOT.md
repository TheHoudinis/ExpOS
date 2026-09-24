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
make genesis-iso   # native UEFI installer at build/ExpOS-0.9-x86_64.iso
make genesis-check # install to a blank disk, boot it, log in, shut down
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

The encrypted portion remains an approved target flow, not behavior of the
current development image. Exact loader/key-envelope exchange, AEAD algorithm, Argon2id parameters,
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

Genesis now has a separate native-UEFI installation build. Its first supported
vertical slice is Architect, whole-disk and explicitly unencrypted. It writes
primary/backup GPT structures, a FAT32 EFI System Partition, the runtime at
`EFI/BOOT/BOOTX64.EFI`, and a checksummed manifest with new CFC and Primary
Dimension identities plus a salted Operator password verifier. On first
installed boot those identities drive core bootstrap and the Operator becomes
an ExpFS account record. `make genesis-check` proves ISO install, ISO-free disk
boot, Operator login, and shutdown.

Basic remains gated rather than silently weakened. The current state database
is still plaintext and does not implement the approved CFC key envelope, AEAD,
protected installation baseline, or password-driven unlock flow. The Architect
slice currently sees only the ATA primary master and does not enumerate model
or serial identities, so it is not yet approved for physical-disk use. The
automated check modifies only its generated blank image.
