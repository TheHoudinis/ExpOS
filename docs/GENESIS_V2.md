# Genesis v2 implementation slice

Genesis v2 keeps the existing native-UEFI whole-disk constructor and adds a
versioned, redundant installation plan at LBAs 42 and 43. Manifest v4 marks the
plan as mandatory, so damaged or disagreeing copies fail closed.

## Installed choices

- protection: Easy encrypted, Paranoid encrypted, or Architect-only
  unencrypted development mode;
- packages: Minimal, Essentials, all thirty optional Ayo apps, or Custom with
  independent Browser, ExpPython and per-app selection;
- optional runtime components: Browser and ExpPython according to the selected
  package/security profile;
- SDK contract set: none, Rust+C, or Rust+C+Go+Python;
- detected RTL8139 network driver on/off (forced off by Paranoid);
- zero to five additional security-boundary Dimensions; and
- zero to two initial Power/Guest accounts in addition to Operator.

The first installed boot seeds these choices into the transactional CFC state.
Dimensions become real persisted Dimension records, accounts become hashed
ExpFS account records, selected apps become `AyoApps.state`, and the complete
selection is retained in `GenesisPlan.state`. Seeding is one-shot: later user
changes are authoritative and are not overwritten on every reboot.

## Hardware overview and warm-reboot input

Before asking destructive questions Genesis reports x86-64, NX, SMEP, hardware
entropy, storage transport/size, PS/2 or USB keyboard detection, GOP display and
the currently supported RTL8139 Ethernet scope. This is an implementation
probe, not certification for untested physical hardware.

Both the installer and installed runtime now reclaim the 8042 path from
firmware: disable ports, drain stale bytes, explicitly enable set-2 scanning,
enable controller set-1 translation, and retain polling ownership. This targets
the failure where a warm reboot from Genesis left the installed system with
keyboard scanning or translation disabled. xHCI HID remains an independent
input path.

## Recovery from removable media

When removable Genesis media sees existing ExpOS metadata on the selected
device it offers:

1. read-only troubleshooting of disk, manifest, plan and encryption status;
2. verified repair from one valid redundant metadata copy;
3. authenticated restore of the protected installation baseline; or
4. confirmed annihilation of ExpOS boot, identity, ExpFS, checkpoints and
   baseline regions.

Recovery intentionally does not guess about unknown non-ExpOS disks. The
current installer still selects storage by driver probe order and therefore is
not approved for physical-disk installation until a model/serial/stable-device
selector and physical test matrix land.

## UI and verification

The flow uses a compact hardware overview, short numbered choices, an explicit
final summary, eight visible progress bars and stage-specific errors. Install
verification reads back GPT, FAT32, the complete UEFI application, redundant
manifest v4 copies and redundant plan copies before reporting completion.

The current UI remains a keyboard-driven framebuffer console. A mouse-first
widget wizard, stable physical disk picker, Secure Boot and additional hardware
drivers are later Genesis v2 work.
