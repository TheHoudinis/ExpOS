//! Minimal native IPv4 networking for the transitional x86_64 kernel.
//!
//! The stack is deliberately small and allocation-free: a polling RTL8139
//! driver, Ethernet/ARP, static IPv4, ICMP echo, checksum-correct UDP, DNS A
//! queries, a single synchronous TCP client, and bounded HTTP/1.0 GET. TLS 1.3
//! is layered over the streaming TCP interface in `crate::tls`; DHCP, IPv6,
//! TCP servers, and concurrent sockets are not implemented.

use crate::{port, println, slog};
use core::sync::atomic::{compiler_fence, Ordering};
use hexa_core::{CapabilityBroker, Fin, Operations};

pub const NETWORK_FIN: Fin = Fin::from_u128(0x4E45_5457_4F52_4B00_0000_0000_0000_0001);

const RTL_VENDOR: u16 = 0x10EC;
const RTL_DEVICE: u16 = 0x8139;

const IDR0: u16 = 0x00;
const TSD0: u16 = 0x10;
const TSAD0: u16 = 0x20;
const RBSTART: u16 = 0x30;
const COMMAND: u16 = 0x37;
const CAPR: u16 = 0x38;
const IMR: u16 = 0x3C;
const ISR: u16 = 0x3E;
const TCR: u16 = 0x40;
const RCR: u16 = 0x44;
const CONFIG1: u16 = 0x52;
const MEDIA_STATUS: u16 = 0x58;

const COMMAND_RX_BUFFER_EMPTY: u8 = 1;
const COMMAND_TX_ENABLE: u8 = 1 << 2;
const COMMAND_RX_ENABLE: u8 = 1 << 3;
const COMMAND_RESET: u8 = 1 << 4;

const TX_STATUS_OWN: u32 = 1 << 13;
const TX_STATUS_OK: u32 = 1 << 15;
const TX_STATUS_ABORTED: u32 = 1 << 30;

const RX_RING_SIZE: usize = 8 * 1024;
const MAX_FRAME_SIZE: usize = 1536;
const RX_BUFFER_SIZE: usize = RX_RING_SIZE + 16 + MAX_FRAME_SIZE;
const TX_BUFFER_COUNT: usize = 4;
const ETHERNET_HEADER_SIZE: usize = 14;

const LOCAL_IP: [u8; 4] = [10, 0, 2, 15];
const NETMASK: [u8; 4] = [255, 255, 255, 0];
const GATEWAY: [u8; 4] = [10, 0, 2, 2];
const DNS_SERVER: [u8; 4] = [10, 0, 2, 3];
const BROADCAST_MAC: [u8; 6] = [0xFF; 6];

const IO_WAIT_LIMIT: usize = 2_000_000;
const RECEIVE_WAIT_LIMIT: usize = 12_000_000;
const TRANSPORT_RETRIES: usize = 3;
const UDP_HEADER_SIZE: usize = 8;
const TCP_HEADER_SIZE: usize = 20;
const TCP_SYN_HEADER_SIZE: usize = 24;
const DNS_PACKET_CAPACITY: usize = 512;
const HTTP_REQUEST_CAPACITY: usize = 768;
const HTTP_WIRE_CAPACITY: usize = 16 * 1024;
pub const HTTP_BODY_CAPACITY: usize = 14 * 1024;
const HTTP_LOCATION_CAPACITY: usize = 512;
const TCP_PAYLOAD_CAPACITY: usize = 1500 - 20 - TCP_HEADER_SIZE;
const TCP_RECEIVE_WINDOW: u16 = 32 * 1024;

const TCP_FIN: u8 = 0x01;
const TCP_SYN: u8 = 0x02;
const TCP_RST: u8 = 0x04;
const TCP_PSH: u8 = 0x08;
const TCP_ACK: u8 = 0x10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkError {
    CapabilityDenied,
    PolicyDisabled,
    BadAddress,
    BadHostname,
    BadUrl,
    UnsupportedScheme,
    NoDevice,
    LinkDown,
    ArpTimeout,
    TransmitFailed,
    ReplyTimeout,
    DnsTimeout,
    DnsRefused,
    DnsNoAddress,
    MalformedDns,
    TcpTimeout,
    TcpReset,
    EntropyUnavailable,
    TlsHandshake,
    TlsCertificate,
    TlsProtocol,
    MalformedHttp,
}

impl NetworkError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::CapabilityDenied => "DIESE denied the Network Handle",
            Self::PolicyDisabled => "network access is disabled in Settings",
            Self::BadAddress => "invalid IPv4 address",
            Self::BadHostname => "invalid DNS hostname",
            Self::BadUrl => "invalid HTTP URL",
            Self::UnsupportedScheme => "URL scheme must be http:// or https://",
            Self::NoDevice => "RTL8139 network device is unavailable",
            Self::LinkDown => "RTL8139 link is down",
            Self::ArpTimeout => "ARP neighbor resolution timed out",
            Self::TransmitFailed => "RTL8139 transmit failed",
            Self::ReplyTimeout => "network reply timed out",
            Self::DnsTimeout => "DNS reply timed out",
            Self::DnsRefused => "DNS server returned an error",
            Self::DnsNoAddress => "DNS response contains no IPv4 address",
            Self::MalformedDns => "malformed DNS response",
            Self::TcpTimeout => "TCP peer timed out",
            Self::TcpReset => "TCP peer reset the connection",
            Self::EntropyUnavailable => "secure hardware entropy is unavailable",
            Self::TlsHandshake => "TLS 1.3 handshake failed",
            Self::TlsCertificate => "TLS certificate or hostname verification failed",
            Self::TlsProtocol => "TLS peer returned an unsupported or malformed record",
            Self::MalformedHttp => "malformed or incomplete HTTP response",
        }
    }
}

impl core::fmt::Display for NetworkError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl core::error::Error for NetworkError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UdpReply {
    pub source: [u8; 4],
    pub source_port: u16,
    pub bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: [u8; HTTP_BODY_CAPACITY],
    pub body_len: usize,
    pub truncated: bool,
    pub peer: [u8; 4],
    location: [u8; HTTP_LOCATION_CAPACITY],
    location_len: usize,
}

impl HttpResponse {
    pub fn body(&self) -> &[u8] {
        &self.body[..self.body_len]
    }

