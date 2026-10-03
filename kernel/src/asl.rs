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
    StorageAhci,
    StorageNvme,
    UsbXhci,
    NetworkEthernet,
    OtherPci,
}

impl Kind {
    const fn label(self) -> &'static str {
        match self {
            Self::FirmwareFramebuffer => "firmware-framebuffer",
            Self::Display => "display",
            Self::StorageAhci => "storage-ahci",
            Self::StorageNvme => "storage-nvme",
            Self::UsbXhci => "usb-xhci",
            Self::NetworkEthernet => "network-ethernet",
            Self::OtherPci => "pci-function",
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
        );
    }
    pci::visit(|function| {
        let kind = classify(function);
        let (msi, msix) = interrupts::supports(function);
        inventory.push(kind, Some(function.location), Owner::Unbound, msi, msix);
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
    crate::println!("ID   KIND                  LOCATION     OWNER        IRQ");
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
                "{:<4} {:<21} {:02x}:{:02x}.{}    {:<12} {}",
                entry.id,
                entry.kind.label(),
                location.bus,
                location.slot,
                location.function,
                entry.owner.label(),
                irq
            );
        } else {
            crate::println!(
                "{:<4} {:<21} firmware     {:<12} {}",
                entry.id,
                entry.kind.label(),
                entry.owner.label(),
                irq
            );
        }
    }
}

const fn classify(function: pci::Function) -> Kind {
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
        _ => Kind::OtherPci,
    }
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
        );
        assert!(inventory.claim(location, Owner::ExpStorage));
        assert!(!inventory.claim(location, Owner::ExpUsb));
    }
}
