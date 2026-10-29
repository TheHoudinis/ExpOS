# ExpOS native security model

This document describes implemented controls and remaining trust boundaries. It
does not treat Rust, capability names, checksums, or a successful unit test as
proof that the kernel cannot be compromised.

## Boot profiles and attack surface

Genesis records an immutable-for-the-boot service policy. Minimal installs omit
Browser and ExpPython. Essentials enables Browser but keeps ExpPython absent.
Paranoid uses the six-pass Argon2id storage-key profile, requires CPU NX and
SMEP, and disables the network driver, Browser and ExpPython before any of them
receive a Handle or initialize hardware. A shell command cannot turn a service
back on during that boot.

Encrypted manifest v4 installations authenticate the complete Genesis plan as
associated data on the password-wrapped CFC storage key. Offline changes to its
users, Dimensions, packages, SDK flags, drivers, encryption profile or optional
runtime components therefore make key unwrap fail. Architect's explicit
unencrypted development mode has checksums and redundant copies, but it does
not provide this authenticity guarantee.

## Threat controls

| Threat | Implemented control | Important remaining limit |
|---|---|---|
| Kernel memory corruption | Rust for normal parsing/state paths; CR0.WP; NXE; SMEP; CPU-supported SMAP; section-aligned RX text, R/NX rodata and RW/NX data/heap/stack/MMIO; unmapped null page; saved-CS fault attribution; read-only executable Form code; NX Form data/stack; fixed-capacity protocol records | Hardware drivers and FFI still require `unsafe`; the monolithic kernel is not fault-isolated; no IOMMU; NX/SMEP/SMAP depend on CPU support |
| Malformed network/TLS input | The RTL8139 polling driver, private DMA buffers, Ethernet, DHCP, DNS, ICMP, generic UDP, plain TCP/HTTP and authenticated streaming TCP/TLS execute in one sealed Ring 3 worker; HTTP response parsing uses another worker; replies are revalidated; the worker receives only a private writable/NX DMA arena and an exact register/value portal; DMA addresses are kernel-substituted; bus mastering is revoked before arena release; response/document sizes are fixed; network initialization can be omitted | PCI discovery, page pin/translation, the register portal and revocation remain Ring 0; without an IOMMU a malicious or faulty physical device is not confined to the programmed aperture; deterministic mutation campaigns cover network decoders, but coverage-guided/sanitizer fuzzing is not complete |
| MicroPython escape | Real VM runs in a private Ring 3 service with W^X, read-only input, sealed Execute/reply Handles, fault containment and hardware watchdog; minimal ROM, no filesystem/network imports, fresh 256 KiB heap, source/step/output bounds and install-time omission | The loader, ABI bridge, scheduler and kernel remain trusted; no general user-mode service filesystem/network API yet; worker requires NX |
| Capability forgery/leak | CFC/requester/target/Dimension/operation/expiry validation; parent-linked revocation; restored children must re-prove strict attenuation; IDs skip collisions; ExpSeal closes new ambient root issuance | Kernel compromise can bypass the broker; durable general seal/context isolation remains incomplete |
| Package supply chain | `ayo registry install` downloads a same-origin HTTPS manifest/artifact pair, requires a caller-pinned Ed25519 key, verifies signed length and SHA-256, and feeds the bounded EXF1 image through private staging and one atomic ExpFS publication; no package scripts or links | The compiled seed catalog is part of the trusted image; no production public endpoint/key or transparency log is shipped |
| Music package supply chain | `ayo music install` accepts only the two compiled track identities, fetches bounded HTTP/HTTPS byte ranges with retries, verifies the pinned exact length and SHA-256 during transfer and again from disk, then atomically publishes a redundant catalog header; interrupted or mismatched downloads stay invisible; the MP3 binaries are absent from the trusted image | Plain HTTP hides no metadata and depends on the pinned digest for integrity; the raw store is VM-only, limited to 8 MiB per track, and deliberately refuses GPT/Genesis disks |
| Credential guessing | New account records use 100,000-round PBKDF2-HMAC-SHA256; failed logins back off from 250 ms to 2 s; encrypted CFC unlock powers down after five failures; comparisons are constant-time | Login backoff is not durable across reboot; no TPM, FIDO, hardware-backed key or remote attestation |

## Recovery boundary

Booting Genesis from removable media against a disk with ExpOS metadata opens a
recovery menu. Troubleshooting is read-only. Metadata repair copies only a sole
valid redundant manifest/plan and verifies both copies after the flush. Baseline
restore first performs the normal encrypted-CFC unlock. Annihilation requires
the exact strings `ANNIHILATE` and `ERASE`, wipes ExpOS boot/metadata/ExpFS
regions, and verifies representative sectors; it is not a forensic whole-disk
erase.

## Claims deliberately not made

- Browser document/UI state and device drivers are not separate Ring 3 services;
  ExpPython, HTTP parsing and network/TLS protocol state are.
- Kernel image section W^X is active after early boot on NX-capable CPUs;
  bootstrap initially uses writable executable huge pages. Unsupported CPUs
  retain a diagnostic boot path with `wx=false`. The low identity map still
  covers RAM and MMIO, and kernel code can modify page tables.
- EXF1 container-v2 Form pages are leased from the firmware-derived physical
  frame allocator. Code is populated while RW/NX, then its supervisor identity
  alias is sealed R/NX while the Form alias is RX. Reclaim restores RW/NX.
  Cross-CPU TLB shootdown and IOMMU/DMA isolation remain open; kernel page-table
  code is still trusted.
- EXF1 images require NX and receive a bounded CPL3 address space; GS is
  switched on entry/exit, unowned fast syscall entries are disabled, and
  floating-point/SIMD instructions fault until owned state saving exists.
  NMI/IST stacks and demand-grown heaps are still missing; v2 receives a fixed,
  eagerly committed private 64 KiB RW/NX heap.
- Secure Boot, measured boot, TPM sealing, IOMMU DMA isolation and signed kernel
  updates are not implemented.
- CRC protects unencrypted Architect records from accidental damage, not a
  malicious disk editor.
- Static tests do not replace hostile packet/parser fuzzing or physical-machine
  validation.

## v9.4 protection validation

`make isolation-check` builds a separate disposable image with the
`isolation-self-test` feature. Its narrowly matched exception fixups do not
exist in normal runtime/Genesis builds. QEMU must actually fault on writes to
kernel text/rodata, execution from rodata/data, and SMAP-blocked reads of user
pages. A user-set AC flag must be cleared at kernel entry. Additional Forms
try writes to code, execution from data, supervisor reads, division by zero and
invalid instructions; a survivor Form must still run. A separate boot injects
a supervisor null read while a Form is active and must halt as a kernel fault.
The suite also boots with NX/SMEP/SMAP disabled and checks the diagnostic path.

ABI requests and replies use the admitted Form slot's supervisor data alias,
with exact user virtual addresses and bounded lengths validated first. No
untrusted user pointer is dereferenced and SMAP stays enabled; no STAC window
is needed for the current fixed-page ABI. Future arbitrary user buffers need
an explicit validated copy contract.

The AP trampoline is RW/NX during copying and RX before SIPI. Its mailbox is
on a separate RW/NX page; APs enable NXE before using the shared root. Normal
Form exceptions carry saved CPL, RIP, error code and CR2. Supervisor faults halt
instead of unwinding to a Form's saved kernel stack. Double-fault/NMI/machine-
check IST stacks and a general recoverable kernel-fault model remain open.

`make parser-fuzz` runs host-only seeded mutation campaigns against network,
WAV and Genesis parsers. See `LOOP_V9_3.md` for scope and reproduction settings.
