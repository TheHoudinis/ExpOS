# ExpOS hardware compatibility

This matrix records observed results, not assumptions. “Implemented, untested”
is not a promise that a physical machine will boot.

| Platform | UEFI boot | Install | Storage | Keyboard / mouse | Ethernet | Display | Status |
|---|---:|---:|---|---|---|---|---|
| QEMU `pc` + OVMF/SeaBIOS | Yes | Yes | NVMe, AHCI, ATA-PIO | xHCI HID, PS/2 | RTL8139 | direct UEFI GOP; Bochs BIOS fallback | Automated by modern-hardware, startup and Genesis checks |
| Framework Laptop 13 | Untested | Not approved | NVMe implemented, untested | xHCI HID implemented, untested | Untested | GOP implemented, untested | Not ready for destructive install |
| ThinkPad T480 | Untested | Not approved | NVMe/AHCI implemented, untested | xHCI HID implemented, untested | Untested | GOP implemented, untested | Not ready for destructive install |
| Dell OptiPlex 7060 | Untested | Not approved | NVMe/AHCI implemented, untested | xHCI HID implemented, untested | Untested | GOP implemented, untested | Not ready for destructive install |
| Intel NUC | Untested | Not approved | NVMe/AHCI implemented, untested | xHCI HID implemented, untested | Untested | GOP implemented, untested | Not ready for destructive install |
| Generic Ryzen desktop | Untested | Not approved | NVMe/AHCI implemented, untested | xHCI HID implemented, untested | Untested | GOP implemented, untested | Not ready for destructive install |

The first modern-driver milestone is intentionally bounded:

- storage selects the first usable NVMe controller/namespace, then AHCI SATA,
  then legacy ATA-PIO; NVMe currently requires a 512-byte LBA namespace and
  neither driver supports hotplug, power management or IOMMU remapping;
- xHCI supports one controller, fixed DMA rings, up to four boot-protocol HID
  devices and eight scratchpads; it does not provide hubs, USB mass storage,
  Bluetooth, audio or arbitrary HID report parsing;
- UEFI uses validated GOP address, size, geometry, stride and RGB/BGR order
  directly. The kernel does not change firmware modes or negotiate EDID;
- PCI MSI/MSI-X capability discovery, vector reservation and programming exist,
  but storage, USB and networking still use bounded polling because device IDT
  handlers are not armed yet; and
- DMA relies on the current below-4-GiB identity-mapped kernel allocation model.

Genesis can issue I/O through NVMe or AHCI, but it still chooses by driver probe
order and does not show a model, serial number or stable device identity before
the whole-disk `ERASE` gate. Do not run the destructive installer against a
physical machine until identity-based disk selection and physical-hardware
validation land. VirtIO-block is not implemented.
