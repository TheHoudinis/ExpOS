//! VirtIO legacy PCI Ethernet backend built on the shared split queue.

use super::NetworkError;
use crate::{
    asl, pci,
    sync::SpinMutex,
    virtio::{self, Descriptor, LegacyTransport, QueueMemory, SplitQueue},
};
use core::sync::atomic::{AtomicU16, AtomicU64, AtomicU8, Ordering};

const FEATURE_MAC: u32 = 1 << 5;
const HEADER_BYTES: usize = 10;
const FRAME_BYTES: usize = 1536;
const RX_BUFFERS_COUNT: usize = 32;
const NO_IRQ: u8 = u8::MAX;

#[repr(C, align(16))]
struct PacketBuffer([u8; HEADER_BYTES + FRAME_BYTES]);
#[repr(C, align(4096))]
struct RxBuffers([PacketBuffer; RX_BUFFERS_COUNT]);

static mut RX_QUEUE_MEMORY: QueueMemory = QueueMemory::new();
static mut TX_QUEUE_MEMORY: QueueMemory = QueueMemory::new();
static mut RX_BUFFERS: RxBuffers =
    RxBuffers([const { PacketBuffer([0; HEADER_BYTES + FRAME_BYTES]) }; RX_BUFFERS_COUNT]);
static mut TX_BUFFER: PacketBuffer = PacketBuffer([0; HEADER_BYTES + FRAME_BYTES]);
static IRQ_PORT: AtomicU16 = AtomicU16::new(0);
static IRQ_LINE: AtomicU8 = AtomicU8::new(NO_IRQ);
static IRQ_COUNT: AtomicU64 = AtomicU64::new(0);

struct State {
    transport: Option<LegacyTransport>,
    rx: Option<SplitQueue>,
    tx: Option<SplitQueue>,
    mac: [u8; 6],
}

impl State {
    const fn new() -> Self {
        Self {
            transport: None,
            rx: None,
            tx: None,
            mac: [0; 6],
        }
    }
}

static STATE: SpinMutex<State> = SpinMutex::new(State::new());

pub fn initialize() -> Option<[u8; 6]> {
    let mut state = STATE.lock();
    if state.transport.is_some() {
        return Some(state.mac);
    }
    let function = pci::find(|candidate| {
        candidate.vendor_id == virtio::VENDOR_ID && candidate.device_id == virtio::LEGACY_NET_DEVICE
    })?;
    let transport = LegacyTransport::claim(function, FEATURE_MAC)?;
    let mut rx = transport
        .configure_queue(0, core::ptr::addr_of_mut!(RX_QUEUE_MEMORY))
        .ok()?;
    let tx = transport
        .configure_queue(1, core::ptr::addr_of_mut!(TX_QUEUE_MEMORY))
        .ok()?;
    if rx.size() < RX_BUFFERS_COUNT as u16 || tx.size() == 0 {
        transport.fail();
        return None;
    }
    let mut mac = [0_u8; 6];
    for (index, byte) in mac.iter_mut().enumerate() {
        *byte = transport.config_u8(index as u16);
    }
    if mac == [0; 6] || mac == [0xFF; 6] {
        transport.fail();
        return None;
    }
    for index in 0..RX_BUFFERS_COUNT {
        rx.set_descriptor(
            index as u16,
            Descriptor {
                address: unsafe { core::ptr::addr_of!(RX_BUFFERS.0[index]) as u64 },
                length: (HEADER_BYTES + FRAME_BYTES) as u32,
                flags: virtio::DESC_WRITE,
                next: 0,
            },
        )
        .ok()?;
        rx.submit(index as u16).ok()?;
    }
    transport.finish();
    if let Some(irq) = function.interrupt_line() {
        if !crate::interrupts::enable_legacy_irq(irq) {
            transport.fail();
            return None;
        }
        IRQ_PORT.store(transport.io_base(), Ordering::Release);
        IRQ_LINE.store(irq, Ordering::Release);
    }
    IRQ_COUNT.store(0, Ordering::Release);
    state.transport = Some(transport);
    state.rx = Some(rx);
    state.tx = Some(tx);
    state.mac = mac;
    let _ = asl::claim(function, asl::Owner::ExpNetwork);
    crate::slog!(
        "EXPOS_VIRTIO_NET_READY transport=legacy-pci rx_buffers={} queue_dma_bytes={} completion=interrupt irq={:?}\r\n",
        RX_BUFFERS_COUNT,
        virtio::QUEUE_BYTES * 2,
        function.interrupt_line()
    );
    Some(mac)
}

pub fn link_up() -> bool {
    STATE.lock().transport.is_some()
}

pub fn send(frame: &[u8]) -> Result<(), NetworkError> {
    if frame.len() > FRAME_BYTES {
        return Err(NetworkError::PacketTooLarge);
    }
    let mut state = STATE.lock();
    let queue = state.tx.as_mut().ok_or(NetworkError::NoDevice)?;
    if queue.pop_used().is_some() {
        // Reap the previous single-buffer transaction.
    }
    unsafe {
        let buffer = core::ptr::addr_of_mut!(TX_BUFFER.0).cast::<u8>();
        core::ptr::write_bytes(buffer, 0, HEADER_BYTES + FRAME_BYTES);
        core::ptr::copy_nonoverlapping(frame.as_ptr(), buffer.add(HEADER_BYTES), frame.len());
    }
    queue
        .set_descriptor(
            0,
            Descriptor {
                address: unsafe { core::ptr::addr_of!(TX_BUFFER.0) as u64 },
                length: (HEADER_BYTES + frame.len()) as u32,
                flags: 0,
                next: 0,
            },
        )
        .map_err(|_| NetworkError::TransmitFailed)?;
    queue.submit(0).map_err(|_| NetworkError::TransmitFailed)?;
    Ok(())
}

pub fn receive(output: &mut [u8]) -> Result<usize, NetworkError> {
    let mut state = STATE.lock();
    let queue = state.rx.as_mut().ok_or(NetworkError::NoDevice)?;
    let Some((id, total)) = queue.pop_used() else {
        return Ok(0);
    };
    let index = id as usize;
    if index >= RX_BUFFERS_COUNT || (total as usize) < HEADER_BYTES {
        return Err(NetworkError::ReceiveFailed);
    }
    let length = total as usize - HEADER_BYTES;
    if length > FRAME_BYTES || length > output.len() {
        return Err(NetworkError::ReceiveFailed);
    }
    unsafe {
        output[..length]
            .copy_from_slice(&RX_BUFFERS.0[index].0[HEADER_BYTES..HEADER_BYTES + length]);
    }
    queue
        .submit(index as u16)
        .map_err(|_| NetworkError::ReceiveFailed)?;
    Ok(length)
}

pub fn handle_legacy_interrupt(irq: u8) {
    if IRQ_LINE.load(Ordering::Acquire) != irq {
        return;
    }
    let port = IRQ_PORT.load(Ordering::Acquire);
    if port != 0 {
        let status = unsafe { crate::port::inb(port + 0x13) };
        if status != 0 {
            IRQ_COUNT.fetch_add(1, Ordering::AcqRel);
        }
    }
}

pub fn interrupt_count() -> u64 {
    IRQ_COUNT.load(Ordering::Acquire)
}