    pub fn location(&self) -> Option<&str> {
        (self.location_len != 0).then(|| {
            // Header parsing accepts visible ASCII only.
            unsafe { core::str::from_utf8_unchecked(&self.location[..self.location_len]) }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpUrl<'a> {
    pub scheme: HttpScheme,
    pub host: &'a str,
    pub port: u16,
    pub path: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpScheme {
    Http,
    Https,
}

impl HttpScheme {
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }

    pub const fn is_secure(self) -> bool {
        matches!(self, Self::Https)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PingError {
    BadAddress,
    NoDevice,
    LinkDown,
    ArpTimeout,
    TransmitFailed,
    ReplyTimeout,
}

impl PingError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::BadAddress => "invalid IPv4 address",
            Self::NoDevice => "RTL8139 network device is unavailable",
            Self::LinkDown => "RTL8139 link is down",
            Self::ArpTimeout => "ARP neighbor resolution timed out",
            Self::TransmitFailed => "RTL8139 transmit failed",
            Self::ReplyTimeout => "ICMP echo reply timed out",
        }
    }
}

impl From<PingError> for NetworkError {
    fn from(error: PingError) -> Self {
        match error {
            PingError::BadAddress => Self::BadAddress,
            PingError::NoDevice => Self::NoDevice,
            PingError::LinkDown => Self::LinkDown,
            PingError::ArpTimeout => Self::ArpTimeout,
            PingError::TransmitFailed => Self::TransmitFailed,
            PingError::ReplyTimeout => Self::ReplyTimeout,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PingReply {
    pub address: [u8; 4],
    pub bytes: usize,
    pub sequence: u16,
    pub ttl: u8,
    pub elapsed_cycles: u64,
}

#[repr(C, align(256))]
struct DmaBuffers {
    receive: [u8; RX_BUFFER_SIZE],
    transmit: [[u8; MAX_FRAME_SIZE]; TX_BUFFER_COUNT],
}

impl DmaBuffers {
    const fn new() -> Self {
        Self {
            receive: [0; RX_BUFFER_SIZE],
            transmit: [[0; MAX_FRAME_SIZE]; TX_BUFFER_COUNT],
        }
    }
}

struct NetworkStack {
    initialized: bool,
    io_base: u16,
    pci_bus: u8,
    pci_slot: u8,
    mac: [u8; 6],
    receive_offset: usize,
    transmit_index: usize,
    transmitted_packets: u64,
    received_packets: u64,
    dropped_packets: u64,
    cached_neighbor_ip: [u8; 4],
    cached_neighbor_mac: [u8; 6],
    neighbor_valid: bool,
    next_echo_sequence: u16,
    next_ip_identification: u16,
    next_ephemeral_port: u16,
    next_dns_identifier: u16,
    dma: DmaBuffers,
}

impl NetworkStack {
    const fn new() -> Self {
        Self {
            initialized: false,
            io_base: 0,
            pci_bus: 0,
            pci_slot: 0,
            mac: [0; 6],
            receive_offset: 0,
            transmit_index: 0,
            transmitted_packets: 0,
            received_packets: 0,
            dropped_packets: 0,
            cached_neighbor_ip: [0; 4],
            cached_neighbor_mac: [0; 6],
            neighbor_valid: false,
            next_echo_sequence: 1,
            next_ip_identification: 1,
            next_ephemeral_port: 49_152,
            next_dns_identifier: 1,
            dma: DmaBuffers::new(),
        }
    }

    fn initialize(&mut self) -> bool {
        if self.initialized {
            return true;
        }
        let Some(device) = find_rtl8139() else {
            return false;
        };

        let bar = pci_read(device.bus, device.slot, 0, 0x10);
        if bar & 1 == 0 {
            // This compact driver intentionally supports RTL8139 I/O BARs.
            return false;
        }
        let io_base = (bar & 0xFFFC) as u16;
        if io_base == 0 {
            return false;
        }

        let command_and_status = pci_read(device.bus, device.slot, 0, 0x04);
        // Permit I/O-space accesses and PCI bus-master DMA.
        pci_write(
            device.bus,
            device.slot,
            0,
            0x04,
            command_and_status | 0x0000_0005,
        );

        self.io_base = io_base;
        self.pci_bus = device.bus;
        self.pci_slot = device.slot;

        self.write8(CONFIG1, 0x00);
        self.write8(COMMAND, COMMAND_RESET);
        let mut reset = false;
        for _ in 0..IO_WAIT_LIMIT {
            if self.read8(COMMAND) & COMMAND_RESET == 0 {
                reset = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !reset {
            self.io_base = 0;
            return false;
        }

        for (index, octet) in self.mac.iter_mut().enumerate() {
            *octet = unsafe { port::inb(io_base + IDR0 + index as u16) };
        }
        if self.mac == [0; 6] || self.mac == [0xFF; 6] {
            self.io_base = 0;
            return false;
        }

        self.receive_offset = 0;
        self.transmit_index = 0;
        self.neighbor_valid = false;
        self.dma.receive.fill(0);
        for buffer in &mut self.dma.transmit {
            buffer.fill(0);
        }
        compiler_fence(Ordering::SeqCst);

        let receive_address = self.dma.receive.as_ptr() as usize;
        if receive_address > u32::MAX as usize {
            self.io_base = 0;
            return false;
        }
        self.write32(RBSTART, receive_address as u32);
        for index in 0..TX_BUFFER_COUNT {
            let address = self.dma.transmit[index].as_ptr() as usize;
            if address > u32::MAX as usize {
                self.io_base = 0;
                return false;
            }
            self.write32(TSAD0 + index as u16 * 4, address as u32);
        }

        self.write16(IMR, 0); // polling driver; keep device IRQs masked
        self.write16(ISR, 0xFFFF);
        self.write32(TCR, 0x0300_0000); // standard 96-bit inter-frame gap
        self.write8(COMMAND, COMMAND_RX_ENABLE | COMMAND_TX_ENABLE);
        // 8 KiB ring with device wraparound, unlimited DMA burst,
        // physical-match and broadcast reception. WRAP stays clear because
        // the reader below deliberately handles ring-boundary modulo copies.
        self.write32(RCR, (7 << 13) | (7 << 8) | (1 << 3) | (1 << 1));
        self.write16(CAPR, 0xFFF0);
        compiler_fence(Ordering::SeqCst);

        self.initialized = true;
        true
    }

    fn link_up(&self) -> bool {
        self.initialized && self.read8(MEDIA_STATUS) & (1 << 2) == 0
    }

    fn send_frame(&mut self, frame: &[u8]) -> bool {
        if !self.initialized || frame.len() > MAX_FRAME_SIZE {
            return false;
        }
        let length = frame.len().max(60);
        let index = self.transmit_index;
        let status_register = TSD0 + index as u16 * 4;
        let mut available = false;
        for _ in 0..IO_WAIT_LIMIT {
            if self.read32(status_register) & TX_STATUS_OWN != 0 {
                available = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !available {
            return false;
        }

        let buffer = &mut self.dma.transmit[index];
        buffer[..length].fill(0);
        buffer[..frame.len()].copy_from_slice(frame);
        compiler_fence(Ordering::Release);
        // The threshold field asks the device to wait for 1024 bytes where
        // possible, avoiding needless underflow on real PCI implementations.
        self.write32(status_register, (32 << 16) | length as u32);

        let mut sent = false;
        for _ in 0..IO_WAIT_LIMIT {
            let status = self.read32(status_register);
            if status & TX_STATUS_OK != 0 {
                sent = true;
                break;
            }
            if status & TX_STATUS_ABORTED != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        if sent {
            self.transmit_index = (index + 1) % TX_BUFFER_COUNT;
            self.transmitted_packets = self.transmitted_packets.saturating_add(1);
        }
        sent
    }

    fn receive_frame(&mut self, output: &mut [u8; MAX_FRAME_SIZE]) -> Option<usize> {
        if !self.initialized || self.read8(COMMAND) & COMMAND_RX_BUFFER_EMPTY != 0 {
            return None;
        }
        compiler_fence(Ordering::Acquire);

        let offset = self.receive_offset;
        let status = read_wrapped_u16(&self.dma.receive, offset);
        let device_length = read_wrapped_u16(&self.dma.receive, offset + 2) as usize;
        let valid_length = (4..=MAX_FRAME_SIZE + 4).contains(&device_length);
        let frame_length = device_length.saturating_sub(4);
        let valid = status & 1 != 0 && valid_length && frame_length <= output.len();
        if valid {
            copy_wrapped(&self.dma.receive, offset + 4, &mut output[..frame_length]);
            self.received_packets = self.received_packets.saturating_add(1);
        } else {
            self.dropped_packets = self.dropped_packets.saturating_add(1);
        }

        let advance = if valid_length { device_length + 4 } else { 4 };
        self.receive_offset = (offset + advance + 3) & !3;
        self.receive_offset %= RX_RING_SIZE;
        self.write16(CAPR, self.receive_offset.wrapping_sub(16) as u16);
        self.write16(ISR, 0xFFFF);
        compiler_fence(Ordering::Release);

        valid.then_some(frame_length)
    }

    fn resolve_neighbor(&mut self, destination: [u8; 4]) -> Result<[u8; 6], PingError> {
        let next_hop = if same_subnet(destination, LOCAL_IP, NETMASK) {
            destination
        } else {
            GATEWAY
        };
        if self.neighbor_valid && self.cached_neighbor_ip == next_hop {
            return Ok(self.cached_neighbor_mac);
        }

        let mut request = [0_u8; 42];
        request[0..6].copy_from_slice(&BROADCAST_MAC);
        request[6..12].copy_from_slice(&self.mac);
        request[12..14].copy_from_slice(&0x0806_u16.to_be_bytes());
        request[14..16].copy_from_slice(&1_u16.to_be_bytes()); // Ethernet
        request[16..18].copy_from_slice(&0x0800_u16.to_be_bytes()); // IPv4
        request[18] = 6;
        request[19] = 4;
        request[20..22].copy_from_slice(&1_u16.to_be_bytes()); // request
        request[22..28].copy_from_slice(&self.mac);
        request[28..32].copy_from_slice(&LOCAL_IP);
        request[32..38].fill(0);
        request[38..42].copy_from_slice(&next_hop);
        if !self.send_frame(&request) {
            return Err(PingError::TransmitFailed);
        }

        let mut frame = [0_u8; MAX_FRAME_SIZE];
        for _ in 0..RECEIVE_WAIT_LIMIT {
            if let Some(length) = self.receive_frame(&mut frame) {
                self.answer_local_requests(&frame[..length]);
                if let Some((sender_ip, sender_mac)) = arp_reply(&frame[..length], next_hop) {
                    self.cached_neighbor_ip = sender_ip;
                    self.cached_neighbor_mac = sender_mac;
                    self.neighbor_valid = true;
                    return Ok(sender_mac);
                }
            }
            core::hint::spin_loop();
        }
        Err(PingError::ArpTimeout)
    }

    fn ping(&mut self, destination: [u8; 4]) -> Result<PingReply, PingError> {
        if !self.initialize() {
            return Err(PingError::NoDevice);
        }
        if !self.link_up() {
            return Err(PingError::LinkDown);
        }

        let destination_mac = self.resolve_neighbor(destination)?;
        let sequence = self.next_echo_sequence;
        self.next_echo_sequence = self.next_echo_sequence.wrapping_add(1).max(1);
        let identifier = 0x4858_u16;
        let payload = *b"ExpOS native ICMP";
        let icmp_length = 8 + payload.len();
        let ip_length = 20 + icmp_length;
        let frame_length = ETHERNET_HEADER_SIZE + ip_length;
        let mut frame = [0_u8; MAX_FRAME_SIZE];

        frame[0..6].copy_from_slice(&destination_mac);
        frame[6..12].copy_from_slice(&self.mac);
        frame[12..14].copy_from_slice(&0x0800_u16.to_be_bytes());
        let ip = &mut frame[14..34];
        ip[0] = 0x45;
        ip[1] = 0;
        ip[2..4].copy_from_slice(&(ip_length as u16).to_be_bytes());
        ip[4..6].copy_from_slice(&sequence.to_be_bytes());
        ip[6..8].copy_from_slice(&0x4000_u16.to_be_bytes()); // do not fragment
        ip[8] = 64;
        ip[9] = 1; // ICMP
        ip[10..12].fill(0);
        ip[12..16].copy_from_slice(&LOCAL_IP);
        ip[16..20].copy_from_slice(&destination);
        let ip_checksum = internet_checksum(ip);
        ip[10..12].copy_from_slice(&ip_checksum.to_be_bytes());

        let icmp = &mut frame[34..34 + icmp_length];
        icmp[0] = 8; // echo request
        icmp[1] = 0;
        icmp[2..4].fill(0);
        icmp[4..6].copy_from_slice(&identifier.to_be_bytes());
        icmp[6..8].copy_from_slice(&sequence.to_be_bytes());
        icmp[8..].copy_from_slice(&payload);
        let icmp_checksum = internet_checksum(icmp);
        icmp[2..4].copy_from_slice(&icmp_checksum.to_be_bytes());

        let started = crate::hardware::timestamp();
        if !self.send_frame(&frame[..frame_length]) {
            return Err(PingError::TransmitFailed);
        }

        let mut received = [0_u8; MAX_FRAME_SIZE];
        for _ in 0..RECEIVE_WAIT_LIMIT {
            if let Some(length) = self.receive_frame(&mut received) {
                self.answer_local_requests(&received[..length]);
                if let Some((source, ttl, data_length)) =
                    icmp_echo_reply(&received[..length], destination, identifier, sequence)
                {
                    return Ok(PingReply {
                        address: source,
                        bytes: data_length,
                        sequence,
                        ttl,
                        elapsed_cycles: crate::hardware::timestamp().wrapping_sub(started),
                    });
                }
            }
            core::hint::spin_loop();
        }
        Err(PingError::ReplyTimeout)
    }

    fn prepare_transport(&mut self) -> Result<(), NetworkError> {
        if !self.initialize() {
            return Err(NetworkError::NoDevice);
        }
        if !self.link_up() {
            return Err(NetworkError::LinkDown);
        }
        Ok(())
    }

    fn allocate_ephemeral_port(&mut self) -> u16 {
        let port = self.next_ephemeral_port;
        self.next_ephemeral_port = if port == u16::MAX { 49_152 } else { port + 1 };
        port
    }

    fn allocate_dns_identifier(&mut self) -> u16 {
        let identifier = self.next_dns_identifier;
        self.next_dns_identifier = self.next_dns_identifier.wrapping_add(1).max(1);
        identifier
    }

    fn send_ipv4(
        &mut self,
        destination_mac: [u8; 6],
        destination: [u8; 4],
        protocol: u8,
        payload: &[u8],
    ) -> Result<(), NetworkError> {
        if payload.len() > 1500 - 20 {
            return Err(NetworkError::TransmitFailed);
        }
        let ip_length = 20 + payload.len();
        let frame_length = ETHERNET_HEADER_SIZE + ip_length;
        let mut frame = [0_u8; MAX_FRAME_SIZE];
        frame[0..6].copy_from_slice(&destination_mac);
        frame[6..12].copy_from_slice(&self.mac);
        frame[12..14].copy_from_slice(&0x0800_u16.to_be_bytes());

        let identification = self.next_ip_identification;
        self.next_ip_identification = identification.wrapping_add(1).max(1);
        let ip = &mut frame[ETHERNET_HEADER_SIZE..ETHERNET_HEADER_SIZE + 20];
        build_ipv4_header(
            ip,
            ip_length,
            identification,
            protocol,
            LOCAL_IP,
            destination,
        );
        frame[ETHERNET_HEADER_SIZE + 20..frame_length].copy_from_slice(payload);
        if self.send_frame(&frame[..frame_length]) {
            Ok(())
        } else {
            Err(NetworkError::TransmitFailed)
        }
    }

    fn udp_exchange(
        &mut self,
        destination: [u8; 4],
        destination_port: u16,
        request: &[u8],
        response: &mut [u8],
    ) -> Result<UdpReply, NetworkError> {
        self.prepare_transport()?;
        if destination_port == 0 || request.len() > 1500 - 20 - UDP_HEADER_SIZE {
            return Err(NetworkError::BadAddress);
        }
        let destination_mac = self
            .resolve_neighbor(destination)
            .map_err(NetworkError::from)?;
        let source_port = self.allocate_ephemeral_port();
        let udp_length = UDP_HEADER_SIZE + request.len();
        let mut segment = [0_u8; 1500 - 20];
        segment[0..2].copy_from_slice(&source_port.to_be_bytes());
        segment[2..4].copy_from_slice(&destination_port.to_be_bytes());
        segment[4..6].copy_from_slice(&(udp_length as u16).to_be_bytes());
        segment[6..8].fill(0);
        segment[8..udp_length].copy_from_slice(request);
        let checksum = transport_checksum(LOCAL_IP, destination, 17, &segment[..udp_length]);
        segment[6..8].copy_from_slice(&nonzero_checksum(checksum).to_be_bytes());

        let mut frame = [0_u8; MAX_FRAME_SIZE];
        for _ in 0..TRANSPORT_RETRIES {
            self.send_ipv4(destination_mac, destination, 17, &segment[..udp_length])?;
            for _ in 0..RECEIVE_WAIT_LIMIT {
                if let Some(length) = self.receive_frame(&mut frame) {
                    self.answer_local_requests(&frame[..length]);
                    let Some(packet) = parse_udp_packet(
                        &frame[..length],
                        destination,
                        destination_port,
                        source_port,
                    ) else {
                        continue;
                    };
                    let copied = packet.payload.len().min(response.len());
                    response[..copied].copy_from_slice(&packet.payload[..copied]);
                    return Ok(UdpReply {
                        source: packet.source,
                        source_port: packet.source_port,
                        bytes: copied,
                    });
                }
                core::hint::spin_loop();
            }
        }
        Err(NetworkError::ReplyTimeout)
    }

    fn dns_lookup(&mut self, hostname: &str) -> Result<[u8; 4], NetworkError> {
        self.prepare_transport()?;
        if let Some(address) = parse_ipv4(hostname) {
            return Ok(address);
        }
        validate_hostname(hostname)?;
        let identifier = self.allocate_dns_identifier();
        let mut request = [0_u8; DNS_PACKET_CAPACITY];
        let request_length = build_dns_query(identifier, hostname, &mut request)?;
        let mut response = [0_u8; DNS_PACKET_CAPACITY];
        match self.udp_exchange(DNS_SERVER, 53, &request[..request_length], &mut response) {
            Ok(reply) => parse_dns_a_response(identifier, &response[..reply.bytes]),
            Err(NetworkError::ReplyTimeout) => Err(NetworkError::DnsTimeout),
            Err(error) => Err(error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn send_tcp_segment(
        &mut self,
        destination_mac: [u8; 6],
        destination: [u8; 4],
        source_port: u16,
        destination_port: u16,
        sequence: u32,
        acknowledgement: u32,
        flags: u8,
        payload: &[u8],
    ) -> Result<(), NetworkError> {
        let syn = flags & TCP_SYN != 0;
        let header_length = if syn {
            TCP_SYN_HEADER_SIZE
        } else {
            TCP_HEADER_SIZE
        };
        if payload.len() > 1500 - 20 - header_length {
            return Err(NetworkError::TransmitFailed);
        }
        let length = header_length + payload.len();
        let mut segment = [0_u8; 1500 - 20];
        segment[0..2].copy_from_slice(&source_port.to_be_bytes());
        segment[2..4].copy_from_slice(&destination_port.to_be_bytes());
        segment[4..8].copy_from_slice(&sequence.to_be_bytes());
        segment[8..12].copy_from_slice(&acknowledgement.to_be_bytes());
        segment[12] = ((header_length / 4) as u8) << 4;
        segment[13] = flags;
        segment[14..16].copy_from_slice(&TCP_RECEIVE_WINDOW.to_be_bytes());
        segment[16..20].fill(0);
        if syn {
            // MSS 1460.  Avoiding window scaling and timestamps keeps this
            // one-connection client deterministic and easy to audit.
            segment[20..24].copy_from_slice(&[2, 4, 0x05, 0xB4]);
        }
        segment[header_length..length].copy_from_slice(payload);
        let checksum = transport_checksum(LOCAL_IP, destination, 6, &segment[..length]);
        segment[16..18].copy_from_slice(&checksum.to_be_bytes());
        self.send_ipv4(destination_mac, destination, 6, &segment[..length])
    }

    fn tcp_connect(
        &mut self,
        destination: [u8; 4],
        destination_port: u16,
    ) -> Result<TcpConnection<'_>, NetworkError> {
        self.prepare_transport()?;
        if destination_port == 0 {
            return Err(NetworkError::BadAddress);
        }
        let destination_mac = self
            .resolve_neighbor(destination)
            .map_err(NetworkError::from)?;
        let source_port = self.allocate_ephemeral_port();
        let initial_sequence = (crate::hardware::timestamp() as u32)
            .wrapping_add((source_port as u32) << 16)
            .max(1);
        let mut frame = [0_u8; MAX_FRAME_SIZE];
        let mut remote_next = None;

        for _ in 0..TRANSPORT_RETRIES {
            self.send_tcp_segment(
                destination_mac,
                destination,
                source_port,
                destination_port,
                initial_sequence,
                0,
                TCP_SYN,
                &[],
            )?;
            for _ in 0..RECEIVE_WAIT_LIMIT {
                if let Some(length) = self.receive_frame(&mut frame) {
                    self.answer_local_requests(&frame[..length]);
                    let Some(packet) = parse_tcp_packet(
                        &frame[..length],
                        destination,
                        destination_port,
                        source_port,
                    ) else {
                        continue;
                    };
                    if packet.flags & TCP_RST != 0 {
                        return Err(NetworkError::TcpReset);
                    }
                    if packet.flags & (TCP_SYN | TCP_ACK) == (TCP_SYN | TCP_ACK)
                        && packet.acknowledgement == initial_sequence.wrapping_add(1)
                    {
                        remote_next = Some(packet.sequence.wrapping_add(1));
                        break;
                    }
                }
                core::hint::spin_loop();
            }
            if remote_next.is_some() {
                break;
            }
        }

        let remote_next = remote_next.ok_or(NetworkError::TcpTimeout)?;
        let local_next = initial_sequence.wrapping_add(1);
        self.send_tcp_segment(
            destination_mac,
            destination,
            source_port,
            destination_port,
            local_next,
            remote_next,
            TCP_ACK,
            &[],
        )?;
        Ok(TcpConnection {
            stack: self,
            destination_mac,
            destination,
            source_port,
            destination_port,
            local_next,
            remote_next,
            pending: [0; TCP_PAYLOAD_CAPACITY],
            pending_offset: 0,
            pending_length: 0,
            remote_closed: false,
            local_closed: false,
        })
    }

    fn tcp_exchange(
        &mut self,
        destination: [u8; 4],
        destination_port: u16,
        request: &[u8],
        output: &mut [u8],
    ) -> Result<(usize, bool), NetworkError> {
        self.prepare_transport()?;
        if destination_port == 0 || request.len() > 1500 - 20 - TCP_HEADER_SIZE {
            return Err(NetworkError::BadAddress);
        }
        let destination_mac = self
            .resolve_neighbor(destination)
            .map_err(NetworkError::from)?;
        let source_port = self.allocate_ephemeral_port();
        let initial_sequence = (crate::hardware::timestamp() as u32)
            .wrapping_add((source_port as u32) << 16)
            .max(1);
        let mut frame = [0_u8; MAX_FRAME_SIZE];
        let mut remote_next = None;

        for _ in 0..TRANSPORT_RETRIES {
            self.send_tcp_segment(
                destination_mac,
                destination,
                source_port,
                destination_port,
                initial_sequence,
                0,
                TCP_SYN,
                &[],
            )?;
            for _ in 0..RECEIVE_WAIT_LIMIT {
                if let Some(length) = self.receive_frame(&mut frame) {
                    self.answer_local_requests(&frame[..length]);
                    let Some(packet) = parse_tcp_packet(
                        &frame[..length],
                        destination,
                        destination_port,
                        source_port,
                    ) else {
                        continue;
                    };
                    if packet.flags & TCP_RST != 0 {
                        return Err(NetworkError::TcpReset);
                    }
                    if packet.flags & (TCP_SYN | TCP_ACK) == (TCP_SYN | TCP_ACK)
                        && packet.acknowledgement == initial_sequence.wrapping_add(1)
                    {
                        remote_next = Some(packet.sequence.wrapping_add(1));
                        break;
                    }
                }
                core::hint::spin_loop();
            }
            if remote_next.is_some() {
                break;
            }
        }
        let mut remote_next = remote_next.ok_or(NetworkError::TcpTimeout)?;
        let request_sequence = initial_sequence.wrapping_add(1);
        let mut local_next = request_sequence;
        self.send_tcp_segment(
            destination_mac,
            destination,
            source_port,
            destination_port,
            local_next,
            remote_next,
            TCP_ACK,
            &[],
        )?;
        self.send_tcp_segment(
            destination_mac,
            destination,
            source_port,
            destination_port,
            local_next,
            remote_next,
            TCP_ACK | TCP_PSH,
            request,
        )?;
        local_next = local_next.wrapping_add(request.len() as u32);

        let mut output_length = 0;
        let mut truncated = false;
        let mut peer_acked_request = false;
        let mut idle = 0;
        let mut retries = 0;
        let mut remote_closed = false;
        while !remote_closed && !truncated {
            let Some(length) = self.receive_frame(&mut frame) else {
                idle += 1;
                if idle < RECEIVE_WAIT_LIMIT {
                    core::hint::spin_loop();
                    continue;
                }
                idle = 0;
                if retries + 1 >= TRANSPORT_RETRIES {
                    return Err(NetworkError::TcpTimeout);
                }
                retries += 1;
                if peer_acked_request || output_length != 0 {
                    self.send_tcp_segment(
                        destination_mac,
                        destination,
                        source_port,
                        destination_port,
                        local_next,
                        remote_next,
                        TCP_ACK,
                        &[],
                    )?;
                } else {
                    self.send_tcp_segment(
                        destination_mac,
                        destination,
                        source_port,
                        destination_port,
                        request_sequence,
                        remote_next,
                        TCP_ACK | TCP_PSH,
                        request,
                    )?;
                }
                continue;
            };
            self.answer_local_requests(&frame[..length]);
            let Some(packet) =
                parse_tcp_packet(&frame[..length], destination, destination_port, source_port)
            else {
                continue;
            };
            idle = 0;
            if packet.flags & TCP_RST != 0 {
                return Err(NetworkError::TcpReset);
            }
            if packet.flags & TCP_ACK != 0 && packet.acknowledgement == local_next {
                peer_acked_request = true;
            }

            if packet.sequence == remote_next {
                if !packet.payload.is_empty() {
                    let remaining = output.len().saturating_sub(output_length);
                    let copied = packet.payload.len().min(remaining);
                    output[output_length..output_length + copied]
                        .copy_from_slice(&packet.payload[..copied]);
                    output_length += copied;
                    remote_next = remote_next.wrapping_add(packet.payload.len() as u32);
                    truncated = copied != packet.payload.len();
                }
                if packet.flags & TCP_FIN != 0 {
                    remote_next = remote_next.wrapping_add(1);
                    remote_closed = true;
                }
            }
            if !packet.payload.is_empty() || packet.flags & TCP_FIN != 0 {
                self.send_tcp_segment(
                    destination_mac,
                    destination,
                    source_port,
                    destination_port,
                    local_next,
                    remote_next,
                    TCP_ACK,
                    &[],
                )?;
            }
        }

        // Active close after a complete peer FIN, or stop a response that hit
        // the fixed wire bound.  The final ACK is best effort: the response is
        // already complete from the caller's perspective.
        let _ = self.send_tcp_segment(
            destination_mac,
            destination,
            source_port,
            destination_port,
            local_next,
            remote_next,
            TCP_FIN | TCP_ACK,
            &[],
        );
        Ok((output_length, truncated))
    }

    fn answer_local_requests(&mut self, frame: &[u8]) {
        if frame.len() < 42 || frame[12..14] != [0x08, 0x06] {
            return;
        }
        if frame[20..22] != [0, 1] || frame[38..42] != LOCAL_IP {
            return;
        }
        let mut reply = [0_u8; 42];
        reply[0..6].copy_from_slice(&frame[22..28]);
        reply[6..12].copy_from_slice(&self.mac);
        reply[12..14].copy_from_slice(&[0x08, 0x06]);
        reply[14..20].copy_from_slice(&frame[14..20]);
        reply[20..22].copy_from_slice(&2_u16.to_be_bytes());
        reply[22..28].copy_from_slice(&self.mac);
        reply[28..32].copy_from_slice(&LOCAL_IP);
        reply[32..38].copy_from_slice(&frame[22..28]);
        reply[38..42].copy_from_slice(&frame[28..32]);
        let _ = self.send_frame(&reply);
    }

    fn read8(&self, register: u16) -> u8 {
        unsafe { port::inb(self.io_base + register) }
    }

    fn read32(&self, register: u16) -> u32 {
        unsafe { port::inl(self.io_base + register) }
    }

    fn write8(&self, register: u16, value: u8) {
        unsafe { port::outb(self.io_base + register, value) }
    }

    fn write16(&self, register: u16, value: u16) {
        unsafe { port::outw(self.io_base + register, value) }
    }

    fn write32(&self, register: u16, value: u32) {
        unsafe { port::outl(self.io_base + register, value) }
    }
}

/// One established, in-order TCP stream backed by the polling RTL8139 stack.
///
/// ExpOS currently serializes network clients behind `NETWORK`, so the stream
/// borrows the stack exclusively for its lifetime. That is enough for TLS and
/// HTTP while making the transport a real byte stream instead of a single
/// request/response shortcut.
struct TcpConnection<'a> {
    stack: &'a mut NetworkStack,
    destination_mac: [u8; 6],
    destination: [u8; 4],
    source_port: u16,
    destination_port: u16,
    local_next: u32,
    remote_next: u32,
    pending: [u8; TCP_PAYLOAD_CAPACITY],
    pending_offset: usize,
    pending_length: usize,
    remote_closed: bool,
    local_closed: bool,
}

impl TcpConnection<'_> {
    fn send_segment(
        &mut self,
        sequence: u32,
        flags: u8,
        payload: &[u8],
    ) -> Result<(), NetworkError> {
        self.stack.send_tcp_segment(
            self.destination_mac,
            self.destination,
            self.source_port,
            self.destination_port,
            sequence,
            self.remote_next,
            flags,
            payload,
        )
    }

    fn send_ack(&mut self) -> Result<(), NetworkError> {
        self.send_segment(self.local_next, TCP_ACK, &[])
    }

    fn drain_pending(&mut self, output: &mut [u8]) -> usize {
        let available = self.pending_length.saturating_sub(self.pending_offset);
        let copied = available.min(output.len());
        if copied != 0 {
            output[..copied]
                .copy_from_slice(&self.pending[self.pending_offset..self.pending_offset + copied]);
            self.pending_offset += copied;
        }
        if self.pending_offset == self.pending_length {
            self.pending_offset = 0;
            self.pending_length = 0;
        }
        copied
    }

    fn receive_into_pending(&mut self, frame: &[u8]) -> Result<(bool, bool), NetworkError> {
        let Some(packet) = parse_tcp_packet(
            frame,
            self.destination,
            self.destination_port,
            self.source_port,
        ) else {
            return Ok((false, false));
        };
        if packet.flags & TCP_RST != 0 {
            return Err(NetworkError::TcpReset);
        }

        let acknowledges_local =
            packet.flags & TCP_ACK != 0 && packet.acknowledgement == self.local_next;
        let mut accepted_data = false;
        if packet.sequence == self.remote_next {
            if !packet.payload.is_empty() && self.pending_length == 0 {
                let copied = packet.payload.len().min(self.pending.len());
                self.pending[..copied].copy_from_slice(&packet.payload[..copied]);
                self.pending_offset = 0;
                self.pending_length = copied;
                if copied != packet.payload.len() {
                    return Err(NetworkError::TlsProtocol);
                }
                self.remote_next = self.remote_next.wrapping_add(packet.payload.len() as u32);
                accepted_data = true;
            }
            if packet.flags & TCP_FIN != 0 {
                self.remote_next = self.remote_next.wrapping_add(1);
                self.remote_closed = true;
                accepted_data = true;
            }
        }
        if accepted_data || (!packet.payload.is_empty() && packet.sequence != self.remote_next) {
            self.send_ack()?;
        }
        Ok((true, acknowledges_local))
    }

    fn write_stream(&mut self, input: &[u8]) -> Result<usize, NetworkError> {
        if self.local_closed || self.remote_closed {
            return Err(NetworkError::TcpReset);
        }
        if input.is_empty() {
            return Ok(0);
        }
        let written = input.len().min(TCP_PAYLOAD_CAPACITY);
        let payload = &input[..written];
        let sequence = self.local_next;
        let expected_ack = sequence.wrapping_add(written as u32);
        let mut frame = [0_u8; MAX_FRAME_SIZE];

        for _ in 0..TRANSPORT_RETRIES {
            self.send_segment(sequence, TCP_ACK | TCP_PSH, payload)?;
            for _ in 0..RECEIVE_WAIT_LIMIT {
                let Some(length) = self.stack.receive_frame(&mut frame) else {
                    core::hint::spin_loop();
                    continue;
                };
                self.stack.answer_local_requests(&frame[..length]);
                let acknowledgement = parse_tcp_packet(
                    &frame[..length],
                    self.destination,
                    self.destination_port,
                    self.source_port,
                )
                .filter(|packet| packet.flags & TCP_ACK != 0)
                .map(|packet| packet.acknowledgement);

                // Install the sequence before processing a data-bearing ACK,
                // because an immediate response must be acknowledged with the
                // next local sequence number.
                if acknowledgement == Some(expected_ack) {
                    self.local_next = expected_ack;
                }
                let (_, acknowledged) = self.receive_into_pending(&frame[..length])?;
                if acknowledged || acknowledgement == Some(expected_ack) {
                    self.local_next = expected_ack;
                    return Ok(written);
                }
            }
        }
        Err(NetworkError::TcpTimeout)
    }

    fn read_stream(&mut self, output: &mut [u8]) -> Result<usize, NetworkError> {
        if output.is_empty() {
            return Ok(0);
        }
        let copied = self.drain_pending(output);
        if copied != 0 {
            return Ok(copied);
        }
        if self.remote_closed {
            return Ok(0);
        }

        let mut frame = [0_u8; MAX_FRAME_SIZE];
        for retry in 0..TRANSPORT_RETRIES {
            for _ in 0..RECEIVE_WAIT_LIMIT {
                let Some(length) = self.stack.receive_frame(&mut frame) else {
                    core::hint::spin_loop();
                    continue;
                };
                self.stack.answer_local_requests(&frame[..length]);
                self.receive_into_pending(&frame[..length])?;
                let copied = self.drain_pending(output);
                if copied != 0 {
                    return Ok(copied);
                }
                if self.remote_closed {
                    return Ok(0);
                }
            }
            if retry + 1 < TRANSPORT_RETRIES {
                // A duplicate ACK prompts a peer with missing data to
                // retransmit without fabricating any application bytes.
                self.send_ack()?;
            }
        }
        Err(NetworkError::TcpTimeout)
    }

    fn close_stream(&mut self) {
        if self.local_closed {
            return;
        }
        let _ = self.send_segment(self.local_next, TCP_FIN | TCP_ACK, &[]);
        self.local_next = self.local_next.wrapping_add(1);
        self.local_closed = true;
    }
}

impl Drop for TcpConnection<'_> {
    fn drop(&mut self) {
        self.close_stream();
    }
}

impl embedded_io::Error for NetworkError {
    fn kind(&self) -> embedded_io::ErrorKind {
        match self {
            Self::TcpTimeout | Self::ReplyTimeout | Self::DnsTimeout => {
                embedded_io::ErrorKind::TimedOut
            }
            Self::TcpReset => embedded_io::ErrorKind::ConnectionReset,
            Self::BadAddress | Self::BadHostname | Self::BadUrl | Self::UnsupportedScheme => {
                embedded_io::ErrorKind::InvalidInput
            }
            _ => embedded_io::ErrorKind::Other,
        }
    }
}

impl embedded_io::ErrorType for TcpConnection<'_> {
    type Error = NetworkError;
}

impl embedded_io::Read for TcpConnection<'_> {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, Self::Error> {
        self.read_stream(output)
    }
}

impl embedded_io::Write for TcpConnection<'_> {
    fn write(&mut self, input: &[u8]) -> Result<usize, Self::Error> {
        self.write_stream(input)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

static NETWORK: crate::sync::SpinMutex<NetworkStack> =
    crate::sync::SpinMutex::new(NetworkStack::new());

pub fn initialize() -> bool {
    let mut network = NETWORK.lock();
    let ready = network.initialize();
    if ready {
        let mac = network.mac;
        slog!(
            "HEXA_NET_READY driver=rtl8139 mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\r\n",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        );
    } else {
        slog!("HEXA_NET_UNAVAILABLE driver=rtl8139\r\n");
    }
    ready
}

pub fn available() -> bool {
    NETWORK.lock().initialized
}

/// Report the physical Ethernet carrier independently of policy/capabilities.
pub fn link_up() -> bool {
    NETWORK.lock().link_up()
}

pub fn print_configuration() {
    let network = NETWORK.lock();
    if !network.initialized {
        println!("ether0: unavailable (no supported RTL8139 PCI device)");
        println!("supported virtual NIC: Realtek RTL8139 (10ec:8139)");
        return;
    }
    println!("NAME    STATE  ADDRESS       NETMASK         GATEWAY");
    println!(
        "ether0  {:<5}  {}.{}.{}.{}   {}.{}.{}.{}  {}.{}.{}.{}",
        if network.link_up() { "up" } else { "down" },
        LOCAL_IP[0],
        LOCAL_IP[1],
        LOCAL_IP[2],
        LOCAL_IP[3],
        NETMASK[0],
        NETMASK[1],
        NETMASK[2],
        NETMASK[3],
        GATEWAY[0],
        GATEWAY[1],
        GATEWAY[2],
        GATEWAY[3]
    );
    println!(
        "        mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} mtu=1500 driver=rtl8139-poll",
        network.mac[0],
        network.mac[1],
        network.mac[2],
        network.mac[3],
        network.mac[4],
        network.mac[5]
    );
    println!("policy: restricted; network I/O requires Power or Operator authority");
}

pub fn print_statistics() {
    let network = NETWORK.lock();
    if !network.initialized {
        println!("network driver unavailable; no packet statistics");
        return;
    }
    println!("DRIVER       LINK  TX-PACKETS  RX-PACKETS  DROPPED");
    println!(
        "rtl8139-poll {:<4}  {:<10}  {:<10}  {}",
        if network.link_up() { "up" } else { "down" },
        network.transmitted_packets,
        network.received_packets,
        network.dropped_packets
    );
    println!("transport: Ethernet + ARP + static IPv4 + ICMP + UDP/DNS + TCP");
    println!("application: HTTP/1.0 + verified TLS 1.3 HTTPS");
    println!("dhcp/ipv6: not implemented");
}

pub fn ping_text(
    broker: &CapabilityBroker,
    handle_id: u32,
    requester: Fin,
    dimension: Fin,
    arguments: &str,
) {
    if !crate::radio::network_allowed() {
        println!("ping: network access is disabled in Settings");
        slog!("HEXA_PING_DENIED policy=disabled\r\n");
        return;
    }
    if let Err(error) = broker.authorize_requester(
        handle_id,
        requester,
        NETWORK_FIN,
        dimension,
        Operations::NETWORK,
        crate::hardware::timestamp(),
    ) {
        println!("ping: DIESE denied a Network Handle: {:?}", error);
        slog!(
            "HEXA_PING_DENIED handle={} error={:?}\r\n",
            handle_id,
            error
        );
        return;
    }
    let mut words = arguments.split_ascii_whitespace();
    let Some(target) = words.next() else {
        println!("usage: ping <IPv4-address> [count]");
        return;
    };
    let Some(address) = parse_ipv4(target) else {
        println!("ping: {}", PingError::BadAddress.message());
        println!("usage: ping <IPv4-address> [count]  example: ping 10.0.2.2 4");
        return;
    };
    let count = match words.next() {
        Some(raw) => match raw.parse::<u16>() {
            Ok(value @ 1..=200) => value,
            _ => {
                println!("ping: count must be between 1 and 200");
                return;
            }
        },
        None => 1,
    };
    if words.next().is_some() {
        println!("usage: ping <IPv4-address> [count]");
        return;
    }
    println!(
        "PING {}.{}.{}.{} via ether0: {} ICMP echo request(s)",
        address[0], address[1], address[2], address[3], count
    );
    let mut received = 0_u16;
    for _ in 0..count {
        match NETWORK.lock().ping(address) {
            Ok(reply) => {
                received += 1;
                println!(
                    "{} bytes from {}.{}.{}.{}: icmp_seq={} ttl={} time={} TSC cycles",
                    reply.bytes,
                    reply.address[0],
                    reply.address[1],
                    reply.address[2],
                    reply.address[3],
                    reply.sequence,
                    reply.ttl,
                    reply.elapsed_cycles
                );
                slog!(
                    "HEXA_PING_REPLY address={}.{}.{}.{} sequence={}\r\n",
                    reply.address[0],
                    reply.address[1],
                    reply.address[2],
                    reply.address[3],
                    reply.sequence
                );
            }
            Err(error) => {
                println!("ping: {}", error.message());
                slog!("HEXA_PING_ERROR {:?}\r\n", error);
                break;
            }
        }
    }
    println!("ping summary: sent={} received={}", count, received);
    slog!("HEXA_PING_SUMMARY sent={} received={}\r\n", count, received);
}

fn authorize_network(
    broker: &CapabilityBroker,
    handle_id: u32,
    requester: Fin,
    dimension: Fin,
) -> Result<(), NetworkError> {
    if !crate::radio::network_allowed() {
        return Err(NetworkError::PolicyDisabled);
    }
    broker
        .authorize_requester(
            handle_id,
            requester,
            NETWORK_FIN,
            dimension,
            Operations::NETWORK,
            crate::hardware::timestamp(),
        )
        .map_err(|_| NetworkError::CapabilityDenied)
}

/// Send one UDP datagram and wait for a reply from the same address and port.
/// The caller owns the response buffer; excess reply bytes are deliberately
/// discarded and `UdpReply::bytes` reports the copied length.
#[allow(dead_code, clippy::too_many_arguments)]
pub fn udp_exchange(
    broker: &CapabilityBroker,
    handle_id: u32,
    requester: Fin,
    dimension: Fin,
    destination: [u8; 4],
    destination_port: u16,
    request: &[u8],
    response: &mut [u8],
) -> Result<UdpReply, NetworkError> {
    authorize_network(broker, handle_id, requester, dimension)?;
    NETWORK
        .lock()
        .udp_exchange(destination, destination_port, request, response)
}

/// Resolve one IPv4 address with the QEMU/SLiRP DNS service at `10.0.2.3`.
pub fn dns_lookup(
    broker: &CapabilityBroker,
    handle_id: u32,
    requester: Fin,
    dimension: Fin,
    hostname: &str,
) -> Result<[u8; 4], NetworkError> {
    authorize_network(broker, handle_id, requester, dimension)?;
    NETWORK.lock().dns_lookup(hostname)
}

/// Parse the intentionally narrow URL syntax accepted by the native client.
/// Both `http://host[:port]/path` and `https://host[:port]/path` are accepted;
/// certificate validation still requires a DNS hostname for secure URLs.
pub fn parse_http_url(url: &str) -> Result<HttpUrl<'_>, NetworkError> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (HttpScheme::Https, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (HttpScheme::Http, rest)
    } else {
        return Err(NetworkError::UnsupportedScheme);
    };
    let without_fragment = rest.split_once('#').map_or(rest, |(before, _)| before);
    let (authority, path) = match without_fragment.find('/') {
        Some(index) => (&without_fragment[..index], &without_fragment[index..]),
        None => (without_fragment, "/"),
    };
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains('[')
        || authority.contains(']')
    {
        return Err(NetworkError::BadUrl);
    }
    let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
        if host.contains(':') || port.is_empty() {
            return Err(NetworkError::BadUrl);
        }
        let port = port.parse::<u16>().map_err(|_| NetworkError::BadUrl)?;
        if port == 0 {
            return Err(NetworkError::BadUrl);
        }
        (host, port)
    } else {
        (authority, scheme.default_port())
    };
    if scheme.is_secure() && parse_ipv4(host).is_some() {
        return Err(NetworkError::BadHostname);
    }
    if parse_ipv4(host).is_none() {
        validate_hostname(host)?;
    }
    if path.is_empty()
        || !path.starts_with('/')
        || !path.bytes().all(|byte| (0x21..=0x7E).contains(&byte))
    {
        return Err(NetworkError::BadUrl);
    }
    Ok(HttpUrl {
        scheme,
        host,
        port,
        path,
    })
}

/// Perform a bounded HTTP/1.0 GET.  Capability authorization happens before
/// the NIC lock, DNS query, or TCP packet transmission.
pub fn http_get(
    broker: &CapabilityBroker,
    handle_id: u32,
    requester: Fin,
    dimension: Fin,
    url: &str,
) -> Result<HttpResponse, NetworkError> {
    authorize_network(broker, handle_id, requester, dimension)?;
    let parsed = parse_http_url(url)?;
    let mut request = [0_u8; HTTP_REQUEST_CAPACITY];
    let mut request_length = 0;
    append_bytes(&mut request, &mut request_length, b"GET ")?;
    append_bytes(&mut request, &mut request_length, parsed.path.as_bytes())?;
    append_bytes(&mut request, &mut request_length, b" HTTP/1.0\r\nHost: ")?;
    append_bytes(&mut request, &mut request_length, parsed.host.as_bytes())?;
    if parsed.port != parsed.scheme.default_port() {
        append_bytes(&mut request, &mut request_length, b":")?;
        append_decimal_u16(&mut request, &mut request_length, parsed.port)?;
    }
    append_bytes(
        &mut request,
        &mut request_length,
        b"\r\nUser-Agent: ExpOS/8\r\nAccept: text/html,text/plain,*/*;q=0.1\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
    )?;

    let mut network = NETWORK.lock();
    let peer = network.dns_lookup(parsed.host)?;
    let mut wire = [0_u8; HTTP_WIRE_CAPACITY];
    let (wire_length, wire_truncated) = if parsed.scheme.is_secure() {
        let stream = network.tcp_connect(peer, parsed.port)?;
        crate::tls::https_exchange(stream, parsed.host, &request[..request_length], &mut wire)?
    } else {
        network.tcp_exchange(peer, parsed.port, &request[..request_length], &mut wire)?
    };
    parse_http_response(peer, &wire[..wire_length], wire_truncated)
}

fn append_bytes(output: &mut [u8], length: &mut usize, bytes: &[u8]) -> Result<(), NetworkError> {
    let end = length
        .checked_add(bytes.len())
        .filter(|end| *end <= output.len())
        .ok_or(NetworkError::BadUrl)?;
    output[*length..end].copy_from_slice(bytes);
    *length = end;
    Ok(())
}

fn append_decimal_u16(
    output: &mut [u8],
    length: &mut usize,
    value: u16,
) -> Result<(), NetworkError> {
    let mut digits = [0_u8; 5];
    let mut cursor = digits.len();
    let mut remaining = value;
    loop {
        cursor -= 1;
        digits[cursor] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    append_bytes(output, length, &digits[cursor..])
}

fn build_ipv4_header(
    output: &mut [u8],
    total_length: usize,
    identification: u16,
    protocol: u8,
    source: [u8; 4],
    destination: [u8; 4],
) {
    debug_assert!(output.len() >= 20);
    output[..20].fill(0);
    output[0] = 0x45;
    output[2..4].copy_from_slice(&(total_length as u16).to_be_bytes());
    output[4..6].copy_from_slice(&identification.to_be_bytes());
    output[6..8].copy_from_slice(&0x4000_u16.to_be_bytes());
    output[8] = 64;
    output[9] = protocol;
    output[12..16].copy_from_slice(&source);
    output[16..20].copy_from_slice(&destination);
    let checksum = internet_checksum(&output[..20]);
    output[10..12].copy_from_slice(&checksum.to_be_bytes());
}

fn nonzero_checksum(checksum: u16) -> u16 {
    if checksum == 0 {
        u16::MAX
    } else {
        checksum
    }
}

fn checksum_sum(bytes: &[u8]) -> u32 {
    let mut sum = 0_u32;
    let mut index = 0;
    while index + 1 < bytes.len() {
        sum += u16::from_be_bytes([bytes[index], bytes[index + 1]]) as u32;
        index += 2;
    }
    if index < bytes.len() {
        sum += (bytes[index] as u32) << 8;
    }
    sum
}

fn finish_checksum(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn transport_checksum(source: [u8; 4], destination: [u8; 4], protocol: u8, segment: &[u8]) -> u16 {
    let pseudo_header = [
        source[0],
        source[1],
        source[2],
        source[3],
        destination[0],
        destination[1],
        destination[2],
        destination[3],
        0,
        protocol,
        (segment.len() >> 8) as u8,
        segment.len() as u8,
    ];
    finish_checksum(checksum_sum(&pseudo_header) + checksum_sum(segment))
}

struct Ipv4Packet<'a> {
    source: [u8; 4],
    destination: [u8; 4],
    protocol: u8,
    payload: &'a [u8],
}

fn parse_ipv4_packet(frame: &[u8]) -> Option<Ipv4Packet<'_>> {
    if frame.len() < ETHERNET_HEADER_SIZE + 20 || frame[12..14] != [0x08, 0] {
        return None;
    }
    let ip = &frame[ETHERNET_HEADER_SIZE..];
    if ip[0] >> 4 != 4 {
        return None;
    }
    let header_length = ((ip[0] & 0x0F) as usize) * 4;
    if header_length < 20 || ip.len() < header_length {
        return None;
    }
    let total_length = u16::from_be_bytes([ip[2], ip[3]]) as usize;
    let fragment = u16::from_be_bytes([ip[6], ip[7]]);
    if total_length < header_length
        || total_length > ip.len()
        || fragment & 0x3FFF != 0
        || internet_checksum(&ip[..header_length]) != 0
    {
        return None;
    }
    let mut source = [0_u8; 4];
    source.copy_from_slice(&ip[12..16]);
    let mut destination = [0_u8; 4];
    destination.copy_from_slice(&ip[16..20]);
    if destination != LOCAL_IP {
        return None;
    }
    Some(Ipv4Packet {
        source,
        destination,
        protocol: ip[9],
        payload: &ip[header_length..total_length],
    })
}

struct ParsedUdp<'a> {
    source: [u8; 4],
    source_port: u16,
    payload: &'a [u8],
}

fn parse_udp_packet<'a>(
    frame: &'a [u8],
    expected_source: [u8; 4],
    expected_source_port: u16,
    expected_destination_port: u16,
) -> Option<ParsedUdp<'a>> {
    let ip = parse_ipv4_packet(frame)?;
    if ip.protocol != 17 || ip.source != expected_source || ip.payload.len() < UDP_HEADER_SIZE {
        return None;
    }
    let source_port = u16::from_be_bytes([ip.payload[0], ip.payload[1]]);
    let destination_port = u16::from_be_bytes([ip.payload[2], ip.payload[3]]);
    let length = u16::from_be_bytes([ip.payload[4], ip.payload[5]]) as usize;
    if source_port != expected_source_port
        || destination_port != expected_destination_port
        || length < UDP_HEADER_SIZE
        || length > ip.payload.len()
    {
        return None;
    }
    let datagram = &ip.payload[..length];
    let received_checksum = u16::from_be_bytes([datagram[6], datagram[7]]);
    if received_checksum != 0 && transport_checksum(ip.source, ip.destination, 17, datagram) != 0 {
        return None;
    }
    Some(ParsedUdp {
        source: ip.source,
        source_port,
        payload: &datagram[UDP_HEADER_SIZE..],
    })
}

struct ParsedTcp<'a> {
    sequence: u32,
    acknowledgement: u32,
    flags: u8,
    payload: &'a [u8],
}

