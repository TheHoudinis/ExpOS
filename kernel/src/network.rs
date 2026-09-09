//! Minimal native IPv4 networking for the transitional x86_64 kernel.
//!
//! This module deliberately implements only what the kernel can honestly
//! support today: a polling RTL8139 driver plus Ethernet, ARP, IPv4 and ICMP
//! echo.  It does not claim Wi-Fi, DHCP, DNS, TCP, TLS or HTTP support.

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
const BROADCAST_MAC: [u8; 6] = [0xFF; 6];

const IO_WAIT_LIMIT: usize = 2_000_000;
const RECEIVE_WAIT_LIMIT: usize = 12_000_000;

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
    println!("policy: restricted; ICMP diagnostics require Power or Operator authority");
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
    println!("transport: Ethernet + ARP + static IPv4 + ICMP echo");
    println!("tcp/udp/dns/tls: not implemented");
}

pub fn ping_text(
    broker: &CapabilityBroker,
    handle_id: u32,
    requester: Fin,
    dimension: Fin,
    arguments: &str,
) {
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
}
