# ExpOS hardware compatibility

This matrix records observed results, not assumptions. “Untested” is not a
promise that a device should boot.

| Platform | UEFI boot | Install | Storage | Keyboard | Ethernet | Display | Status |
|---|---:|---:|---|---|---|---|---|
| QEMU `pc` + OVMF | Yes | Yes | ATA PIO | PS/2 | RTL8139 | Bochs VGA/GOP | Automated by `make genesis-check` |
| Framework Laptop 13 | Untested | No | No AHCI/NVMe driver | No USB HID | Untested | GOP handoff only | Not ready |
| ThinkPad T480 | Untested | No | No AHCI/NVMe driver | No USB HID | Untested | GOP handoff only | Not ready |
| Dell OptiPlex 7060 | Untested | No | No AHCI/NVMe driver | No USB HID | Untested | GOP handoff only | Not ready |
| Intel NUC | Untested | No | No AHCI/NVMe driver | No USB HID | Untested | GOP handoff only | Not ready |
| Generic Ryzen desktop | Untested | No | No AHCI/NVMe driver | No USB HID | Untested | GOP handoff only | Not ready |

Current physical blockers are explicit: the installer can address only the
legacy primary-master ATA PIO device and cannot yet identify disks by model or
serial; input is PS/2/serial; scanout beyond firmware handoff relies on the
Bochs/QEMU framebuffer; USB, AHCI, NVMe, VirtIO-block and interrupt-driven NIC
support are absent. Do not run the destructive installer against a physical
machine until disk enumeration and identity confirmation are implemented.
