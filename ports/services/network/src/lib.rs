#![no_std]

use core::{
    alloc::{GlobalAlloc, Layout},
    arch::{
        asm,
        x86_64::{__cpuid, _rdrand64_step},
    },
    num::NonZeroU32,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};
use embedded_io::Write as _;
use embedded_tls::blocking::{
    Aes128GcmSha256, Certificate, CryptoProvider, TlsClock, TlsConfig, TlsConnection, TlsContext,
    TlsVerifier,
};
use embedded_tls::{pki::CertVerifier, TlsError};
use expos_core::{AbiCall, AbiRequest, AbiResponse, AbiStatus};
use rand_core::{CryptoRng, RngCore};

const PAYLOAD_OFFSET: usize = 256;
const FRAME_CAPACITY: usize = 1536;
const DNS_CAPACITY: usize = 512;
const SOURCE_PORT: u16 = 49_193;
const RECEIVE_POLLS: usize = 200_000;
const RETRIES: usize = 2;
const OP_DNS: u8 = 1;
const OP_DHCP: u8 = 2;
const OP_TCP_EXCHANGE: u8 = 3;
const OP_HTTPS_EXCHANGE: u8 = 4;
const OP_UDP_EXCHANGE: u8 = 5;
const OP_PING: u8 = 6;
const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;
const DHCP_NAK: u8 = 6;
const DHCP_PACKET_CAPACITY: usize = 576;
const DHCP_POLLS: usize = 1_000_000;
const TCP_HEADER_SIZE: usize = 20;
const TCP_SYN_HEADER_SIZE: usize = 24;
const TCP_PAYLOAD_CAPACITY: usize = 1500 - 20 - TCP_HEADER_SIZE;
const TCP_OUTPUT_CAPACITY: usize = 16 * 1024;
const TCP_SOURCE_PORT: u16 = SOURCE_PORT + 1;
const TCP_RECEIVE_WINDOW: u16 = 32 * 1024;
const TCP_FIN: u8 = 0x01;
const TCP_SYN: u8 = 0x02;
const TCP_RST: u8 = 0x04;
const TCP_PSH: u8 = 0x08;
const TCP_ACK: u8 = 0x10;
const TLS_RECORD_CAPACITY: usize = 16_640;
const TLS_WRITE_CAPACITY: usize = 4_096;
const TLS_CERTIFICATE_CAPACITY: usize = 12 * 1024;
const ICMP_IDENTIFIER: u16 = 0x4858;
const ICMP_PAYLOAD: &[u8] = b"ExpOS native ICMP";
const RX_RING_SIZE: usize = 8 * 1024;
const RX_BUFFER_SIZE: usize = RX_RING_SIZE + 16 + FRAME_CAPACITY;
const TX_BUFFER_COUNT: usize = 4;
const IDR0: u16 = 0x00;
const TSD0: u16 = 0x10;
const TSAD0: u16 = 0x20;
const RBSTART: u16 = 0x30;
const COMMAND: u16 = 0x37;
const CAPR: u16 = 0x38;
const IMR: u16 = 0x3c;
const ISR: u16 = 0x3e;
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
const DRIVER_POLLS: usize = 2_000_000;

const GLOBAL_SIGN_ROOT_R1: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/global_sign_root_r1.der"));
const DIGICERT_GLOBAL_ROOT_G2: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/digicert_global_root_g2.der"));
const ISRG_ROOT_X1: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/isrg_root_x1.der"));
static TLS_UNIX_TIME: AtomicU64 = AtomicU64::new(0);
const SERVICE_HEAP_BYTES: usize = 384 * 1024;

#[repr(align(16))]
struct ServiceHeap([u8; SERVICE_HEAP_BYTES]);

static mut SERVICE_HEAP: ServiceHeap = ServiceHeap([0; SERVICE_HEAP_BYTES]);

#[repr(C, align(256))]
struct DmaBuffers {
    receive: [u8; RX_BUFFER_SIZE],
    transmit: [[u8; FRAME_CAPACITY]; TX_BUFFER_COUNT],
}

static mut SERVICE_DMA: DmaBuffers = DmaBuffers {
    receive: [0; RX_BUFFER_SIZE],
    transmit: [[0; FRAME_CAPACITY]; TX_BUFFER_COUNT],
};

#[derive(Clone, Copy)]
struct DriverState {
    initialized: bool,
    receive_offset: usize,
    transmit_index: usize,
}

static mut DRIVER: DriverState = DriverState {
    initialized: false,
    receive_offset: 0,
    transmit_index: 0,
};

struct ServiceAllocator {
    cursor: AtomicUsize,
}

unsafe impl Sync for ServiceAllocator {}

unsafe impl GlobalAlloc for ServiceAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let mut cursor = self.cursor.load(Ordering::Relaxed);
        loop {
            let start = cursor.wrapping_add(align - 1) & !(align - 1);
            let Some(end) = start.checked_add(layout.size()) else {
                return core::ptr::null_mut();
            };
            if end > SERVICE_HEAP_BYTES {
                return core::ptr::null_mut();
            }
            match self.cursor.compare_exchange_weak(
                cursor,
                end,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return core::ptr::addr_of_mut!(SERVICE_HEAP.0)
                        .cast::<u8>()
                        .add(start);
                }
                Err(next) => cursor = next,
            }
        }
    }

    unsafe fn dealloc(&self, _pointer: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOCATOR: ServiceAllocator = ServiceAllocator {
    cursor: AtomicUsize::new(0),
};

#[derive(Clone, Copy)]
struct Interface {
    mac: [u8; 6],
    address: [u8; 4],
    netmask: [u8; 4],
    gateway: [u8; 4],
    dns: [u8; 4],
}

#[derive(Clone, Copy)]
struct DhcpReply {
    message_type: u8,
    address: [u8; 4],
    netmask: Option<[u8; 4]>,
    gateway: Option<[u8; 4]>,
    dns: Option<[u8; 4]>,
    server: Option<[u8; 4]>,
    lease_seconds: Option<u32>,
}

#[derive(Clone, Copy)]
struct DhcpLease {
    address: [u8; 4],
    netmask: [u8; 4],
    gateway: [u8; 4],
    dns: [u8; 4],
    server: [u8; 4],
    lease_seconds: u32,
}

struct TcpPacket<'a> {
    sequence: u32,
    acknowledgement: u32,
    flags: u8,
    payload: &'a [u8],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrustAnchor {
    GlobalSignRootR1,
    DigiCertGlobalRootG2,
    IsrgRootX1,
}

impl TrustAnchor {
    const fn certificate(self) -> &'static [u8] {
        match self {
            Self::GlobalSignRootR1 => GLOBAL_SIGN_ROOT_R1,
            Self::DigiCertGlobalRootG2 => DIGICERT_GLOBAL_ROOT_G2,
            Self::IsrgRootX1 => ISRG_ROOT_X1,
        }
    }

    const fn id(self) -> u8 {
        match self {
            Self::GlobalSignRootR1 => 1,
            Self::DigiCertGlobalRootG2 => 2,
            Self::IsrgRootX1 => 3,
        }
    }
}

fn trust_anchor_for(hostname: &[u8]) -> TrustAnchor {
    let in_domain = |domain: &[u8]| {
        hostname.eq_ignore_ascii_case(domain)
            || (hostname.len() > domain.len()
                && hostname[hostname.len() - domain.len() - 1] == b'.'
                && hostname[hostname.len() - domain.len()..].eq_ignore_ascii_case(domain))
    };
    if in_domain(b"duckduckgo.com") {
        TrustAnchor::DigiCertGlobalRootG2
    } else if in_domain(b"wikipedia.org") || in_domain(b"open-meteo.com") {
        TrustAnchor::IsrgRootX1
    } else {
        TrustAnchor::GlobalSignRootR1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamError {
    Timeout,
    Reset,
    Invalid,
}

impl core::fmt::Display for StreamError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Timeout => "TCP timeout",
            Self::Reset => "TCP reset",
            Self::Invalid => "invalid TCP state",
        })
    }
}

impl core::error::Error for StreamError {}

impl embedded_io::Error for StreamError {
    fn kind(&self) -> embedded_io::ErrorKind {
        match self {
            Self::Timeout => embedded_io::ErrorKind::TimedOut,
            Self::Reset => embedded_io::ErrorKind::ConnectionReset,
            Self::Invalid => embedded_io::ErrorKind::InvalidData,
        }
    }
}

struct ServiceClock;

impl TlsClock for ServiceClock {
    fn now() -> Option<u64> {
        let value = TLS_UNIX_TIME.load(Ordering::Relaxed);
        (value != 0).then_some(value)
    }
}

struct HardwareRng;

impl HardwareRng {
    fn available() -> bool {
        __cpuid(1).ecx & (1 << 30) != 0
    }

    fn word() -> Option<u64> {
        if !Self::available() {
            return None;
        }
        for _ in 0..64 {
            let mut value = 0_u64;
            if unsafe { _rdrand64_step(&mut value) } == 1 {
                return Some(value);
            }
            core::hint::spin_loop();
        }
        None
    }

    fn failure() -> rand_core::Error {
        rand_core::Error::from(
            NonZeroU32::new(rand_core::Error::CUSTOM_START)
                .expect("rand_core custom error code is non-zero"),
        )
    }
}

