# ExpOS hardware compatibility

This matrix records observed results, not assumptions. “Implemented, untested”
is not a promise that a physical machine will boot.

| Platform | UEFI boot | Install | Storage | Keyboard / mouse | Ethernet | Audio | Display | Status |
|---|---:|---:|---|---|---|---|---|---|
| QEMU `pc` + OVMF/SeaBIOS | Yes | Yes | NVMe, AHCI, ATA-PIO, transitional VirtIO-block | xHCI HID, PS/2 | RTL8139, e1000/e1000e, transitional VirtIO-net | AC'97, Intel HDA and modern VirtIO Sound PCM output; Ayo-installed VBR MP3 | direct UEFI GOP; Bochs BIOS fallback | Automated by modern-hardware, driver-network, HDA/audio/modern-audio and Genesis checks |
| Framework Laptop 13 | Untested | Not approved | NVMe implemented, untested | xHCI HID implemented, untested | Untested | HDA implemented, untested | GOP implemented, untested | Not ready for destructive install |
| ThinkPad T480 | Untested | Not approved | NVMe/AHCI implemented, untested | xHCI HID implemented, untested | Untested | HDA implemented, untested | GOP implemented, untested | Not ready for destructive install |
| Dell OptiPlex 7060 | Untested | Not approved | NVMe/AHCI implemented, untested | xHCI HID implemented, untested | Untested | HDA implemented, untested | GOP implemented, untested | Not ready for destructive install |
| Intel NUC | Untested | Not approved | NVMe/AHCI implemented, untested | xHCI HID implemented, untested | Untested | HDA implemented, untested | GOP implemented, untested | Not ready for destructive install |
| Generic Ryzen desktop | Untested | Not approved | NVMe/AHCI implemented, untested | xHCI HID implemented, untested | Untested | HDA implemented, USB audio unsupported | GOP implemented, untested | Not ready for destructive install |

The first modern-driver milestone is intentionally bounded:

- runtime storage selects the first usable NVMe controller/namespace, AHCI SATA,
  transitional VirtIO block or legacy ATA-PIO target; NVMe currently requires
  a 512-byte LBA namespace and no backend supports hotplug or IOMMU remapping;
- xHCI supports one controller, fixed DMA rings, up to four boot-protocol HID
  devices and eight scratchpads; it does not provide hubs, USB mass storage,
  Bluetooth, audio or arbitrary HID report parsing;
- ExpAudio supports QEMU VirtIO Sound plus Intel/QEMU-compatible AC'97 and
  Intel HDA PCM-out. VirtIO Sound uses modern PCI capabilities, output-stream
  discovery, SET_PARAMS/PREPARE/START and TX used-ring completion; the Intel
  paths use bounded descriptor rings and legacy PCI INTx. All share 48 kHz
  stereo signed-16 buffers, strict integer-PCM WAV conversion, bounded VBR MP3
  streaming from the VM-only Ayo media store, and underrun diagnostics. The
  store deliberately refuses partitioned Genesis disks. USB audio and physical audio validation
  remain absent, so physical audio is not claimed;
- UEFI uses validated GOP address, size, geometry, stride and RGB/BGR order
  directly. The kernel does not change firmware modes or negotiate EDID;
- PCI MSI/MSI-X capability discovery, vector reservation and programming exist.
  The I/O APIC routes live AC'97, HDA and e1000/e1000e INTx completion; NVMe and
  xHCI retain bounded polling; and
- DMA relies on the current below-4-GiB identity-mapped kernel allocation model.

Genesis inventories up to eight implemented targets, shows model, serial,
capacity, controller and a stable fingerprint, then requires an explicit index
and exact serial before the separate whole-disk `ERASE` gate. The QEMU test
selects an ATA slave beside an unchanged decoy master. Multi-port AHCI and
multi-namespace NVMe enumeration and physical-hardware validation remain open.
Do not run the destructive installer against a physical machine yet.

Genesis v2 prints a preflight overview for x86-64, NX, SMEP, RDRAND, storage,
GOP and detected PS/2/xHCI keyboard paths. The installed runtime explicitly
reinitializes the 8042 keyboard controller after a warm Genesis reboot instead
of assuming firmware left scanning and translation enabled. QEMU coverage is
not evidence that the same path works on the untested physical systems above.