fn parse_tcp_packet<'a>(
    frame: &'a [u8],
    expected_source: [u8; 4],
    expected_source_port: u16,
    expected_destination_port: u16,
) -> Option<ParsedTcp<'a>> {
    let ip = parse_ipv4_packet(frame)?;
    if ip.protocol != 6 || ip.source != expected_source || ip.payload.len() < TCP_HEADER_SIZE {
        return None;
    }
    let source_port = u16::from_be_bytes([ip.payload[0], ip.payload[1]]);
    let destination_port = u16::from_be_bytes([ip.payload[2], ip.payload[3]]);
    let header_length = ((ip.payload[12] >> 4) as usize) * 4;
    if source_port != expected_source_port
        || destination_port != expected_destination_port
        || header_length < TCP_HEADER_SIZE
        || header_length > ip.payload.len()
        || transport_checksum(ip.source, ip.destination, 6, ip.payload) != 0
    {
        return None;
    }
    Some(ParsedTcp {
        sequence: u32::from_be_bytes([ip.payload[4], ip.payload[5], ip.payload[6], ip.payload[7]]),
        acknowledgement: u32::from_be_bytes([
            ip.payload[8],
            ip.payload[9],
            ip.payload[10],
            ip.payload[11],
        ]),
        flags: ip.payload[13],
        payload: &ip.payload[header_length..],
    })
}