impl RngCore for HardwareRng {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_u64(&mut self) -> u64 {
        Self::word().expect("RDRAND failed after availability check")
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        self.try_fill_bytes(destination)
            .expect("RDRAND failed after availability check");
    }

    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> Result<(), rand_core::Error> {
        for chunk in destination.chunks_mut(8) {
            let bytes = Self::word().ok_or_else(Self::failure)?.to_ne_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Ok(())
    }
}

impl CryptoRng for HardwareRng {}

struct VerifiedProvider<'a> {
    rng: HardwareRng,
    verifier: CertVerifier<'a, Aes128GcmSha256, ServiceClock, TLS_CERTIFICATE_CAPACITY>,
}

impl CryptoProvider for VerifiedProvider<'_> {
    type CipherSuite = Aes128GcmSha256;
    type Signature = &'static [u8];

    fn rng(&mut self) -> impl embedded_tls::CryptoRngCore {
        &mut self.rng
    }

    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Aes128GcmSha256>, TlsError> {
        Ok(&mut self.verifier)
    }
}

unsafe fn call(request: *mut AbiRequest, response: *mut AbiResponse) {
    let mut magic = 0x4558_504F_5341_4249_u64;
    asm!("int 0x80", inout("rax") magic, in("rdi") request, in("rsi") response);
    let _ = magic;
}

