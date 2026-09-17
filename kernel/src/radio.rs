//! Capability-scoped connectivity policy and honest radio discovery.
//!
//! ExpOS currently has a native RTL8139 Ethernet data path, but no 802.11 or
//! USB host stack.  This module therefore separates three things that desktop
//! controls often blur together: software policy, detected hardware, and a
//! loaded data-plane driver.  A switch is only reported operational when all
//! three are true.

use crate::{hardware, slog, sync::SpinMutex};
use expos_core::{CapabilityBroker, Fin, Operations};

/// The settings authority target for connectivity configuration Handles.
pub const RADIO_FIN: Fin = Fin::from_u128(0x5241_4449_4F00_0000_0000_0000_0000_0001);

const NETWORK_CLASS: u8 = 0x02;
const OTHER_NETWORK_SUBCLASS: u8 = 0x80;
const WIRELESS_CLASS: u8 = 0x0D;
const WIFI_A_SUBCLASS: u8 = 0x20;
const WIFI_B_SUBCLASS: u8 = 0x21;
const BLUETOOTH_SUBCLASS: u8 = 0x11;
const SERIAL_BUS_CLASS: u8 = 0x0C;
const USB_SUBCLASS: u8 = 0x03;
const RTL8139_VENDOR: u16 = 0x10EC;
const RTL8139_DEVICE: u16 = 0x8139;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadioKind {
    Wifi,
    Bluetooth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Present,
    Absent,
    /// The bus cannot currently be enumerated far enough to answer honestly.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverState {
    Ready,
    /// Hardware was found, but ExpOS has no driver for it.
    Missing,
    /// The transport itself (currently USB) has no kernel stack yet.
    BusUnsupported,
    /// A supported device exists, but its driver did not initialize.
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Pci,
    Usb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
    /// There is no usable driver with which to establish a connection.
    Unavailable,
    /// The bus cannot currently expose connection state.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdapterStatus {
    pub presence: Presence,
    pub driver: DriverState,
    pub connection: ConnectionState,
    pub transport: Transport,
    pub vendor_id: u16,
    pub device_id: u16,
    pub has_pci_identity: bool,
}

impl AdapterStatus {
    const fn absent(transport: Transport, driver: DriverState) -> Self {
        Self {
            presence: Presence::Absent,
            driver,
            connection: ConnectionState::Unavailable,
            transport,
            vendor_id: 0,
            device_id: 0,
            has_pci_identity: false,
        }
    }

    const fn pci(
        function: hardware::PciFunction,
        driver: DriverState,
        connection: ConnectionState,
    ) -> Self {
        Self {
            presence: Presence::Present,
            driver,
            connection,
            transport: Transport::Pci,
            vendor_id: function.vendor_id,
            device_id: function.device_id,
            has_pci_identity: true,
        }
    }

    const fn unknown_usb() -> Self {
        Self {
            presence: Presence::Unknown,
            driver: DriverState::BusUnsupported,
            connection: ConnectionState::Unknown,
            transport: Transport::Usb,
            vendor_id: 0,
            device_id: 0,
            has_pci_identity: false,
        }
    }

    pub const fn operational(self) -> bool {
        matches!(self.presence, Presence::Present) && matches!(self.driver, DriverState::Ready)
    }

    pub const fn connected(self) -> bool {
        matches!(self.connection, ConnectionState::Connected)
    }

    pub const fn status_text(self) -> &'static str {
        match (self.presence, self.driver, self.connection) {
            (Presence::Present, DriverState::Ready, ConnectionState::Connected) => "Connected",
            (Presence::Present, DriverState::Ready, _) => "Available",
            (Presence::Present, DriverState::Missing, _) => "Detected - driver unavailable",
            (Presence::Present, DriverState::Failed, _) => "Driver failed to start",
            (Presence::Unknown, DriverState::BusUnsupported, _) => "Unavailable - no USB stack",
            (Presence::Absent, _, _) => "No adapter detected",
            _ => "Unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectivitySnapshot {
    /// Ethernet and Wi-Fi data traffic is permitted by local software policy.
    pub network_enabled: bool,
    /// Requested state.  `wifi_operational()` remains the source of truth.
    pub wifi_requested: bool,
    /// Requested state.  `bluetooth_operational()` remains the source of truth.
    pub bluetooth_requested: bool,
    pub ethernet: AdapterStatus,
    pub wifi: AdapterStatus,
    pub bluetooth: AdapterStatus,
    /// True when a PCI USB host controller exists.  This does not imply that
    /// a Bluetooth adapter was discovered because USB enumeration is absent.
    pub usb_controller_detected: bool,
    pub generation: u32,
}

impl ConnectivitySnapshot {
    pub const fn wifi_operational(self) -> bool {
        self.network_enabled && self.wifi_requested && self.wifi.operational()
    }