fn validate_hostname(hostname: &str) -> Result<(), NetworkError> {
    if hostname.is_empty() || hostname.len() > 253 || hostname.ends_with('.') {
        return Err(NetworkError::BadHostname);
    }
    for label in hostname.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(NetworkError::BadHostname);
        }
    }
    Ok(())
}

fn build_dns_query(
    identifier: u16,
    hostname: &str,
    output: &mut [u8],
) -> Result<usize, NetworkError> {
    validate_hostname(hostname)?;
    if output.len() < 12 {
        return Err(NetworkError::BadHostname);
    }
    output[..12].fill(0);
    output[0..2].copy_from_slice(&identifier.to_be_bytes());
    output[2..4].copy_from_slice(&0x0100_u16.to_be_bytes()); // recursion desired
    output[4..6].copy_from_slice(&1_u16.to_be_bytes());
    let mut length = 12;
    for label in hostname.split('.') {
        let required = 1 + label.len();
        if length + required + 5 > output.len() {
            return Err(NetworkError::BadHostname);
        }
        output[length] = label.len() as u8;
        length += 1;
        output[length..length + label.len()].copy_from_slice(label.as_bytes());
        length += label.len();
    }
    output[length] = 0;
    length += 1;
    output[length..length + 2].copy_from_slice(&1_u16.to_be_bytes()); // A
    output[length + 2..length + 4].copy_from_slice(&1_u16.to_be_bytes()); // IN
    Ok(length + 4)
}