unsafe fn configuration(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
) -> Option<Interface> {
    let dma = core::ptr::addr_of_mut!(SERVICE_DMA);
    (*request).call = AbiCall::NetworkReceive as u16;
    (*request).handle_id = network;
    (*request).arguments = [
        dma as u64,
        core::mem::size_of::<DmaBuffers>() as u64,
        2,
        RX_BUFFER_SIZE as u64,
        FRAME_CAPACITY as u64,
        TX_BUFFER_COUNT as u64,
    ];
    call(request, response);
    if (*response).status != AbiStatus::Ok as u16 {
        return None;
    }
    if !driver_write(request, response, network, CONFIG1, 1, 0)
        || !driver_write(request, response, network, COMMAND, 1, COMMAND_RESET as u32)
    {
        return None;
    }
    let mut reset = false;
    for _ in 0..DRIVER_POLLS {
        if driver_read(request, response, network, COMMAND, 1)? as u8 & COMMAND_RESET == 0 {
            reset = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !reset {
        return None;
    }
    let mut mac = [0_u8; 6];
    for (index, octet) in mac.iter_mut().enumerate() {
        *octet = driver_read(request, response, network, IDR0 + index as u16, 1)? as u8;
    }
    if mac == [0; 6] || mac == [0xff; 6] {
        return None;
    }
    (*dma).receive.fill(0);
    for buffer in &mut (*dma).transmit {
        buffer.fill(0);
    }
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
    if !driver_write(request, response, network, RBSTART, 4, 0) {
        return None;
    }
    for index in 0..TX_BUFFER_COUNT {
        if !driver_write(request, response, network, TSAD0 + index as u16 * 4, 4, 0) {
            return None;
        }
    }
    if !driver_write(request, response, network, IMR, 2, 0)
        || !driver_write(request, response, network, ISR, 2, 0xffff)
        || !driver_write(request, response, network, TCR, 4, 0x0300_0000)
        || !driver_write(
            request,
            response,
            network,
            RCR,
            4,
            (7 << 13) | (7 << 8) | (1 << 3) | (1 << 1),
        )
        || !driver_write(
            request,
            response,
            network,
            COMMAND,
            1,
            (COMMAND_RX_ENABLE | COMMAND_TX_ENABLE) as u32,
        )
        || !driver_write(request, response, network, CAPR, 2, 0xfff0)
    {
        return None;
    }
    DRIVER = DriverState {
        initialized: true,
        receive_offset: 0,
        transmit_index: 0,
    };
    if driver_read(request, response, network, MEDIA_STATUS, 1)? & (1 << 2) != 0 {
        return None;
    }

    (*request).call = AbiCall::NetworkReceive as u16;
    (*request).handle_id = network;
    (*request).arguments = [0, 0, 1, 0, 0, 0];
    call(request, response);
    if (*response).status != AbiStatus::Ok as u16 || (*response).values[3] == 0 {
        return None;
    }
    let unpack = |value: u64, offset: usize| ((value >> (offset * 8)) & 0xff) as u8;
    let address_value = (*response).values[1];
    let route_value = (*response).values[2];
    Some(Interface {
        mac,
        address: [
            unpack(address_value, 0),
            unpack(address_value, 1),
            unpack(address_value, 2),
            unpack(address_value, 3),
        ],
        netmask: [
            unpack(address_value, 4),
            unpack(address_value, 5),
            unpack(address_value, 6),
            unpack(address_value, 7),
        ],
        gateway: [
            unpack(route_value, 0),
            unpack(route_value, 1),
            unpack(route_value, 2),
            unpack(route_value, 3),
        ],
        dns: [
            unpack(route_value, 4),
            unpack(route_value, 5),
            unpack(route_value, 6),
            unpack(route_value, 7),
        ],
    })
}

unsafe fn driver_read(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    register: u16,
    width: u8,
) -> Option<u32> {
    (*request).call = AbiCall::NetworkReceive as u16;
    (*request).handle_id = network;
    (*request).arguments = [register as u64, width as u64, 3, 0, 0, 0];
    call(request, response);
    ((*response).status == AbiStatus::Ok as u16).then_some((*response).values[0] as u32)
}

unsafe fn driver_write(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    register: u16,
    width: u8,
    value: u32,
) -> bool {
    (*request).call = AbiCall::NetworkSend as u16;
    (*request).handle_id = network;
    (*request).arguments = [register as u64, width as u64, 3, value as u64, 0, 0];
    call(request, response);
    (*response).status == AbiStatus::Ok as u16
}

unsafe fn send(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    frame: &[u8],
) -> bool {
    if !DRIVER.initialized || frame.len() > FRAME_CAPACITY {
        return false;
    }
    let length = frame.len().max(60);
    let index = DRIVER.transmit_index;
    let status_register = TSD0 + index as u16 * 4;
    let mut available = false;
    for _ in 0..DRIVER_POLLS {
        if driver_read(request, response, network, status_register, 4)
            .is_some_and(|status| status & TX_STATUS_OWN != 0)
        {
            available = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !available {
        return false;
    }
    let dma = core::ptr::addr_of_mut!(SERVICE_DMA);
    let buffer = &mut (*dma).transmit[index];
    buffer[..length].fill(0);
    buffer[..frame.len()].copy_from_slice(frame);
    core::sync::atomic::compiler_fence(Ordering::Release);
    if !driver_write(
        request,
        response,
        network,
        status_register,
        4,
        (32 << 16) | length as u32,
    ) {
        return false;
    }
    for _ in 0..DRIVER_POLLS {
        let Some(status) = driver_read(request, response, network, status_register, 4) else {
            return false;
        };
        if status & TX_STATUS_OK != 0 {
            DRIVER.transmit_index = (index + 1) % TX_BUFFER_COUNT;
            return true;
        }
        if status & TX_STATUS_ABORTED != 0 {
            return false;
        }
        core::hint::spin_loop();
    }
    false
}

unsafe fn receive(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    frame: &mut [u8; FRAME_CAPACITY],
) -> Option<usize> {
    if !DRIVER.initialized {
        return Some(0);
    }
    if driver_read(request, response, network, COMMAND, 1)? as u8 & COMMAND_RX_BUFFER_EMPTY != 0 {
        return None;
    }
    core::sync::atomic::compiler_fence(Ordering::Acquire);
    let dma = core::ptr::addr_of_mut!(SERVICE_DMA);
    let ring = &(*dma).receive;
    let offset = DRIVER.receive_offset;
    let status = read_wrapped_u16(ring, offset);
    let device_length = read_wrapped_u16(ring, offset + 2) as usize;
    let valid_length = (4..=FRAME_CAPACITY + 4).contains(&device_length);
    let frame_length = device_length.saturating_sub(4);
    let valid = status & 1 != 0 && valid_length && frame_length <= frame.len();
    if valid {
        copy_wrapped(ring, offset + 4, &mut frame[..frame_length]);
    }
    let advance = if valid_length { device_length + 4 } else { 4 };
    DRIVER.receive_offset = (offset + advance + 3) & !3;
    DRIVER.receive_offset %= RX_RING_SIZE;
    if !driver_write(
        request,
        response,
        network,
        CAPR,
        2,
        DRIVER.receive_offset.wrapping_sub(16) as u16 as u32,
    ) || !driver_write(request, response, network, ISR, 2, 0xffff)
    {
        return Some(0);
    }
    core::sync::atomic::compiler_fence(Ordering::Release);
    valid.then_some(frame_length)
}

fn read_wrapped_u16(buffer: &[u8; RX_BUFFER_SIZE], offset: usize) -> u16 {
    let low = buffer[offset % RX_RING_SIZE];
    let high = buffer[(offset + 1) % RX_RING_SIZE];
    u16::from_le_bytes([low, high])
}

fn copy_wrapped(buffer: &[u8; RX_BUFFER_SIZE], offset: usize, output: &mut [u8]) {
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = buffer[(offset + index) % RX_RING_SIZE];
    }
}

fn same_subnet(interface: Interface, destination: [u8; 4]) -> bool {
    (0..4).all(|index| {
        interface.address[index] & interface.netmask[index]
            == destination[index] & interface.netmask[index]
    })
}

unsafe fn resolve_neighbor(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
    destination: [u8; 4],
) -> Option<[u8; 6]> {
    let target = if same_subnet(interface, destination) {
        destination
    } else {
        interface.gateway
    };
    let mut arp = [0_u8; 42];
    arp[..6].fill(0xff);
    arp[6..12].copy_from_slice(&interface.mac);
    arp[12..14].copy_from_slice(&0x0806_u16.to_be_bytes());
    arp[14..16].copy_from_slice(&1_u16.to_be_bytes());
    arp[16..18].copy_from_slice(&0x0800_u16.to_be_bytes());
    arp[18] = 6;
    arp[19] = 4;
    arp[20..22].copy_from_slice(&1_u16.to_be_bytes());
    arp[22..28].copy_from_slice(&interface.mac);
    arp[28..32].copy_from_slice(&interface.address);
    arp[38..42].copy_from_slice(&target);

    let mut frame = [0_u8; FRAME_CAPACITY];
    for _ in 0..RETRIES {
        if !send(request, response, network, &arp) {
            return None;
        }
        for _ in 0..RECEIVE_POLLS {
            let Some(length) = receive(request, response, network, &mut frame) else {
                core::hint::spin_loop();
                continue;
            };
            if length == 0 {
                return None;
            }
            if is_arp_request(&frame[..length], interface) {
                answer_arp(request, response, network, &frame[..length], interface);
                continue;
            }
            if length >= 42
                && frame[12..14] == 0x0806_u16.to_be_bytes()
                && frame[20..22] == 2_u16.to_be_bytes()
                && frame[28..32] == target
                && frame[38..42] == interface.address
            {
                let mut mac = [0_u8; 6];
                mac.copy_from_slice(&frame[22..28]);
                return Some(mac);
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
unsafe fn udp_exchange(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
    destination: [u8; 4],
    destination_port: u16,
    connection_seed: u64,
    payload: &[u8],
    output: &mut [u8],
) -> Result<(usize, bool), u64> {
    if destination_port == 0 || payload.len() > 1500 - 20 - 8 {
        return Err(1);
    }
    let neighbor =
        resolve_neighbor(request, response, network, interface, destination).ok_or(20_u64)?;
    let source_port = 49_152 + (connection_seed as u16 % 16_384);
    let udp_length = 8 + payload.len();
    let ip_length = 20 + udp_length;
    let frame_length = 14 + ip_length;
    let mut outbound = [0_u8; FRAME_CAPACITY];
    outbound[..6].copy_from_slice(&neighbor);
    outbound[6..12].copy_from_slice(&interface.mac);
    outbound[12..14].copy_from_slice(&0x0800_u16.to_be_bytes());
    let ip = &mut outbound[14..34];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&(ip_length as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&(connection_seed as u16).to_be_bytes());
    ip[6..8].copy_from_slice(&0x4000_u16.to_be_bytes());
    ip[8] = 64;
    ip[9] = 17;
    ip[12..16].copy_from_slice(&interface.address);
    ip[16..20].copy_from_slice(&destination);
    let ip_checksum = checksum(ip);
    ip[10..12].copy_from_slice(&ip_checksum.to_be_bytes());
    let udp = &mut outbound[34..42 + payload.len()];
    udp[0..2].copy_from_slice(&source_port.to_be_bytes());
    udp[2..4].copy_from_slice(&destination_port.to_be_bytes());
    udp[4..6].copy_from_slice(&(udp_length as u16).to_be_bytes());
    udp[8..].copy_from_slice(payload);
    let udp_checksum = transport_checksum(interface.address, destination, 17, udp);
    udp[6..8].copy_from_slice(
        &if udp_checksum == 0 {
            u16::MAX
        } else {
            udp_checksum
        }
        .to_be_bytes(),
    );

    let mut incoming = [0_u8; FRAME_CAPACITY];
    for _ in 0..RETRIES {
        if !send(request, response, network, &outbound[..frame_length]) {
            return Err(21);
        }
        for _ in 0..RECEIVE_POLLS {
            let Some(length) = receive(request, response, network, &mut incoming) else {
                core::hint::spin_loop();
                continue;
            };
            if length == 0 {
                return Err(21);
            }
            if is_arp_request(&incoming[..length], interface) {
                answer_arp(request, response, network, &incoming[..length], interface);
                continue;
            }
            let Some(reply_payload) = parse_udp_reply(
                &incoming[..length],
                interface.address,
                destination,
                destination_port,
                source_port,
            ) else {
                continue;
            };
            let copied = reply_payload.len().min(output.len());
            output[..copied].copy_from_slice(&reply_payload[..copied]);
            return Ok((copied, copied != reply_payload.len()));
        }
    }
    Err(22)
}

fn parse_udp_reply(
    frame: &[u8],
    local_address: [u8; 4],
    expected_source: [u8; 4],
    expected_source_port: u16,
    expected_destination_port: u16,
) -> Option<&[u8]> {
    if frame.len() < 14 + 20 + 8 || frame[12..14] != 0x0800_u16.to_be_bytes() || frame[14] >> 4 != 4
    {
        return None;
    }
    let ip_header = ((frame[14] & 0x0f) as usize) * 4;
    let total_length = u16::from_be_bytes([frame[16], frame[17]]) as usize;
    if ip_header < 20
        || total_length < ip_header + 8
        || 14 + total_length > frame.len()
        || u16::from_be_bytes([frame[20], frame[21]]) & 0x3fff != 0
        || checksum(&frame[14..14 + ip_header]) != 0
        || frame[23] != 17
        || frame[26..30] != expected_source
        || frame[30..34] != local_address
    {
        return None;
    }
    let udp_offset = 14 + ip_header;
    let source_port = u16::from_be_bytes([frame[udp_offset], frame[udp_offset + 1]]);
    let destination_port = u16::from_be_bytes([frame[udp_offset + 2], frame[udp_offset + 3]]);
    let udp_length = u16::from_be_bytes([frame[udp_offset + 4], frame[udp_offset + 5]]) as usize;
    if source_port != expected_source_port
        || destination_port != expected_destination_port
        || udp_length < 8
        || udp_offset + udp_length > 14 + total_length
    {
        return None;
    }
    let udp = &frame[udp_offset..udp_offset + udp_length];
    let received_checksum = u16::from_be_bytes([udp[6], udp[7]]);
    if received_checksum != 0 && transport_checksum(expected_source, local_address, 17, udp) != 0 {
        return None;
    }
    Some(&udp[8..])
}

unsafe fn ping(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
    destination: [u8; 4],
    sequence: u16,
) -> Result<[u8; 9], u64> {
    let neighbor =
        resolve_neighbor(request, response, network, interface, destination).ok_or(20_u64)?;
    let icmp_length = 8 + ICMP_PAYLOAD.len();
    let ip_length = 20 + icmp_length;
    let frame_length = 14 + ip_length;
    let mut outbound = [0_u8; FRAME_CAPACITY];
    outbound[..6].copy_from_slice(&neighbor);
    outbound[6..12].copy_from_slice(&interface.mac);
    outbound[12..14].copy_from_slice(&0x0800_u16.to_be_bytes());
    let ip = &mut outbound[14..34];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&(ip_length as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&sequence.to_be_bytes());
    ip[6..8].copy_from_slice(&0x4000_u16.to_be_bytes());
    ip[8] = 64;
    ip[9] = 1;
    ip[12..16].copy_from_slice(&interface.address);
    ip[16..20].copy_from_slice(&destination);
    let ip_checksum = checksum(ip);
    ip[10..12].copy_from_slice(&ip_checksum.to_be_bytes());
    let icmp = &mut outbound[34..34 + icmp_length];
    icmp[0] = 8;
    icmp[4..6].copy_from_slice(&ICMP_IDENTIFIER.to_be_bytes());
    icmp[6..8].copy_from_slice(&sequence.to_be_bytes());
    icmp[8..].copy_from_slice(ICMP_PAYLOAD);
    let icmp_checksum = checksum(icmp);
    icmp[2..4].copy_from_slice(&icmp_checksum.to_be_bytes());

    let mut incoming = [0_u8; FRAME_CAPACITY];
    for _ in 0..RETRIES {
        if !send(request, response, network, &outbound[..frame_length]) {
            return Err(21);
        }
        for _ in 0..RECEIVE_POLLS {
            let Some(length) = receive(request, response, network, &mut incoming) else {
                core::hint::spin_loop();
                continue;
            };
            if length == 0 {
                return Err(21);
            }
            if is_arp_request(&incoming[..length], interface) {
                answer_arp(request, response, network, &incoming[..length], interface);
                continue;
            }
            if let Some((address, ttl, bytes)) = parse_echo_reply(
                &incoming[..length],
                destination,
                interface.address,
                sequence,
            ) {
                let mut encoded = [0_u8; 9];
                encoded[..4].copy_from_slice(&address);
                encoded[4..6].copy_from_slice(&(bytes as u16).to_be_bytes());
                encoded[6..8].copy_from_slice(&sequence.to_be_bytes());
                encoded[8] = ttl;
                return Ok(encoded);
            }
        }
    }
    Err(22)
}

fn parse_echo_reply(
    frame: &[u8],
    expected_source: [u8; 4],
    local_address: [u8; 4],
    sequence: u16,
) -> Option<([u8; 4], u8, usize)> {
    if frame.len() < 14 + 20 + 8 || frame[12..14] != 0x0800_u16.to_be_bytes() || frame[14] >> 4 != 4
    {
        return None;
    }
    let ip_header = ((frame[14] & 0x0f) as usize) * 4;
    let total_length = u16::from_be_bytes([frame[16], frame[17]]) as usize;
    if ip_header < 20
        || total_length < ip_header + 8
        || 14 + total_length > frame.len()
        || u16::from_be_bytes([frame[20], frame[21]]) & 0x3fff != 0
        || checksum(&frame[14..14 + ip_header]) != 0
        || frame[23] != 1
        || frame[26..30] != expected_source
        || frame[30..34] != local_address
    {
        return None;
    }
    let icmp = &frame[14 + ip_header..14 + total_length];
    if icmp[0] != 0
        || icmp[1] != 0
        || checksum(icmp) != 0
        || u16::from_be_bytes([icmp[4], icmp[5]]) != ICMP_IDENTIFIER
        || u16::from_be_bytes([icmp[6], icmp[7]]) != sequence
    {
        return None;
    }
    Some((expected_source, frame[22], icmp.len()))
}

fn is_arp_request(frame: &[u8], interface: Interface) -> bool {
    frame.len() >= 42
        && frame[12..14] == 0x0806_u16.to_be_bytes()
        && frame[20..22] == 1_u16.to_be_bytes()
        && frame[38..42] == interface.address
}

unsafe fn answer_arp(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    incoming: &[u8],
    interface: Interface,
) {
    let mut reply = [0_u8; 42];
    reply[..6].copy_from_slice(&incoming[22..28]);
    reply[6..12].copy_from_slice(&interface.mac);
    reply[12..14].copy_from_slice(&0x0806_u16.to_be_bytes());
    reply[14..20].copy_from_slice(&incoming[14..20]);
    reply[20..22].copy_from_slice(&2_u16.to_be_bytes());
    reply[22..28].copy_from_slice(&interface.mac);
    reply[28..32].copy_from_slice(&interface.address);
    reply[32..38].copy_from_slice(&incoming[22..28]);
    reply[38..42].copy_from_slice(&incoming[28..32]);
    let _ = send(request, response, network, &reply);
}

fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0_u32;
    for pair in bytes.chunks(2) {
        let word = if pair.len() == 2 {
            u16::from_be_bytes([pair[0], pair[1]])
        } else {
            (pair[0] as u16) << 8
        };
        sum += word as u32;
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
    }
    !(sum as u16)
}

fn checksum_add(mut sum: u32, bytes: &[u8]) -> u32 {
    for pair in bytes.chunks(2) {
        let word = if pair.len() == 2 {
            u16::from_be_bytes([pair[0], pair[1]])
        } else {
            (pair[0] as u16) << 8
        };
        sum += word as u32;
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
    }
    sum
}

fn transport_checksum(source: [u8; 4], destination: [u8; 4], protocol: u8, data: &[u8]) -> u16 {
    let mut pseudo = [0_u8; 12];
    pseudo[0..4].copy_from_slice(&source);
    pseudo[4..8].copy_from_slice(&destination);
    pseudo[9] = protocol;
    pseudo[10..12].copy_from_slice(&(data.len() as u16).to_be_bytes());
    let sum = checksum_add(checksum_add(0, &pseudo), data);
    !(sum as u16)
}

#[allow(clippy::too_many_arguments)]
unsafe fn send_tcp_segment(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
    destination_mac: [u8; 6],
    destination: [u8; 4],
    source_port: u16,
    destination_port: u16,
    sequence: u32,
    acknowledgement: u32,
    flags: u8,
    payload: &[u8],
) -> bool {
    let tcp_header = if flags & TCP_SYN != 0 {
        TCP_SYN_HEADER_SIZE
    } else {
        TCP_HEADER_SIZE
    };
    let tcp_length = tcp_header + payload.len();
    let ip_length = 20 + tcp_length;
    let frame_length = 14 + ip_length;
    if tcp_length > 1500 - 20 || frame_length > FRAME_CAPACITY {
        return false;
    }

    let mut frame = [0_u8; FRAME_CAPACITY];
    frame[..6].copy_from_slice(&destination_mac);
    frame[6..12].copy_from_slice(&interface.mac);
    frame[12..14].copy_from_slice(&0x0800_u16.to_be_bytes());
    let ip = &mut frame[14..34];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&(ip_length as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&(sequence as u16).to_be_bytes());
    ip[6..8].copy_from_slice(&0x4000_u16.to_be_bytes());
    ip[8] = 64;
    ip[9] = 6;
    ip[12..16].copy_from_slice(&interface.address);
    ip[16..20].copy_from_slice(&destination);
    let ip_checksum = checksum(ip);
    ip[10..12].copy_from_slice(&ip_checksum.to_be_bytes());

    let tcp = &mut frame[34..34 + tcp_length];
    tcp[0..2].copy_from_slice(&source_port.to_be_bytes());
    tcp[2..4].copy_from_slice(&destination_port.to_be_bytes());
    tcp[4..8].copy_from_slice(&sequence.to_be_bytes());
    tcp[8..12].copy_from_slice(&acknowledgement.to_be_bytes());
    tcp[12] = ((tcp_header / 4) as u8) << 4;
    tcp[13] = flags;
    tcp[14..16].copy_from_slice(&TCP_RECEIVE_WINDOW.to_be_bytes());
    if flags & TCP_SYN != 0 {
        tcp[20..24].copy_from_slice(&[2, 4, 0x05, 0xB4]);
    }
    tcp[tcp_header..].copy_from_slice(payload);
    let tcp_checksum = transport_checksum(interface.address, destination, 6, tcp);
    tcp[16..18].copy_from_slice(&tcp_checksum.to_be_bytes());
    send(request, response, network, &frame[..frame_length])
}

fn parse_tcp_packet<'a>(
    frame: &'a [u8],
    interface: Interface,
    expected_source: [u8; 4],
    expected_source_port: u16,
    expected_destination_port: u16,
) -> Result<TcpPacket<'a>, u64> {
    if frame.len() < 14 + 20 + TCP_HEADER_SIZE
        || frame[12..14] != 0x0800_u16.to_be_bytes()
        || frame[14] >> 4 != 4
    {
        return Err(10);
    }
    let ip_header = ((frame[14] & 0x0f) as usize).checked_mul(4).ok_or(11_u64)?;
    let total_length = u16::from_be_bytes([frame[16], frame[17]]) as usize;
    if ip_header < 20 {
        return Err(11);
    }
    if total_length < ip_header + TCP_HEADER_SIZE || 14 + total_length > frame.len() {
        return Err(12);
    }
    if frame[23] != 6 {
        return Err(13);
    }
    if u16::from_be_bytes([frame[20], frame[21]]) & 0x3fff != 0 {
        return Err(14);
    }
    if checksum(&frame[14..14 + ip_header]) != 0 {
        return Err(15);
    }
    if frame[26..30] != expected_source || frame[30..34] != interface.address {
        return Err(16);
    }
    let tcp = &frame[14 + ip_header..14 + total_length];
    let header_length = ((tcp[12] >> 4) as usize).checked_mul(4).ok_or(17_u64)?;
    if u16::from_be_bytes([tcp[0], tcp[1]]) != expected_source_port
        || u16::from_be_bytes([tcp[2], tcp[3]]) != expected_destination_port
        || header_length < TCP_HEADER_SIZE
        || header_length > tcp.len()
    {
        return Err(17);
    }
    if transport_checksum(expected_source, interface.address, 6, tcp) != 0 {
        return Err(18);
    }
    Ok(TcpPacket {
        sequence: u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]]),
        acknowledgement: u32::from_be_bytes([tcp[8], tcp[9], tcp[10], tcp[11]]),
        flags: tcp[13],
        payload: &tcp[header_length..],
    })
}

#[allow(clippy::too_many_arguments)]
unsafe fn tcp_exchange(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
    destination: [u8; 4],
    destination_port: u16,
    connection_seed: u64,
    payload: &[u8],
    output: &mut [u8],
) -> Result<(usize, bool), u64> {
    if destination == [0; 4]
        || destination_port == 0
        || payload.is_empty()
        || payload.len() > 1500 - 20 - TCP_HEADER_SIZE
        || output.is_empty()
    {
        return Err(20);
    }
    let destination_mac =
        resolve_neighbor(request, response, network, interface, destination).ok_or(21_u64)?;
    let mut source_port = 49_152 | (connection_seed as u16 & 0x3fff);
    if source_port == SOURCE_PORT || source_port == TCP_SOURCE_PORT {
        source_port = source_port.wrapping_add(2);
    }
    let initial_sequence = ((connection_seed >> 16) as u32
        ^ u32::from_be_bytes(destination)
        ^ ((source_port as u32) << 16)
        ^ destination_port as u32)
        .max(1);
    let mut incoming = [0_u8; FRAME_CAPACITY];
    let mut remote_next = None;
    let mut parse_failure = 0;
    for _ in 0..RETRIES {
        if !send_tcp_segment(
            request,
            response,
            network,
            interface,
            destination_mac,
            destination,
            source_port,
            destination_port,
            initial_sequence,
            0,
            TCP_SYN,
            &[],
        ) {
            return Err(22);
        }
        for _ in 0..RECEIVE_POLLS {
            let Some(length) = receive(request, response, network, &mut incoming) else {
                core::hint::spin_loop();
                continue;
            };
            if length == 0 {
                return Err(23);
            }
            if is_arp_request(&incoming[..length], interface) {
                answer_arp(request, response, network, &incoming[..length], interface);
                continue;
            }
            let packet = match parse_tcp_packet(
                &incoming[..length],
                interface,
                destination,
                destination_port,
                source_port,
            ) {
                Ok(packet) => packet,
                Err(error) => {
                    parse_failure = error;
                    continue;
                }
            };
            if packet.flags & TCP_RST != 0 {
                return Err(24);
            }
            if packet.flags & (TCP_SYN | TCP_ACK) == (TCP_SYN | TCP_ACK)
                && packet.acknowledgement == initial_sequence.wrapping_add(1)
            {
                remote_next = Some(packet.sequence.wrapping_add(1));
                break;
            }
        }
        if remote_next.is_some() {
            break;
        }
    }
    let mut remote_next = remote_next.ok_or(30 + parse_failure)?;
    let request_sequence = initial_sequence.wrapping_add(1);
    let mut local_next = request_sequence;
    if !send_tcp_segment(
        request,
        response,
        network,
        interface,
        destination_mac,
        destination,
        source_port,
        destination_port,
        local_next,
        remote_next,
        TCP_ACK,
        &[],
    ) || !send_tcp_segment(
        request,
        response,
        network,
        interface,
        destination_mac,
        destination,
        source_port,
        destination_port,
        local_next,
        remote_next,
        TCP_ACK | TCP_PSH,
        payload,
    ) {
        return Err(25);
    }
    local_next = local_next.wrapping_add(payload.len() as u32);

    let mut output_length = 0;
    let mut truncated = false;
    let mut peer_acked_request = false;
    let mut idle = 0;
    let mut retries = 0;
    let mut remote_closed = false;
    while !remote_closed && !truncated {
        let Some(length) = receive(request, response, network, &mut incoming) else {
            idle += 1;
            if idle < RECEIVE_POLLS {
                core::hint::spin_loop();
                continue;
            }
            idle = 0;
            if retries + 1 >= RETRIES {
                return Err(26);
            }
            retries += 1;
            let (sequence, flags, retransmit) = if peer_acked_request || output_length != 0 {
                (local_next, TCP_ACK, &[][..])
            } else {
                (request_sequence, TCP_ACK | TCP_PSH, payload)
            };
            if !send_tcp_segment(
                request,
                response,
                network,
                interface,
                destination_mac,
                destination,
                source_port,
                destination_port,
                sequence,
                remote_next,
                flags,
                retransmit,
            ) {
                return Err(27);
            }
            continue;
        };
        if length == 0 {
            return Err(28);
        }
        if is_arp_request(&incoming[..length], interface) {
            answer_arp(request, response, network, &incoming[..length], interface);
            continue;
        }
        let Ok(packet) = parse_tcp_packet(
            &incoming[..length],
            interface,
            destination,
            destination_port,
            source_port,
        ) else {
            continue;
        };
        idle = 0;
        if packet.flags & TCP_RST != 0 {
            return Err(29);
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
        if (!packet.payload.is_empty() || packet.flags & TCP_FIN != 0)
            && !send_tcp_segment(
                request,
                response,
                network,
                interface,
                destination_mac,
                destination,
                source_port,
                destination_port,
                local_next,
                remote_next,
                TCP_ACK,
                &[],
            )
        {
            return Err(30);
        }
    }
    let _ = send_tcp_segment(
        request,
        response,
        network,
        interface,
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

struct TcpConnection {
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
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

impl TcpConnection {
    fn send_segment(
        &mut self,
        sequence: u32,
        flags: u8,
        payload: &[u8],
    ) -> Result<(), StreamError> {
        unsafe {
            send_tcp_segment(
                self.request,
                self.response,
                self.network,
                self.interface,
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
        .then_some(())
        .ok_or(StreamError::Invalid)
    }

    fn send_ack(&mut self) -> Result<(), StreamError> {
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

    fn receive_into_pending(&mut self, frame: &[u8]) -> Result<(bool, bool), StreamError> {
        let Ok(packet) = parse_tcp_packet(
            frame,
            self.interface,
            self.destination,
            self.destination_port,
            self.source_port,
        ) else {
            return Ok((false, false));
        };
        if packet.flags & TCP_RST != 0 {
            return Err(StreamError::Reset);
        }
        let acknowledges_local =
            packet.flags & TCP_ACK != 0 && packet.acknowledgement == self.local_next;
        let mut accepted = false;
        if packet.sequence == self.remote_next {
            if !packet.payload.is_empty() && self.pending_length == 0 {
                if packet.payload.len() > self.pending.len() {
                    return Err(StreamError::Invalid);
                }
                self.pending[..packet.payload.len()].copy_from_slice(packet.payload);
                self.pending_offset = 0;
                self.pending_length = packet.payload.len();
                self.remote_next = self.remote_next.wrapping_add(packet.payload.len() as u32);
                accepted = true;
            }
            if packet.flags & TCP_FIN != 0 {
                self.remote_next = self.remote_next.wrapping_add(1);
                self.remote_closed = true;
                accepted = true;
            }
        }
        if accepted || (!packet.payload.is_empty() && packet.sequence != self.remote_next) {
            self.send_ack()?;
        }
        Ok((true, acknowledges_local))
    }

    fn write_stream(&mut self, input: &[u8]) -> Result<usize, StreamError> {
        if self.local_closed || self.remote_closed {
            return Err(StreamError::Reset);
        }
        if input.is_empty() {
            return Ok(0);
        }
        let written = input.len().min(TCP_PAYLOAD_CAPACITY);
        let payload = &input[..written];
        let sequence = self.local_next;
        let expected_ack = sequence.wrapping_add(written as u32);
        let mut frame = [0_u8; FRAME_CAPACITY];
        for _ in 0..RETRIES {
            self.send_segment(sequence, TCP_ACK | TCP_PSH, payload)?;
            for _ in 0..RECEIVE_POLLS {
                let Some(length) =
                    (unsafe { receive(self.request, self.response, self.network, &mut frame) })
                else {
                    core::hint::spin_loop();
                    continue;
                };
                if length == 0 {
                    return Err(StreamError::Invalid);
                }
                if is_arp_request(&frame[..length], self.interface) {
                    unsafe {
                        answer_arp(
                            self.request,
                            self.response,
                            self.network,
                            &frame[..length],
                            self.interface,
                        )
                    };
                    continue;
                }
                let acknowledgement = parse_tcp_packet(
                    &frame[..length],
                    self.interface,
                    self.destination,
                    self.destination_port,
                    self.source_port,
                )
                .ok()
                .filter(|packet| packet.flags & TCP_ACK != 0)
                .map(|packet| packet.acknowledgement);
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
        Err(StreamError::Timeout)
    }

    fn read_stream(&mut self, output: &mut [u8]) -> Result<usize, StreamError> {
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
        let mut frame = [0_u8; FRAME_CAPACITY];
        for retry in 0..RETRIES {
            for _ in 0..RECEIVE_POLLS {
                let Some(length) =
                    (unsafe { receive(self.request, self.response, self.network, &mut frame) })
                else {
                    core::hint::spin_loop();
                    continue;
                };
                if length == 0 {
                    return Err(StreamError::Invalid);
                }
                if is_arp_request(&frame[..length], self.interface) {
                    unsafe {
                        answer_arp(
                            self.request,
                            self.response,
                            self.network,
                            &frame[..length],
                            self.interface,
                        )
                    };
                    continue;
                }
                self.receive_into_pending(&frame[..length])?;
                let copied = self.drain_pending(output);
                if copied != 0 {
                    return Ok(copied);
                }
                if self.remote_closed {
                    return Ok(0);
                }
            }
            if retry + 1 < RETRIES {
                self.send_ack()?;
            }
        }
        Err(StreamError::Timeout)
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

impl Drop for TcpConnection {
    fn drop(&mut self) {
        self.close_stream();
    }
}

impl embedded_io::ErrorType for TcpConnection {
    type Error = StreamError;
}

impl embedded_io::Read for TcpConnection {
    fn read(&mut self, output: &mut [u8]) -> Result<usize, Self::Error> {
        self.read_stream(output)
    }
}

impl embedded_io::Write for TcpConnection {
    fn write(&mut self, input: &[u8]) -> Result<usize, Self::Error> {
        self.write_stream(input)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn tcp_connect(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
    destination: [u8; 4],
    destination_port: u16,
    connection_seed: u64,
) -> Result<TcpConnection, StreamError> {
    let destination_mac = resolve_neighbor(request, response, network, interface, destination)
        .ok_or(StreamError::Timeout)?;
    let mut source_port = 49_152 | (connection_seed as u16 & 0x3fff);
    if source_port == SOURCE_PORT || source_port == TCP_SOURCE_PORT {
        source_port = source_port.wrapping_add(2);
    }
    let initial_sequence = ((connection_seed >> 16) as u32
        ^ u32::from_be_bytes(destination)
        ^ ((source_port as u32) << 16)
        ^ destination_port as u32)
        .max(1);
    let mut incoming = [0_u8; FRAME_CAPACITY];
    let mut remote_next = None;
    for _ in 0..RETRIES {
        if !send_tcp_segment(
            request,
            response,
            network,
            interface,
            destination_mac,
            destination,
            source_port,
            destination_port,
            initial_sequence,
            0,
            TCP_SYN,
            &[],
        ) {
            return Err(StreamError::Invalid);
        }
        for _ in 0..RECEIVE_POLLS {
            let Some(length) = receive(request, response, network, &mut incoming) else {
                core::hint::spin_loop();
                continue;
            };
            if length == 0 {
                return Err(StreamError::Invalid);
            }
            if is_arp_request(&incoming[..length], interface) {
                answer_arp(request, response, network, &incoming[..length], interface);
                continue;
            }
            let Ok(packet) = parse_tcp_packet(
                &incoming[..length],
                interface,
                destination,
                destination_port,
                source_port,
            ) else {
                continue;
            };
            if packet.flags & TCP_RST != 0 {
                return Err(StreamError::Reset);
            }
            if packet.flags & (TCP_SYN | TCP_ACK) == (TCP_SYN | TCP_ACK)
                && packet.acknowledgement == initial_sequence.wrapping_add(1)
            {
                remote_next = Some(packet.sequence.wrapping_add(1));
                break;
            }
        }
        if remote_next.is_some() {
            break;
        }
    }
    let remote_next = remote_next.ok_or(StreamError::Timeout)?;
    let local_next = initial_sequence.wrapping_add(1);
    if !send_tcp_segment(
        request,
        response,
        network,
        interface,
        destination_mac,
        destination,
        source_port,
        destination_port,
        local_next,
        remote_next,
        TCP_ACK,
        &[],
    ) {
        return Err(StreamError::Invalid);
    }
    Ok(TcpConnection {
        request,
        response,
        network,
        interface,
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

#[allow(clippy::too_many_arguments)]
unsafe fn https_exchange(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
    destination: [u8; 4],
    destination_port: u16,
    connection_seed: u64,
    unix_time: u64,
    hostname: &[u8],
    http_request: &[u8],
    output: &mut [u8],
) -> Result<(usize, bool, TrustAnchor), u64> {
    if unix_time == 0 || !HardwareRng::available() {
        return Err(60);
    }
    let hostname = core::str::from_utf8(hostname).map_err(|_| 61_u64)?;
    TLS_UNIX_TIME.store(unix_time, Ordering::Relaxed);
    let socket = tcp_connect(
        request,
        response,
        network,
        interface,
        destination,
        destination_port,
        connection_seed,
    )
    .map_err(|error| match error {
        StreamError::Timeout => 62_u64,
        StreamError::Reset => 63_u64,
        StreamError::Invalid => 64_u64,
    })?;
    let mut read_records = [0_u8; TLS_RECORD_CAPACITY];
    let mut write_records = [0_u8; TLS_WRITE_CAPACITY];
    let config = TlsConfig::new()
        .with_server_name(hostname)
        .enable_rsa_signatures();
    let trust_anchor = trust_anchor_for(hostname.as_bytes());
    let provider = VerifiedProvider {
        rng: HardwareRng,
        verifier: CertVerifier::new(Certificate::X509(trust_anchor.certificate())),
    };
    let mut connection =
        TlsConnection::<_, Aes128GcmSha256>::new(socket, &mut read_records, &mut write_records);
    connection
        .open(TlsContext::new(&config, provider))
        .map_err(map_tls_error)?;
    connection.write_all(http_request).map_err(map_tls_error)?;
    connection.flush().map_err(map_tls_error)?;
    let mut output_length = 0;
    let mut truncated = false;
    let mut application_data = [0_u8; TLS_RECORD_CAPACITY];
    loop {
        if output_length == output.len() {
            truncated = true;
            break;
        }
        match connection.read(&mut application_data) {
            Ok(0) | Err(TlsError::ConnectionClosed) => break,
            Ok(count) => {
                let copied = count.min(output.len() - output_length);
                output[output_length..output_length + copied]
                    .copy_from_slice(&application_data[..copied]);
                output_length += copied;
                if copied != count {
                    truncated = true;
                    break;
                }
            }
            Err(error) => return Err(map_tls_error(error)),
        }
    }
    let _ = connection.close();
    Ok((output_length, truncated, trust_anchor))
}

fn map_tls_error(error: TlsError) -> u64 {
    match error {
        TlsError::InvalidCertificate
        | TlsError::InvalidCertificateEntry
        | TlsError::InvalidCertificateRequest
        | TlsError::InvalidSignature
        | TlsError::InvalidSignatureScheme => 65,
        TlsError::Io(embedded_io::ErrorKind::TimedOut) => 62,
        TlsError::Io(embedded_io::ErrorKind::ConnectionReset) | TlsError::ConnectionClosed => 63,
        TlsError::InvalidRecord
        | TlsError::UnknownContentType
        | TlsError::InvalidApplicationData
        | TlsError::DecodeError
        | TlsError::ParseError(_) => 66,
        _ => 67,
    }
}

fn encode_dns_query(hostname: &[u8], output: &mut [u8; DNS_CAPACITY]) -> Option<usize> {
    if hostname.is_empty() || hostname.len() > 96 {
        return None;
    }
    output.fill(0);
    output[0..2].copy_from_slice(&0x9301_u16.to_be_bytes());
    output[2..4].copy_from_slice(&0x0100_u16.to_be_bytes());
    output[4..6].copy_from_slice(&1_u16.to_be_bytes());
    let mut used = 12;
    let mut label_start = 0;
    for index in 0..=hostname.len() {
        if index != hostname.len() && hostname[index] != b'.' {
            let byte = hostname[index];
            if !byte.is_ascii_alphanumeric() && byte != b'-' {
                return None;
            }
            continue;
        }
        let label = &hostname[label_start..index];
        if label.is_empty()
            || label.len() > 63
            || label[0] == b'-'
            || label[label.len() - 1] == b'-'
            || used + label.len() + 1 >= output.len()
        {
            return None;
        }
        output[used] = label.len() as u8;
        used += 1;
        output[used..used + label.len()].copy_from_slice(label);
        used += label.len();
        label_start = index + 1;
    }
    if used + 5 > output.len() {
        return None;
    }
    output[used] = 0;
    output[used + 1..used + 3].copy_from_slice(&1_u16.to_be_bytes());
    output[used + 3..used + 5].copy_from_slice(&1_u16.to_be_bytes());
    Some(used + 5)
}

fn skip_name(packet: &[u8], mut offset: usize) -> Option<usize> {
    for _ in 0..128 {
        let length = *packet.get(offset)? as usize;
        if length & 0xc0 == 0xc0 {
            packet.get(offset + 1)?;
            return Some(offset + 2);
        }
        offset += 1;
        if length == 0 {
            return Some(offset);
        }
        if length > 63 {
            return None;
        }
        offset = offset.checked_add(length)?;
        if offset > packet.len() {
            return None;
        }
    }
    None
}

fn parse_dns_response(packet: &[u8]) -> Option<[u8; 4]> {
    if packet.len() < 12
        || u16::from_be_bytes([packet[0], packet[1]]) != 0x9301
        || packet[2] & 0x80 == 0
        || packet[3] & 0x0f != 0
    {
        return None;
    }
    let questions = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let answers = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let mut offset = 12;
    for _ in 0..questions {
        offset = skip_name(packet, offset)?;
        offset = offset.checked_add(4)?;
        if offset > packet.len() {
            return None;
        }
    }
    for _ in 0..answers {
        offset = skip_name(packet, offset)?;
        if offset + 10 > packet.len() {
            return None;
        }
        let kind = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let class = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);
        let length = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        if offset + length > packet.len() {
            return None;
        }
        if kind == 1 && class == 1 && length == 4 {
            return Some([
                packet[offset],
                packet[offset + 1],
                packet[offset + 2],
                packet[offset + 3],
            ]);
        }
        offset += length;
    }
    None
}

fn parse_ipv4(text: &[u8]) -> Option<[u8; 4]> {
    let mut address = [0_u8; 4];
    let mut part = 0;
    let mut value = 0_u16;
    let mut digits = 0;
    for &byte in text {
        if byte == b'.' {
            if digits == 0 || part >= 3 {
                return None;
            }
            address[part] = value as u8;
            part += 1;
            value = 0;
            digits = 0;
        } else if byte.is_ascii_digit() {
            value = value * 10 + (byte - b'0') as u16;
            digits += 1;
            if value > 255 || digits > 3 {
                return None;
            }
        } else {
            return None;
        }
    }
    if part != 3 || digits == 0 {
        return None;
    }
    address[3] = value as u8;
    Some(address)
}

unsafe fn dns_lookup(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
    hostname: &[u8],
) -> Option<[u8; 4]> {
    if let Some(address) = parse_ipv4(hostname) {
        return Some(address);
    }
    let mut dns = [0_u8; DNS_CAPACITY];
    let dns_length = encode_dns_query(hostname, &mut dns)?;
    let mut dns_response = [0_u8; DNS_CAPACITY];
    let (length, truncated) = udp_exchange(
        request,
        response,
        network,
        interface,
        interface.dns,
        53,
        0x9301,
        &dns[..dns_length],
        &mut dns_response,
    )
    .ok()?;
    if truncated {
        return None;
    }
    parse_dns_response(&dns_response[..length])
}

fn append_dhcp_option(
    output: &mut [u8; DHCP_PACKET_CAPACITY],
    cursor: &mut usize,
    kind: u8,
    value: &[u8],
) -> bool {
    let Some(end) = cursor.checked_add(value.len() + 2) else {
        return false;
    };
    if value.len() > u8::MAX as usize || end >= output.len() {
        return false;
    }
    output[*cursor] = kind;
    output[*cursor + 1] = value.len() as u8;
    output[*cursor + 2..end].copy_from_slice(value);
    *cursor = end;
    true
}

fn build_dhcp_message(
    message_type: u8,
    transaction: u32,
    mac: [u8; 6],
    requested: Option<[u8; 4]>,
    server: Option<[u8; 4]>,
    output: &mut [u8; DHCP_PACKET_CAPACITY],
) -> Option<usize> {
    if !matches!(message_type, DHCP_DISCOVER | DHCP_REQUEST) {
        return None;
    }
    output.fill(0);
    output[0] = 1;
    output[1] = 1;
    output[2] = 6;
    output[4..8].copy_from_slice(&transaction.to_be_bytes());
    output[10..12].copy_from_slice(&0x8000_u16.to_be_bytes());
    output[28..34].copy_from_slice(&mac);
    output[236..240].copy_from_slice(&[99, 130, 83, 99]);
    let mut cursor = 240;
    if !append_dhcp_option(output, &mut cursor, 53, &[message_type]) {
        return None;
    }
    let client = [1, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]];
    if !append_dhcp_option(output, &mut cursor, 61, &client) {
        return None;
    }
    if let Some(address) = requested {
        if !append_dhcp_option(output, &mut cursor, 50, &address) {
            return None;
        }
    }
    if let Some(server) = server {
        if !append_dhcp_option(output, &mut cursor, 54, &server) {
            return None;
        }
    }
    if !append_dhcp_option(output, &mut cursor, 55, &[1, 3, 6, 51, 54]) {
        return None;
    }
    output[cursor] = 255;
    Some(cursor + 1)
}

fn dhcp_frame(
    payload: &[u8],
    transaction: u32,
    mac: [u8; 6],
    output: &mut [u8; FRAME_CAPACITY],
) -> Option<usize> {
    let udp_length = 8_usize.checked_add(payload.len())?;
    let ip_length = 20_usize.checked_add(udp_length)?;
    let frame_length = 14_usize.checked_add(ip_length)?;
    if frame_length > output.len() || ip_length > u16::MAX as usize {
        return None;
    }
    output[..frame_length].fill(0);
    output[..6].fill(0xff);
    output[6..12].copy_from_slice(&mac);
    output[12..14].copy_from_slice(&0x0800_u16.to_be_bytes());
    let ip = &mut output[14..34];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&(ip_length as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&(transaction as u16).to_be_bytes());
    ip[6..8].copy_from_slice(&0x4000_u16.to_be_bytes());
    ip[8] = 64;
    ip[9] = 17;
    ip[16..20].fill(0xff);
    let ip_checksum = checksum(ip);
    ip[10..12].copy_from_slice(&ip_checksum.to_be_bytes());
    let udp = &mut output[34..42 + payload.len()];
    udp[0..2].copy_from_slice(&DHCP_CLIENT_PORT.to_be_bytes());
    udp[2..4].copy_from_slice(&DHCP_SERVER_PORT.to_be_bytes());
    udp[4..6].copy_from_slice(&(udp_length as u16).to_be_bytes());
    // IPv4 permits a zero UDP checksum. The worker still validates all reply
    // lengths, transaction identity and BOOTP client identity.
    udp[6..8].fill(0);
    udp[8..].copy_from_slice(payload);
    Some(frame_length)
}

fn parse_dhcp_reply(frame: &[u8], transaction: u32, mac: [u8; 6]) -> Option<DhcpReply> {
    if frame.len() < 14 + 20 + 8 + 240
        || frame[12..14] != 0x0800_u16.to_be_bytes()
        || frame[14] >> 4 != 4
        || frame[23] != 17
    {
        return None;
    }
    let ip_header = ((frame[14] & 0x0f) as usize).checked_mul(4)?;
    if ip_header < 20 || 14 + ip_header + 8 > frame.len() {
        return None;
    }
    let udp_offset = 14 + ip_header;
    if frame[udp_offset..udp_offset + 2] != DHCP_SERVER_PORT.to_be_bytes()
        || frame[udp_offset + 2..udp_offset + 4] != DHCP_CLIENT_PORT.to_be_bytes()
    {
        return None;
    }
    let udp_length = u16::from_be_bytes([frame[udp_offset + 4], frame[udp_offset + 5]]) as usize;
    if udp_length < 8 + 240 || udp_offset + udp_length > frame.len() {
        return None;
    }
    let packet = &frame[udp_offset + 8..udp_offset + udp_length];
    if packet[0] != 2
        || packet[1] != 1
        || packet[2] != 6
        || packet[4..8] != transaction.to_be_bytes()
        || packet[28..34] != mac
        || packet[236..240] != [99, 130, 83, 99]
    {
        return None;
    }
    let address: [u8; 4] = packet[16..20].try_into().ok()?;
    let mut reply = DhcpReply {
        message_type: 0,
        address,
        netmask: None,
        gateway: None,
        dns: None,
        server: None,
        lease_seconds: None,
    };
    let mut cursor = 240;
    while cursor < packet.len() {
        let kind = packet[cursor];
        cursor += 1;
        if kind == 255 {
            break;
        }
        if kind == 0 {
            continue;
        }
        let length = *packet.get(cursor)? as usize;
        cursor += 1;
        let end = cursor.checked_add(length)?;
        let value = packet.get(cursor..end)?;
        match (kind, value) {
            (53, [message_type]) => reply.message_type = *message_type,
            (1, [a, b, c, d]) => reply.netmask = Some([*a, *b, *c, *d]),
            (3, [a, b, c, d, ..]) => reply.gateway = Some([*a, *b, *c, *d]),
            (6, [a, b, c, d, ..]) => reply.dns = Some([*a, *b, *c, *d]),
            (54, [a, b, c, d]) => reply.server = Some([*a, *b, *c, *d]),
            (51, [a, b, c, d]) => reply.lease_seconds = Some(u32::from_be_bytes([*a, *b, *c, *d])),
            _ => {}
        }
        cursor = end;
    }
    (reply.message_type != 0).then_some(reply)
}

unsafe fn exchange_dhcp(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
    payload: &[u8],
    transaction: u32,
    expected: u8,
) -> Option<DhcpReply> {
    let mut outbound = [0_u8; FRAME_CAPACITY];
    let outbound_length = dhcp_frame(payload, transaction, interface.mac, &mut outbound)?;
    let mut incoming = [0_u8; FRAME_CAPACITY];
    for _ in 0..RETRIES {
        if !send(request, response, network, &outbound[..outbound_length]) {
            return None;
        }
        for _ in 0..DHCP_POLLS {
            let Some(length) = receive(request, response, network, &mut incoming) else {
                core::hint::spin_loop();
                continue;
            };
            if length == 0 {
                return None;
            }
            let Some(reply) = parse_dhcp_reply(&incoming[..length], transaction, interface.mac)
            else {
                continue;
            };
            if reply.message_type == DHCP_NAK {
                return None;
            }
            if reply.message_type == expected {
                return Some(reply);
            }
        }
    }
    None
}

unsafe fn dhcp(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    network: u32,
    interface: Interface,
) -> Option<DhcpLease> {
    let transaction = (0x4558_0000_u32
        ^ u32::from_be_bytes([
            interface.mac[2],
            interface.mac[3],
            interface.mac[4],
            interface.mac[5],
        ]))
    .max(1);
    let mut packet = [0_u8; DHCP_PACKET_CAPACITY];
    let discover = build_dhcp_message(
        DHCP_DISCOVER,
        transaction,
        interface.mac,
        None,
        None,
        &mut packet,
    )?;
    let offer = exchange_dhcp(
        request,
        response,
        network,
        interface,
        &packet[..discover],
        transaction,
        DHCP_OFFER,
    )?;
    let server = offer.server?;
    if offer.address == [0; 4] {
        return None;
    }
    let requested = offer.address;
    let request_length = build_dhcp_message(
        DHCP_REQUEST,
        transaction,
        interface.mac,
        Some(requested),
        Some(server),
        &mut packet,
    )?;
    let acknowledgement = exchange_dhcp(
        request,
        response,
        network,
        interface,
        &packet[..request_length],
        transaction,
        DHCP_ACK,
    )?;
    Some(DhcpLease {
        address: if acknowledgement.address == [0; 4] {
            requested
        } else {
            acknowledgement.address
        },
        netmask: acknowledgement
            .netmask
            .or(offer.netmask)
            .unwrap_or([255, 255, 255, 0]),
        gateway: acknowledgement.gateway.or(offer.gateway).unwrap_or(server),
        dns: acknowledgement.dns.or(offer.dns).unwrap_or(server),
        server: acknowledgement.server.unwrap_or(server),
        lease_seconds: acknowledgement
            .lease_seconds
            .or(offer.lease_seconds)
            .unwrap_or(0),
    })
}

fn encode_lease(lease: DhcpLease) -> [u8; 24] {
    let mut output = [0_u8; 24];
    output[0..4].copy_from_slice(&lease.address);
    output[4..8].copy_from_slice(&lease.netmask);
    output[8..12].copy_from_slice(&lease.gateway);
    output[12..16].copy_from_slice(&lease.dns);
    output[16..20].copy_from_slice(&lease.server);
    output[20..24].copy_from_slice(&lease.lease_seconds.to_be_bytes());
    output
}

unsafe fn finish(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    execute: u32,
    reply: u32,
    result: u64,
    output: Option<&[u8]>,
) -> ! {
    if let Some(output) = output {
        for bytes in output.chunks(512) {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (request as *mut u8).add(PAYLOAD_OFFSET),
                bytes.len(),
            );
            (*request).call = AbiCall::IpcSend as u16;
            (*request).handle_id = reply;
            (*request).arguments = [PAYLOAD_OFFSET as u64, bytes.len() as u64, 0, 0, 0, 0];
            call(request, response);
            if (*response).status != AbiStatus::Ok as u16 {
                (*request).call = AbiCall::ExecutionExit as u16;
                (*request).handle_id = execute;
                (*request).arguments = [4, 0, 0, 0, 0, 0];
                call(request, response);
                asm!("ud2", options(noreturn));
            }
        }
    }
    (*request).call = AbiCall::ExecutionExit as u16;
    (*request).handle_id = execute;
    (*request).arguments = [result, 0, 0, 0, 0, 0];
    call(request, response);
    asm!("ud2", options(noreturn));
}

/// Run the bounded RTL8139 driver, DHCP, DNS, ICMP, UDP, TCP and TLS state with
/// no kernel-memory access. The sealed Network Handle admits only its private
/// DMA lease, interface metadata and the confined register/value portal.
///
/// # Safety
/// The service loader supplies the fixed ABI page, admitted Handle IDs and a
/// read-only input mapping that remains valid until the worker exits.
#[no_mangle]
#[link_section = ".text.entry"]
pub unsafe extern "C" fn _start(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    execute: u32,
    reply: u32,
    input: *const u8,
    length: usize,
    network: u32,
) -> ! {
    if length == 0 || length > 2048 {
        finish(request, response, execute, reply, 1, None);
    }
    let Some(interface) = configuration(request, response, network) else {
        finish(request, response, execute, reply, 2, None);
    };
    let operation = core::slice::from_raw_parts(input, length);
    match operation[0] {
        OP_DNS if operation.len() > 1 => {
            let address = dns_lookup(request, response, network, interface, &operation[1..]);
            if let Some(address) = address {
                finish(request, response, execute, reply, 0, Some(&address));
            }
            finish(request, response, execute, reply, 3, None);
        }
        OP_DHCP if operation.len() == 1 => {
            let lease = dhcp(request, response, network, interface);
            if let Some(lease) = lease {
                let encoded = encode_lease(lease);
                finish(request, response, execute, reply, 0, Some(&encoded));
            }
            finish(request, response, execute, reply, 3, None);
        }
        OP_TCP_EXCHANGE if operation.len() >= 20 => {
            let mut destination = [0_u8; 4];
            destination.copy_from_slice(&operation[1..5]);
            let destination_port = u16::from_be_bytes([operation[5], operation[6]]);
            let requested_capacity = u16::from_be_bytes([operation[7], operation[8]]) as usize;
            let request_length = u16::from_be_bytes([operation[9], operation[10]]) as usize;
            let connection_seed = u64::from_be_bytes([
                operation[11],
                operation[12],
                operation[13],
                operation[14],
                operation[15],
                operation[16],
                operation[17],
                operation[18],
            ]);
            if requested_capacity == 0
                || requested_capacity > TCP_OUTPUT_CAPACITY
                || request_length == 0
                || request_length > 1500 - 20 - TCP_HEADER_SIZE
                || operation.len() != 19 + request_length
            {
                finish(request, response, execute, reply, 1, None);
            }
            let mut encoded = [0_u8; TCP_OUTPUT_CAPACITY + 1];
            let exchange = tcp_exchange(
                request,
                response,
                network,
                interface,
                destination,
                destination_port,
                connection_seed,
                &operation[19..],
                &mut encoded[1..requested_capacity + 1],
            );
            match exchange {
                Ok((wire_length, truncated)) => {
                    encoded[0] = truncated as u8;
                    finish(
                        request,
                        response,
                        execute,
                        reply,
                        0,
                        Some(&encoded[..wire_length + 1]),
                    );
                }
                Err(error) => finish(request, response, execute, reply, error, None),
            }
        }
        OP_HTTPS_EXCHANGE if operation.len() >= 30 => {
            let mut destination = [0_u8; 4];
            destination.copy_from_slice(&operation[1..5]);
            let destination_port = u16::from_be_bytes([operation[5], operation[6]]);
            let requested_capacity = u16::from_be_bytes([operation[7], operation[8]]) as usize;
            let request_length = u16::from_be_bytes([operation[9], operation[10]]) as usize;
            let connection_seed = u64::from_be_bytes([
                operation[11],
                operation[12],
                operation[13],
                operation[14],
                operation[15],
                operation[16],
                operation[17],
                operation[18],
            ]);
            let unix_time = u64::from_be_bytes([
                operation[19],
                operation[20],
                operation[21],
                operation[22],
                operation[23],
                operation[24],
                operation[25],
                operation[26],
            ]);
            let hostname_length = operation[27] as usize;
            let request_offset = 28 + hostname_length;
            if requested_capacity == 0
                || requested_capacity > TCP_OUTPUT_CAPACITY
                || request_length == 0
                || request_length > 1500 - 20 - TCP_HEADER_SIZE
                || hostname_length == 0
                || hostname_length > 96
                || operation.len() != request_offset + request_length
            {
                finish(request, response, execute, reply, 1, None);
            }
            let mut encoded = [0_u8; TCP_OUTPUT_CAPACITY + 2];
            let exchange = https_exchange(
                request,
                response,
                network,
                interface,
                destination,
                destination_port,
                connection_seed,
                unix_time,
                &operation[28..request_offset],
                &operation[request_offset..],
                &mut encoded[2..requested_capacity + 2],
            );
            match exchange {
                Ok((wire_length, truncated, trust_anchor)) => {
                    encoded[0] = truncated as u8;
                    encoded[1] = trust_anchor.id();
                    finish(
                        request,
                        response,
                        execute,
                        reply,
                        0,
                        Some(&encoded[..wire_length + 2]),
                    );
                }
                Err(error) => finish(request, response, execute, reply, error, None),
            }
        }
        OP_UDP_EXCHANGE if operation.len() >= 19 => {
            let mut destination = [0_u8; 4];
            destination.copy_from_slice(&operation[1..5]);
            let destination_port = u16::from_be_bytes([operation[5], operation[6]]);
            let requested_capacity = u16::from_be_bytes([operation[7], operation[8]]) as usize;
            let request_length = u16::from_be_bytes([operation[9], operation[10]]) as usize;
            let connection_seed = u64::from_be_bytes([
                operation[11],
                operation[12],
                operation[13],
                operation[14],
                operation[15],
                operation[16],
                operation[17],
                operation[18],
            ]);
            if requested_capacity == 0
                || requested_capacity > TCP_OUTPUT_CAPACITY
                || request_length > 1500 - 20 - 8
                || operation.len() != 19 + request_length
            {
                finish(request, response, execute, reply, 1, None);
            }
            let mut encoded = [0_u8; TCP_OUTPUT_CAPACITY + 1];
            match udp_exchange(
                request,
                response,
                network,
                interface,
                destination,
                destination_port,
                connection_seed,
                &operation[19..],
                &mut encoded[1..requested_capacity + 1],
            ) {
                Ok((wire_length, truncated)) => {
                    encoded[0] = truncated as u8;
                    finish(
                        request,
                        response,
                        execute,
                        reply,
                        0,
                        Some(&encoded[..wire_length + 1]),
                    );
                }
                Err(error) => finish(request, response, execute, reply, error, None),
            }
        }
        OP_PING if operation.len() == 7 => {
            let mut destination = [0_u8; 4];
            destination.copy_from_slice(&operation[1..5]);
            let sequence = u16::from_be_bytes([operation[5], operation[6]]);
            match ping(request, response, network, interface, destination, sequence) {
                Ok(encoded) => finish(request, response, execute, reply, 0, Some(&encoded)),
                Err(error) => finish(request, response, execute, reply, error, None),
            }
        }
        _ => finish(request, response, execute, reply, 1, None),
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { asm!("ud2", options(noreturn)) }
}
