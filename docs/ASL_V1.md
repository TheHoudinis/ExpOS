# Abstracted Silicon Layer v1

ASL v1 is ExpOS's kernel-owned inventory and hardware-ownership boundary. It is
an initial, deliberately small contract: it records firmware and PCI
capabilities, permits one kernel subsystem to claim each resource, and reports
the result without exposing BAR addresses to Forms.

## Boot contract

The x86_64 boot order is:

1. validate the UEFI or Multiboot handoff;
2. establish the Ring 3 Form runtime, TSS, IDT, PIC and PIT;
3. discover the local APIC and reserve the bounded device-vector range;
4. build the ASL inventory from validated GOP metadata and PCI configuration;
5. let xHCI, storage, display and network drivers claim matching entries.

`asl` prints this inventory. Capability IDs are boot-local diagnostic IDs, not
persistent FINs or a stable userspace ABI.

## Capability records

The first implementation classifies:

- firmware framebuffer;
- display controller;
- AHCI storage controller;
- NVMe storage controller;
- xHCI USB controller;
- Ethernet controller; and
- an unknown PCI function.

Each record contains a kind, optional PCI bus/device/function location, one
owner and MSI/MSI-X availability. The public diagnostic view omits MMIO and I/O
addresses. The bounded inventory holds at most 32 records and does not publish
entries beyond that limit.

## Ownership

An entry starts `unbound`, except the firmware framebuffer, which starts owned
by firmware. A matching kernel service may claim it as `ExpDisplay`,
`ExpStorage`, `ExpUSB` or `ExpNetwork`. Repeating the same claim is idempotent;
a different owner cannot replace it. Drivers still validate every BAR, DMA
address, queue size and device-specific bound after receiving a PCI function.

ASL v1 does not yet mint Form Handles for hardware. Forms therefore cannot
claim a PCI function or map a BAR. Future driver-Form access must add an
explicit CFC/Dimension/FIN/Handle grant without weakening this exclusive claim
rule.

## Interrupt contract

The interrupt foundation has three separate phases:

- `supports`: discover conventional PCI MSI and MSI-X capabilities;
- `prepare`: reserve vector `0x40..0x7f` and construct a destination message;
- `arm`: program MSI or MSI-X only after a device-specific IDT handler exists.

Current NVMe and xHCI drivers stop after `prepare` and continue bounded polling.
The `arm` implementation exists for the next delivery phase, but no driver
claims interrupt-driven completion yet. PIC/PIT continues to provide the live
preemptive Form scheduler tick.

## Portability boundary

This release supplies the x86_64 PCI/UEFI backend only. The semantic record and
exclusive-claim rules are intended to survive another architecture, but PCI
configuration mechanism 1, APIC messages and UEFI GOP are not presented as
portable interfaces. CPU topology and cross-core scheduling are kernel
facilities rather than ASL v1 records; the x86_64 backend discovers ACPI MADT
processors and starts APs without making that mechanism part of portable ASL.

## Verification

`make modern-hardware-check` boots real QEMU NVMe and AHCI devices twice to
prove ExpFS write/read persistence, then boots with i8042 disabled and proves
xHCI keyboard and mouse report delivery. `make startup-check` verifies UEFI GOP
and BIOS fallback scanout. `make ring3-check` independently revalidates CPL3,
per-Form CR3 isolation and PIT quota preemption.