    pub const fn bluetooth_operational(self) -> bool {
        self.bluetooth_requested && self.bluetooth.operational()
    }

    pub const fn wifi_connected(self) -> bool {
        self.wifi_operational() && self.wifi.connected()
    }

    pub const fn bluetooth_connected(self) -> bool {
        self.bluetooth_operational() && self.bluetooth.connected()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadioError {
    CapabilityDenied,
    NetworkDisabled,
    NoAdapter,
    DriverUnavailable,
    BusUnsupported,
}

impl RadioError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::CapabilityDenied => "DIESE denied connectivity configuration",
            Self::NetworkDisabled => "network access is disabled",
            Self::NoAdapter => "no adapter was detected",
            Self::DriverUnavailable => "adapter detected, but no driver is available",
            Self::BusUnsupported => "adapter discovery needs an unavailable bus driver",
        }
    }
}

#[derive(Clone, Copy)]
struct ConnectivityState {
    network_enabled: bool,
    wifi_requested: bool,
    bluetooth_requested: bool,
    ethernet: AdapterStatus,
    wifi: AdapterStatus,
    bluetooth: AdapterStatus,
    usb_controller_detected: bool,
    generation: u32,
}

impl ConnectivityState {
    const fn empty() -> Self {
        Self {
            network_enabled: true,
            wifi_requested: false,
            bluetooth_requested: false,
            ethernet: AdapterStatus::absent(Transport::Pci, DriverState::Missing),
            wifi: AdapterStatus::absent(Transport::Pci, DriverState::Missing),
            bluetooth: AdapterStatus::unknown_usb(),
            usb_controller_detected: false,
            generation: 0,
        }
    }

    fn discovered(
        ethernet_pci: Option<hardware::PciFunction>,
        ethernet_ready: bool,
        ethernet_connected: bool,
        wifi_pci: Option<hardware::PciFunction>,
        bluetooth_pci: Option<hardware::PciFunction>,
        usb_controller_detected: bool,
    ) -> Self {
        let ethernet = match ethernet_pci {
            Some(device) if ethernet_ready => AdapterStatus::pci(
                device,
                DriverState::Ready,
                if ethernet_connected {
                    ConnectionState::Connected
                } else {
                    ConnectionState::Disconnected
                },
            ),
            Some(device) => {
                AdapterStatus::pci(device, DriverState::Failed, ConnectionState::Unavailable)
            }
            None => AdapterStatus::absent(Transport::Pci, DriverState::Missing),
        };
        let wifi = match wifi_pci {
            Some(device) => {
                AdapterStatus::pci(device, DriverState::Missing, ConnectionState::Unavailable)
            }
            None => AdapterStatus::absent(Transport::Pci, DriverState::Missing),
        };
        let bluetooth = match bluetooth_pci {
            Some(device) => {
                AdapterStatus::pci(device, DriverState::Missing, ConnectionState::Unavailable)
            }
            // Most Bluetooth adapters sit behind USB.  Without USB device
            // enumeration, "unknown" is the only truthful result.
            None => AdapterStatus::unknown_usb(),
        };
        Self {
            ethernet,
            wifi,
            bluetooth,
            usb_controller_detected,
            generation: 1,
            ..Self::empty()
        }
    }

    const fn snapshot(self) -> ConnectivitySnapshot {
        ConnectivitySnapshot {
            network_enabled: self.network_enabled,
            wifi_requested: self.wifi_requested,
            bluetooth_requested: self.bluetooth_requested,
            ethernet: self.ethernet,
            wifi: self.wifi,
            bluetooth: self.bluetooth,
            usb_controller_detected: self.usb_controller_detected,
            generation: self.generation,
        }
    }

