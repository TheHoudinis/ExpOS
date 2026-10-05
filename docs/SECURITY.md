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
| Kernel memory corruption | Rust for normal parsing/state paths; CR0.WP; NXE; SMEP; read-only executable Form code; NX Form data/stack; fixed-capacity protocol records | Hardware drivers and FFI still require `unsafe`; the monolithic kernel and its identity-mapped kernel image are not fully W^X or fault-isolated; no SMAP or IOMMU |
| Malformed network/TLS input | Driver receive lengths are bounded before copies; IPv4/UDP/TCP/DNS/HTTP/TLS decoders validate nested lengths; response/document sizes are fixed; network initialization can be omitted | Network and TLS still execute in Ring 0; fuzzing and a user-mode driver/network split are not complete |
| MicroPython escape | Minimal ROM feature set, no external imports/filesystem/network modules, fresh 256 KiB heap, source/stack/step/output limits, non-catchable abort, callback metadata checks, and install-time omission | MicroPython is C running in Ring 0 when enabled; these bounds reduce exposure but do not make interpreter memory corruption harmless |
| Capability forgery/leak | CFC/requester/target/Dimension/operation/expiry validation; parent-linked revocation; restored children must re-prove strict attenuation; IDs skip collisions; ExpSeal closes new ambient root issuance | Kernel compromise can bypass the broker; durable general seal/context isolation remains incomplete |
| Package supply chain | HTTPS registries require a pinned Ed25519 public key; SHA-256 verifies artifacts; no package scripts or links; safe bounded extraction, ownership receipts, rollback and atomic state publication | The compiled seed catalog is part of the trusted image; a public transparency log and isolated third-party native loader do not yet exist |
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

- ExpPython, Browser, TLS and device drivers are not separate Ring 3 services.
- Kernel text/rodata/data do not yet have complete section-level W^X mappings.
- Secure Boot, measured boot, TPM sealing, IOMMU DMA isolation and signed kernel
  updates are not implemented.
- CRC protects unencrypted Architect records from accidental damage, not a
  malicious disk editor.
- Static tests do not replace hostile packet/parser fuzzing or physical-machine
  validation.
