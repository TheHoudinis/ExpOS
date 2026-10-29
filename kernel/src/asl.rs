//! Abstracted Silicon Layer v1 capability inventory.
//!
//! ASL does not pretend every device is portable. It assigns typed bounded
//! capability records to firmware and PCI resources, records one kernel driver
//! binding, and exposes diagnostics without leaking BAR addresses to Forms.

use crate::{interrupts, pci, sync::SpinMutex};

pub const ASL_VERSION: u16 = 1;
const MAX_CAPABILITIES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    FirmwareFramebuffer,
    Display,
    DisplayVirtio,
    StorageAhci,
    StorageNvme,
    StorageVirtio,
    UsbXhci,
    NetworkEthernet,
    NetworkE1000,
    NetworkVirtio,
    Audio,
    AudioHda,
    AudioVirtio,
    OtherPci,
}

impl Kind {
    const fn label(self) -> &'static str {
        match self {
            Self::FirmwareFramebuffer => "firmware-framebuffer",
            Self::Display => "display",
            Self::DisplayVirtio => "display-virtio",
            Self::StorageAhci => "storage-ahci",
            Self::StorageNvme => "storage-nvme",
            Self::StorageVirtio => "storage-virtio",
            Self::UsbXhci => "usb-xhci",
            Self::NetworkEthernet => "network-ethernet",
            Self::NetworkE1000 => "network-e1000",
            Self::NetworkVirtio => "network-virtio",
            Self::Audio => "audio",
            Self::AudioHda => "audio-hda",
            Self::AudioVirtio => "audio-virtio",
            Self::OtherPci => "pci-function",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DriverStage {
    Operational,
    ProbeOnly,
    Generic,
}

impl DriverStage {
    const fn label(self) -> &'static str {
        match self {
            Self::Operational => "operational",
            Self::ProbeOnly => "probe-only",
            Self::Generic => "inventory",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Owner {
    Unbound,
    Firmware,
    ExpDisplay,
    ExpStorage,
    ExpUsb,
    ExpNetwork,
    ExpAudio,
}

impl Owner {
    const fn label(self) -> &'static str {
        match self {
            Self::Unbound => "unbound",
            Self::Firmware => "firmware",
            Self::ExpDisplay => "ExpDisplay",
            Self::ExpStorage => "ExpStorage",
            Self::ExpUsb => "ExpUSB",
            Self::ExpNetwork => "ExpNetwork",
            Self::ExpAudio => "ExpAudio",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Capability {
    id: u16,
    kind: Kind,
    location: Option<pci::Location>,
    owner: Owner,
    msi: bool,
    msix: bool,
    stage: DriverStage,
}

struct Inventory {
    entries: [Option<Capability>; MAX_CAPABILITIES],
    count: usize,
}

impl Inventory {
    const fn new() -> Self {
        Self {
            entries: [None; MAX_CAPABILITIES],
            count: 0,
        }
    }

    fn push(
        &mut self,
        kind: Kind,
        location: Option<pci::Location>,
        owner: Owner,
        msi: bool,
        msix: bool,
        stage: DriverStage,
    ) {
        if self.count == MAX_CAPABILITIES {
            return;
        }
        self.entries[self.count] = Some(Capability {
            id: (self.count + 1) as u16,
            kind,
            location,
            owner,
            msi,
            msix,
            stage,
        });
        self.count += 1;
    }

    fn claim(&mut self, location: pci::Location, owner: Owner) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.location == Some(location))
        else {
            return false;
        };
        if entry.owner != Owner::Unbound && entry.owner != owner {
            return false;
        }
        entry.owner = owner;
        true
    }

    fn release(&mut self, location: pci::Location, owner: Owner) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.location == Some(location))
        else {
            return false;
        };
        if owner == Owner::Unbound || entry.owner != owner {
            return false;
        }
        entry.owner = Owner::Unbound;
        true
    }
}

static INVENTORY: SpinMutex<Inventory> = SpinMutex::new(Inventory::new());

pub fn initialize() {
    let mut inventory = INVENTORY.lock();
    *inventory = Inventory::new();
    if crate::boot::firmware_framebuffer().is_some() {
        inventory.push(
            Kind::FirmwareFramebuffer,
            None,
            Owner::Firmware,
            false,
            false,
            DriverStage::Operational,
        );
    }
    pci::visit(|function| {
        let kind = classify(function);
        let (msi, msix) = interrupts::supports(function);
        let stage = driver_stage(function, kind);
        if stage == DriverStage::ProbeOnly {
            crate::slog!(
                "EXPOS_DRIVER_PROBE family={} vendor={:04x} device={:04x} stage=probe-only\r\n",
                kind.label(),
                function.vendor_id,
                function.device_id
            );
        }
        inventory.push(
            kind,
            Some(function.location),
            Owner::Unbound,
            msi,
            msix,
            stage,
        );
        None::<()>
    });
    crate::slog!(
        "EXPOS_ASL_READY version={} capabilities={} ownership=exclusive handles=planned\r\n",
        ASL_VERSION,
        inventory.count
    );
}

pub fn claim(function: pci::Function, owner: Owner) -> bool {
    INVENTORY.lock().claim(function.location, owner)
}

/// Return a failed driver's exact capability to the unbound inventory. A
/// caller cannot release another driver's device or use `Unbound` as an owner.
pub fn release(function: pci::Function, owner: Owner) -> bool {
    INVENTORY.lock().release(function.location, owner)
}

pub fn claim_firmware_framebuffer() {
    if let Some(entry) = INVENTORY
        .lock()
        .entries
        .iter_mut()
        .flatten()
        .find(|entry| entry.kind == Kind::FirmwareFramebuffer)
    {
        entry.owner = Owner::ExpDisplay;
    }
}

pub fn print_inventory() {
    let inventory = INVENTORY.lock();
    crate::println!("ASL v{} CAPABILITY INVENTORY", ASL_VERSION);
    crate::println!("ID   KIND                  LOCATION     OWNER        IRQ          DRIVER");
    for entry in inventory.entries.iter().flatten() {
        let irq = if entry.msix {
            "MSI-X"
        } else if entry.msi {
            "MSI"
        } else {
            "legacy/poll"
        };
        if let Some(location) = entry.location {
            crate::println!(
                "{:<4} {:<21} {:02x}:{:02x}.{}    {:<12} {:<12} {}",
                entry.id,
                entry.kind.label(),
                location.bus,
                location.slot,
                location.function,
                entry.owner.label(),
                irq,
                entry.stage.label()
            );
        } else {
            crate::println!(
                "{:<4} {:<21} firmware     {:<12} {:<12} {}",
                entry.id,
                entry.kind.label(),
                entry.owner.label(),
                irq,
                entry.stage.label()
            );
        }
    }
}

const fn classify(function: pci::Function) -> Kind {
    if function.vendor_id == 0x1AF4 {
        return match function.device_id {
            0x1000 | 0x1041 => Kind::NetworkVirtio,
            0x1001 | 0x1042 => Kind::StorageVirtio,
            0x1050 => Kind::DisplayVirtio,
            0x1059 => Kind::AudioVirtio,
            _ => Kind::OtherPci,
        };
    }
    if function.vendor_id == 0x8086
        && matches!(function.device_id, 0x100E | 0x100F | 0x10D3 | 0x153A)
    {
        return Kind::NetworkE1000;
    }
    match (
        function.class_code,
        function.subclass,
        function.programming_interface,
    ) {
        (0x01, 0x06, 0x01) => Kind::StorageAhci,
        (0x01, 0x08, 0x02) => Kind::StorageNvme,
        (0x03, _, _) => Kind::Display,
        (0x0C, 0x03, 0x30) => Kind::UsbXhci,
        (0x02, 0x00, _) => Kind::NetworkEthernet,
        (0x04, 0x03, _) => Kind::AudioHda,
        (0x04, _, _) => Kind::Audio,
        _ => Kind::OtherPci,
    }
}

const fn driver_stage(function: pci::Function, kind: Kind) -> DriverStage {
    match kind {
        Kind::FirmwareFramebuffer
        | Kind::Display
        | Kind::DisplayVirtio
        | Kind::AudioVirtio
        | Kind::StorageAhci
        | Kind::StorageNvme
        | Kind::UsbXhci => DriverStage::Operational,
        Kind::NetworkEthernet if function.vendor_id == 0x10EC && function.device_id == 0x8139 => {
            DriverStage::Operational
        }
        Kind::Audio if function.vendor_id == 0x8086 && function.device_id == 0x2415 => {
            DriverStage::Operational
        }
        Kind::NetworkE1000 | Kind::AudioHda => DriverStage::Operational,
        Kind::StorageVirtio if function.device_id == 0x1001 => DriverStage::Operational,
        Kind::NetworkVirtio if function.device_id == 0x1000 => DriverStage::Operational,
        Kind::StorageVirtio | Kind::NetworkVirtio => DriverStage::ProbeOnly,
        _ => DriverStage::Generic,
    }
}

#[cfg_attr(not(feature = "genesis-installer"), allow(dead_code))]
pub fn driver_summary() -> (usize, usize) {
    let inventory = INVENTORY.lock();
    let operational = inventory
        .entries
        .iter()
        .flatten()
        .filter(|entry| entry.stage == DriverStage::Operational)
        .count();
    let probe_only = inventory
        .entries
        .iter()
        .flatten()
        .filter(|entry| entry.stage == DriverStage::ProbeOnly)
        .count();
    (operational, probe_only)
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn function(class_code: u8, subclass: u8, interface: u8) -> pci::Function {
        pci::Function {
            location: pci::Location {
                bus: 0,
                slot: 1,
                function: 0,
            },
            vendor_id: 1,
            device_id: 2,
            class_code,
            subclass,
            programming_interface: interface,
            revision: 0,
        }
    }

    #[test]
    fn classifies_modern_storage_and_usb_without_vendor_ids() {
        assert_eq!(classify(function(1, 6, 1)), Kind::StorageAhci);
        assert_eq!(classify(function(1, 8, 2)), Kind::StorageNvme);
        assert_eq!(classify(function(0x0C, 3, 0x30)), Kind::UsbXhci);
    }

    #[test]
    fn recognizes_operational_legacy_transports_and_probe_only_modern_virtio() {
        let mut candidate = function(0x02, 0x00, 0);
        candidate.vendor_id = 0x8086;
        candidate.device_id = 0x100E;
        assert_eq!(classify(candidate), Kind::NetworkE1000);
        assert_eq!(
            driver_stage(candidate, classify(candidate)),
            DriverStage::Operational
        );

        candidate.vendor_id = 0x1AF4;
        candidate.device_id = 0x1042;
        assert_eq!(classify(candidate), Kind::StorageVirtio);
        assert_eq!(
            driver_stage(candidate, classify(candidate)),
            DriverStage::ProbeOnly
        );

        candidate.device_id = 0x1001;
        assert_eq!(
            driver_stage(candidate, classify(candidate)),
            DriverStage::Operational
        );

        candidate.device_id = 0x1059;
        assert_eq!(classify(candidate), Kind::AudioVirtio);
        assert_eq!(
            driver_stage(candidate, classify(candidate)),
            DriverStage::Operational
        );

        candidate.vendor_id = 0x1234;
        candidate.device_id = 0x5678;
        candidate.class_code = 0x04;
        candidate.subclass = 0x03;
        assert_eq!(classify(candidate), Kind::AudioHda);
        assert_eq!(
            driver_stage(candidate, classify(candidate)),
            DriverStage::Operational
        );
    }

    #[test]
    fn ownership_is_exclusive() {
        let mut inventory = Inventory::new();
        let location = pci::Location {
            bus: 0,
            slot: 2,
            function: 0,
        };
        inventory.push(
            Kind::StorageNvme,
            Some(location),
            Owner::Unbound,
            true,
            true,
            DriverStage::Operational,
        );
        assert!(inventory.claim(location, Owner::ExpStorage));
        assert!(!inventory.claim(location, Owner::ExpUsb));
        assert!(!inventory.release(location, Owner::ExpUsb));
        assert!(inventory.release(location, Owner::ExpStorage));
        assert!(inventory.claim(location, Owner::ExpUsb));
    }
}