    fn refresh_ethernet_connection(&mut self, connected: bool) {
        if self.ethernet.driver != DriverState::Ready {
            return;
        }
        let next = if connected {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        };
        if self.ethernet.connection != next {
            self.ethernet.connection = next;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    fn set_network(&mut self, enabled: bool) {
        if self.network_enabled != enabled {
            self.network_enabled = enabled;
            if !enabled {
                self.wifi_requested = false;
            }
            self.generation = self.generation.wrapping_add(1);
        }
    }

    fn set_radio(&mut self, kind: RadioKind, enabled: bool) -> Result<(), RadioError> {
        let (adapter, current) = match kind {
            RadioKind::Wifi => (self.wifi, &mut self.wifi_requested),
            RadioKind::Bluetooth => (self.bluetooth, &mut self.bluetooth_requested),
        };
        if !enabled {
            if *current {
                *current = false;
                self.generation = self.generation.wrapping_add(1);
            }
            return Ok(());
        }
        if matches!(kind, RadioKind::Wifi) && !self.network_enabled {
            return Err(RadioError::NetworkDisabled);
        }
        match adapter.driver {
            DriverState::Ready if adapter.presence == Presence::Present => {}
            DriverState::BusUnsupported => return Err(RadioError::BusUnsupported),
            DriverState::Missing | DriverState::Failed if adapter.presence == Presence::Present => {
                return Err(RadioError::DriverUnavailable)
            }
            _ => return Err(RadioError::NoAdapter),
        }
        if !*current {
            *current = true;
            self.generation = self.generation.wrapping_add(1);
        }
        Ok(())
    }
}

static CONNECTIVITY: SpinMutex<ConnectivityState> = SpinMutex::new(ConnectivityState::empty());

/// Discover connectivity hardware without claiming unsupported devices.
pub fn initialize(ethernet_ready: bool, ethernet_connected: bool) {
    let ethernet = hardware::find_pci_device(RTL8139_VENDOR, RTL8139_DEVICE);
    let wifi = hardware::find_pci_class(NETWORK_CLASS, OTHER_NETWORK_SUBCLASS)
        .or_else(|| hardware::find_pci_class(WIRELESS_CLASS, WIFI_A_SUBCLASS))
        .or_else(|| hardware::find_pci_class(WIRELESS_CLASS, WIFI_B_SUBCLASS));
    let bluetooth = hardware::find_pci_class(WIRELESS_CLASS, BLUETOOTH_SUBCLASS);
    let usb_controller = hardware::find_pci_class(SERIAL_BUS_CLASS, USB_SUBCLASS).is_some();
    *CONNECTIVITY.lock() = ConnectivityState::discovered(
        ethernet,
        ethernet_ready,
        ethernet_connected,
        wifi,
        bluetooth,
        usb_controller,
    );
    let snapshot = snapshot();
    slog!(
        "EXPOS_RADIO_READY ethernet={} wifi={} bluetooth={} usb_controller={}\r\n",
        snapshot.ethernet.status_text(),
        snapshot.wifi.status_text(),
        snapshot.bluetooth.status_text(),
        snapshot.usb_controller_detected
    );
}

/// Restore trusted local policy after the state journal has been verified.
///
/// This is deliberately separate from the user-facing setters: interactive
/// changes still require a requester-bound Configure Handle, while early boot
/// is restoring policy that was already authorized and committed. Requested
/// radios are only restored when their discovered adapter and driver are
/// actually usable, so persisted UI state can never manufacture connectivity.
pub(crate) fn restore_persisted_policy(
    network_enabled: bool,
    wifi_requested: bool,
    bluetooth_requested: bool,
) {
    let mut connectivity = CONNECTIVITY.lock();
    connectivity.set_network(network_enabled);

    let wifi_result = connectivity.set_radio(RadioKind::Wifi, network_enabled && wifi_requested);
    let bluetooth_result = connectivity.set_radio(RadioKind::Bluetooth, bluetooth_requested);
    let snapshot = connectivity.snapshot();
    drop(connectivity);

    slog!(
        "EXPOS_RADIO_POLICY_RESTORED network={} wifi={} bluetooth={}\r\n",
        snapshot.network_enabled,
        snapshot.wifi_requested,
        snapshot.bluetooth_requested
    );
    if wifi_requested && wifi_result.is_err() {
        slog!(
            "EXPOS_RADIO_POLICY_SKIPPED kind=wifi reason={:?}\r\n",
            wifi_result.err()
        );
    }
    if bluetooth_requested && bluetooth_result.is_err() {
        slog!(
            "EXPOS_RADIO_POLICY_SKIPPED kind=bluetooth reason={:?}\r\n",
            bluetooth_result.err()
        );
    }
}

pub fn snapshot() -> ConnectivitySnapshot {
    let ethernet_connected = crate::network::link_up();
    let mut connectivity = CONNECTIVITY.lock();
    connectivity.refresh_ethernet_connection(ethernet_connected);
    connectivity.snapshot()
}

/// Fast policy check for callers before they attempt Ethernet or Wi-Fi I/O.
pub fn network_allowed() -> bool {
    CONNECTIVITY.lock().network_enabled
}

pub fn set_network_enabled(
    broker: &CapabilityBroker,
    handle_id: u32,
    requester: Fin,
    dimension: Fin,
    enabled: bool,
) -> Result<ConnectivitySnapshot, RadioError> {
    authorize_configuration(broker, handle_id, requester, dimension)?;
    let mut connectivity = CONNECTIVITY.lock();
    connectivity.set_network(enabled);
    Ok(connectivity.snapshot())
}

pub fn set_radio_enabled(
    broker: &CapabilityBroker,
    handle_id: u32,
    requester: Fin,
    dimension: Fin,
    kind: RadioKind,
    enabled: bool,
) -> Result<ConnectivitySnapshot, RadioError> {
    authorize_configuration(broker, handle_id, requester, dimension)?;
    let mut connectivity = CONNECTIVITY.lock();
    connectivity.set_radio(kind, enabled)?;
    Ok(connectivity.snapshot())
}

fn authorize_configuration(
    broker: &CapabilityBroker,
    handle_id: u32,
    requester: Fin,
    dimension: Fin,
) -> Result<(), RadioError> {
    broker
        .authorize_requester(
            handle_id,
            requester,
            RADIO_FIN,
            dimension,
            Operations::CONFIGURE,
            hardware::timestamp(),
        )
        .map_err(|_| RadioError::CapabilityDenied)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIFI: hardware::PciFunction = hardware::PciFunction {
        bus: 1,
        slot: 2,
        function: 0,
        vendor_id: 0x1234,
        device_id: 0x5678,
        class_code: NETWORK_CLASS,
        subclass: OTHER_NETWORK_SUBCLASS,
        programming_interface: 0,
    };

    #[test]
    fn unsupported_wifi_never_looks_enabled() {
        let mut state = ConnectivityState::discovered(None, false, false, Some(WIFI), None, true);
        assert_eq!(
            state.set_radio(RadioKind::Wifi, true),
            Err(RadioError::DriverUnavailable)
        );
        assert!(!state.snapshot().wifi_requested);
        assert!(!state.snapshot().wifi_operational());
    }

    #[test]
    fn disabling_network_also_blocks_wifi_policy() {
        let mut state = ConnectivityState::discovered(None, false, false, None, None, false);
        state.wifi = AdapterStatus::pci(WIFI, DriverState::Ready, ConnectionState::Disconnected);
        state.set_radio(RadioKind::Wifi, true).unwrap();
        assert!(state.snapshot().wifi_operational());
        state.set_network(false);
        assert!(!state.snapshot().wifi_requested);
        assert!(!state.snapshot().wifi_operational());
    }

    #[test]
    fn bluetooth_reports_unknown_without_usb_enumeration() {
        let state = ConnectivityState::discovered(None, false, false, None, None, true);
        assert_eq!(state.bluetooth.presence, Presence::Unknown);
        assert_eq!(state.bluetooth.driver, DriverState::BusUnsupported);
        assert_eq!(state.bluetooth.status_text(), "Unavailable - no USB stack");
    }

    #[test]
    fn restored_requests_cannot_enable_missing_radios() {
        let mut state = ConnectivityState::discovered(None, false, false, None, None, false);
        state.set_network(false);
        assert_eq!(
            state.set_radio(RadioKind::Wifi, true),
            Err(RadioError::NetworkDisabled)
        );
        assert_eq!(
            state.set_radio(RadioKind::Bluetooth, true),
            Err(RadioError::BusUnsupported)
        );
        assert!(!state.snapshot().wifi_requested);
        assert!(!state.snapshot().bluetooth_requested);
    }
}