fn skip_dns_name(packet: &[u8], mut offset: usize) -> Option<usize> {
    for _ in 0..128 {
        let length = *packet.get(offset)?;
        if length & 0xC0 == 0xC0 {
            packet.get(offset + 1)?;
            return Some(offset + 2);
        }
        if length & 0xC0 != 0 {
            return None;
        }
        offset += 1;
        if length == 0 {
            return Some(offset);
        }
        let label_length = length as usize;
        if label_length > 63 || offset + label_length > packet.len() {
            return None;
        }
        offset += label_length;
    }
    None
}

fn parse_dns_a_response(identifier: u16, packet: &[u8]) -> Result<[u8; 4], NetworkError> {
    if packet.len() < 12 || u16::from_be_bytes([packet[0], packet[1]]) != identifier {
        return Err(NetworkError::MalformedDns);
    }
    let flags = u16::from_be_bytes([packet[2], packet[3]]);
    if flags & 0x8000 == 0 || flags & 0x0200 != 0 {
        return Err(NetworkError::MalformedDns);
    }
    match flags & 0x000F {
        0 => {}
        3 => return Err(NetworkError::DnsNoAddress),
        _ => return Err(NetworkError::DnsRefused),
    }
    let questions = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let answers = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let mut offset = 12;
    for _ in 0..questions {
        offset = skip_dns_name(packet, offset).ok_or(NetworkError::MalformedDns)?;
        if offset + 4 > packet.len() {
            return Err(NetworkError::MalformedDns);
        }
        offset += 4;
    }
    for _ in 0..answers {
        offset = skip_dns_name(packet, offset).ok_or(NetworkError::MalformedDns)?;
        if offset + 10 > packet.len() {
            return Err(NetworkError::MalformedDns);
        }
        let record_type = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let class = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);
        let data_length = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        if offset + data_length > packet.len() {
            return Err(NetworkError::MalformedDns);
        }
        if record_type == 1 && class == 1 && data_length == 4 {
            return Ok([
                packet[offset],
                packet[offset + 1],
                packet[offset + 2],
                packet[offset + 3],
            ]);
        }
        offset += data_length;
    }
    Err(NetworkError::DnsNoAddress)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_ascii_usize(bytes: &[u8]) -> Option<usize> {
    let mut value = 0_usize;
    let mut found = false;
    for byte in bytes.iter().copied() {
        if byte == b' ' || byte == b'\t' {
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        found = true;
        value = value.checked_mul(10)?.checked_add((byte - b'0') as usize)?;
    }
    found.then_some(value)
}

fn parse_chunk_size(line: &[u8]) -> Option<usize> {
    let value = line.split(|byte| *byte == b';').next()?;
    let mut result = 0_usize;
    let mut found = false;
    for byte in value.iter().copied() {
        if byte == b' ' || byte == b'\t' {
            continue;
        }
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        found = true;
        result = result.checked_mul(16)?.checked_add(digit as usize)?;
    }
    found.then_some(result)
}

fn decode_chunked(input: &[u8], output: &mut [u8]) -> Result<(usize, bool), NetworkError> {
    let mut input_offset = 0;
    let mut output_length = 0;
    loop {
        let line_end = find_bytes(&input[input_offset..], b"\r\n")
            .map(|relative| input_offset + relative)
            .ok_or(NetworkError::MalformedHttp)?;
        let chunk_length =
            parse_chunk_size(&input[input_offset..line_end]).ok_or(NetworkError::MalformedHttp)?;
        input_offset = line_end + 2;
        if chunk_length == 0 {
            return Ok((output_length, false));
        }
        let chunk_end = input_offset
            .checked_add(chunk_length)
            .filter(|end| end.checked_add(2).is_some_and(|tail| tail <= input.len()))
            .ok_or(NetworkError::MalformedHttp)?;
        if input[chunk_end..chunk_end + 2] != *b"\r\n" {
            return Err(NetworkError::MalformedHttp);
        }
        let remaining = output.len().saturating_sub(output_length);
        let copied = chunk_length.min(remaining);
        output[output_length..output_length + copied]
            .copy_from_slice(&input[input_offset..input_offset + copied]);
        output_length += copied;
        if copied != chunk_length {
            return Ok((output_length, true));
        }
        input_offset = chunk_end + 2;
    }
}

fn parse_http_response(
    peer: [u8; 4],
    wire: &[u8],
    wire_truncated: bool,
) -> Result<HttpResponse, NetworkError> {
    let header_end = find_bytes(wire, b"\r\n\r\n").ok_or(NetworkError::MalformedHttp)?;
    let first_line_end = find_bytes(&wire[..header_end], b"\r\n").unwrap_or(header_end);
    let status_line = &wire[..first_line_end];
    if !status_line.starts_with(b"HTTP/1.") {
        return Err(NetworkError::MalformedHttp);
    }
    let first_space = status_line
        .iter()
        .position(|byte| *byte == b' ')
        .ok_or(NetworkError::MalformedHttp)?;
    let status_bytes = status_line
        .get(first_space + 1..first_space + 4)
        .ok_or(NetworkError::MalformedHttp)?;
    if !status_bytes.iter().all(u8::is_ascii_digit) {
        return Err(NetworkError::MalformedHttp);
    }
    let status = ((status_bytes[0] - b'0') as u16) * 100
        + ((status_bytes[1] - b'0') as u16) * 10
        + (status_bytes[2] - b'0') as u16;

    let mut content_length = None;
    let mut chunked = false;
    let mut location = [0_u8; HTTP_LOCATION_CAPACITY];
    let mut location_len = 0;
    let mut cursor = first_line_end.saturating_add(2);
    while cursor < header_end {
        let line_length =
            find_bytes(&wire[cursor..header_end], b"\r\n").unwrap_or(header_end - cursor);
        let line = &wire[cursor..cursor + line_length];
        if let Some(colon) = line.iter().position(|byte| *byte == b':') {
            let name = &line[..colon];
            let value = &line[colon + 1..];
            if name.eq_ignore_ascii_case(b"content-length") {
                content_length = Some(parse_ascii_usize(value).ok_or(NetworkError::MalformedHttp)?);
            } else if name.eq_ignore_ascii_case(b"location") {
                let value = value.trim_ascii();
                if value.is_empty()
                    || value.len() > location.len()
                    || !value.iter().all(|byte| (0x21..=0x7E).contains(byte))
                {
                    return Err(NetworkError::MalformedHttp);
                }
                location[..value.len()].copy_from_slice(value);
                location_len = value.len();
            } else if name.eq_ignore_ascii_case(b"transfer-encoding")
                && value
                    .windows(7)
                    .any(|part| part.eq_ignore_ascii_case(b"chunked"))
            {
                chunked = true;
            }
        }
        cursor += line_length + 2;
    }

    let encoded_body = &wire[header_end + 4..];
    let mut response = HttpResponse {
        status,
        body: [0; HTTP_BODY_CAPACITY],
        body_len: 0,
        truncated: false,
        peer,
        location,
        location_len,
    };
    if chunked {
        let (length, truncated) = decode_chunked(encoded_body, &mut response.body)?;
        response.body_len = length;
        response.truncated = truncated || wire_truncated;
    } else {
        let expected = content_length.unwrap_or(encoded_body.len());
        if encoded_body.len() < expected && !wire_truncated {
            return Err(NetworkError::MalformedHttp);
        }
        let available = expected.min(encoded_body.len());
        response.body_len = available.min(response.body.len());
        response.body[..response.body_len].copy_from_slice(&encoded_body[..response.body_len]);
        response.truncated = expected > response.body.len()
            || available < expected
            || (content_length.is_none() && wire_truncated);
    }
    Ok(response)
}

#[derive(Clone, Copy)]
struct PciDevice {
    bus: u8,
    slot: u8,
}

fn find_rtl8139() -> Option<PciDevice> {
    for bus in 0_u16..=255 {
        for slot in 0_u8..32 {
            let identity = pci_read(bus as u8, slot, 0, 0);
            if identity as u16 == RTL_VENDOR && identity >> 16 == RTL_DEVICE as u32 {
                return Some(PciDevice {
                    bus: bus as u8,
                    slot,
                });
            }
        }
    }
    None
}

fn pci_read(bus: u8, slot: u8, function: u8, offset: u8) -> u32 {
    let address = 0x8000_0000
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((function as u32) << 8)
        | (offset as u32 & 0xFC);
    unsafe {
        port::outl(0xCF8, address);
        port::inl(0xCFC)
    }
}

fn pci_write(bus: u8, slot: u8, function: u8, offset: u8, value: u32) {
    let address = 0x8000_0000
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((function as u32) << 8)
        | (offset as u32 & 0xFC);
    unsafe {
        port::outl(0xCF8, address);
        port::outl(0xCFC, value);
    }
}

fn parse_ipv4(value: &str) -> Option<[u8; 4]> {
    let mut result = [0_u8; 4];
    let mut parts = value.split('.');
    for octet in &mut result {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 {
            return None;
        }
        let mut number = 0_u16;
        for byte in part.bytes() {
            if !byte.is_ascii_digit() {
                return None;
            }
            number = number * 10 + (byte - b'0') as u16;
        }
        *octet = u8::try_from(number).ok()?;
    }
    parts.next().is_none().then_some(result)
}

fn same_subnet(left: [u8; 4], right: [u8; 4], mask: [u8; 4]) -> bool {
    (0..4).all(|index| left[index] & mask[index] == right[index] & mask[index])
}

fn arp_reply(frame: &[u8], expected_ip: [u8; 4]) -> Option<([u8; 4], [u8; 6])> {
    if frame.len() < 42
        || frame[12..14] != [0x08, 0x06]
        || frame[14..16] != [0, 1]
        || frame[16..18] != [0x08, 0]
        || frame[18] != 6
        || frame[19] != 4
        || frame[20..22] != [0, 2]
        || frame[28..32] != expected_ip
        || frame[38..42] != LOCAL_IP
    {
        return None;
    }
    let mut sender_ip = [0_u8; 4];
    sender_ip.copy_from_slice(&frame[28..32]);
    let mut sender_mac = [0_u8; 6];
    sender_mac.copy_from_slice(&frame[22..28]);
    Some((sender_ip, sender_mac))
}

fn icmp_echo_reply(
    frame: &[u8],
    expected_ip: [u8; 4],
    identifier: u16,
    sequence: u16,
) -> Option<([u8; 4], u8, usize)> {
    if frame.len() < ETHERNET_HEADER_SIZE + 20 + 8 || frame[12..14] != [0x08, 0] {
        return None;
    }
    let ip_start = ETHERNET_HEADER_SIZE;
    let version_and_ihl = frame[ip_start];
    if version_and_ihl >> 4 != 4 {
        return None;
    }
    let header_length = ((version_and_ihl & 0x0F) as usize) * 4;
    if header_length < 20 || frame.len() < ip_start + header_length + 8 {
        return None;
    }
    if frame[ip_start + 9] != 1
        || frame[ip_start + 12..ip_start + 16] != expected_ip
        || frame[ip_start + 16..ip_start + 20] != LOCAL_IP
    {
        return None;
    }
    let total_length = u16::from_be_bytes([frame[ip_start + 2], frame[ip_start + 3]]) as usize;
    if total_length < header_length + 8 || ip_start + total_length > frame.len() {
        return None;
    }
    let icmp_start = ip_start + header_length;
    if frame[icmp_start] != 0
        || frame[icmp_start + 1] != 0
        || u16::from_be_bytes([frame[icmp_start + 4], frame[icmp_start + 5]]) != identifier
        || u16::from_be_bytes([frame[icmp_start + 6], frame[icmp_start + 7]]) != sequence
    {
        return None;
    }
    let mut source = [0_u8; 4];
    source.copy_from_slice(&frame[ip_start + 12..ip_start + 16]);
    Some((source, frame[ip_start + 8], total_length - header_length))
}

fn internet_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0_u32;
    let mut index = 0;
    while index + 1 < bytes.len() {
        sum += u16::from_be_bytes([bytes[index], bytes[index + 1]]) as u32;
        index += 2;
    }
    if index < bytes.len() {
        sum += (bytes[index] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn read_wrapped_u16(buffer: &[u8; RX_BUFFER_SIZE], offset: usize) -> u16 {
    u16::from_le_bytes([
        buffer[offset % RX_RING_SIZE],
        buffer[(offset + 1) % RX_RING_SIZE],
    ])
}

fn copy_wrapped(buffer: &[u8; RX_BUFFER_SIZE], offset: usize, output: &mut [u8]) {
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = buffer[(offset + index) % RX_RING_SIZE];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_strictly() {
        assert_eq!(parse_ipv4("10.0.2.2"), Some([10, 0, 2, 2]));
        assert_eq!(parse_ipv4("255.255.255.255"), Some([255; 4]));
        assert_eq!(parse_ipv4("256.1.1.1"), None);
        assert_eq!(parse_ipv4("1.2.3"), None);
        assert_eq!(parse_ipv4("1.2.3.4.5"), None);
    }

    #[test]
    fn calculates_known_ipv4_checksum() {
        let header = [
            0x45, 0x00, 0x00, 0x54, 0x00, 0x00, 0x40, 0x00, 0x40, 0x01, 0x00, 0x00, 0xC0, 0xA8,
            0x00, 0x01, 0xC0, 0xA8, 0x00, 0xC7,
        ];
        assert_eq!(internet_checksum(&header), 0xB890);
    }

    #[test]
    fn parses_bounded_redirect_location() {
        let wire = b"HTTP/1.1 302 Found\r\nLocation: /next?q=1\r\nContent-Length: 0\r\n\r\n";
        let response = parse_http_response([10, 0, 2, 2], wire, false).unwrap();
        assert_eq!(response.status, 302);
        assert_eq!(response.location(), Some("/next?q=1"));
        assert!(response.body().is_empty());
    }
}
