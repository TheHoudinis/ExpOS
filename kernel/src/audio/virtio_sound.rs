//! VirtIO 1.x PCM output for virtual machines.
//!
//! The driver implements the message-based 48 kHz, signed-16 stereo path from
//! the VirtIO Sound specification: stream discovery, SET_PARAMS/PREPARE/START,
//! a three-descriptor TX request and used-ring completion. Capture, jack
//! remapping, shared-memory transport and control elements remain out of scope.

use crate::{asl, pci, sync::SpinMutex, virtio};
use core::{hint::spin_loop, ptr};

const CONTROL_QUEUE: u16 = 0;
const EVENT_QUEUE: u16 = 1;
const TX_QUEUE: u16 = 2;
const RX_QUEUE: u16 = 3;
const R_PCM_INFO: u32 = 0x0100;
const R_PCM_SET_PARAMS: u32 = 0x0101;
const R_PCM_PREPARE: u32 = 0x0102;
const R_PCM_RELEASE: u32 = 0x0103;
const R_PCM_START: u32 = 0x0104;
const R_PCM_STOP: u32 = 0x0105;
const S_OK: u32 = 0x8000;
const PCM_FMT_S16: u8 = 5;
const PCM_RATE_48000: u8 = 7;
const PCM_DIRECTION_OUTPUT: u8 = 0;
const PCM_INFO_BYTES: usize = 32;
const CONTROL_DMA_BYTES: usize = 512;
const CONTROL_POLL_LIMIT: usize = 5_000_000;
const BUFFER_BYTES: u32 = 2 * 1024 * 1024;
const PERIOD_BYTES: u32 = 64 * 1024;

#[repr(C, align(4096))]
struct Queue(virtio::QueueMemory);

#[repr(C, align(64))]
struct ControlDma {
    request: [u8; CONTROL_DMA_BYTES],
    response: [u8; CONTROL_DMA_BYTES],
}

#[repr(C, align(16))]
struct TxDma {
    stream_id: u32,
    status: u32,
    latency_bytes: u32,
}

#[repr(C, align(16))]
struct EventDma {
    code: u32,
    data: u32,
}

static mut CONTROL_MEMORY: Queue = Queue(virtio::QueueMemory::new());
static mut EVENT_MEMORY: Queue = Queue(virtio::QueueMemory::new());
static mut TX_MEMORY: Queue = Queue(virtio::QueueMemory::new());
static mut RX_MEMORY: Queue = Queue(virtio::QueueMemory::new());
static mut CONTROL_DMA: ControlDma = ControlDma {
    request: [0; CONTROL_DMA_BYTES],
    response: [0; CONTROL_DMA_BYTES],
};
static mut TX_DMA: TxDma = TxDma {
    stream_id: 0,
    status: 0,
    latency_bytes: 0,
};
static mut EVENT_DMA: EventDma = EventDma { code: 0, data: 0 };

pub struct Ready {
    pub stream: u32,
    pub queue_size: u16,
}

struct Device {
    function: pci::Function,
    transport: virtio::ModernTransport,
    control: virtio::ModernSplitQueue,
    event: virtio::ModernSplitQueue,
    tx: virtio::ModernSplitQueue,
    _rx: virtio::ModernSplitQueue,
    stream: u32,
    started: bool,
    in_flight: bool,
    completions: u64,
    errors: u64,
    last_status: u32,
}

static DEVICE: SpinMutex<Option<Device>> = SpinMutex::new(None);

pub fn initialize() -> Option<Ready> {
    let function = pci::find(|candidate| {
        candidate.vendor_id == virtio::VENDOR_ID
            && candidate.device_id == virtio::MODERN_SOUND_DEVICE
    })?;
    let transport = virtio::ModernTransport::claim(function, virtio::MODERN_SOUND_DEVICE, 0, 0)?;
    if !asl::claim(function, asl::Owner::ExpAudio) {
        transport.reset();
        return None;
    }
    let configured = (|| {
        let streams = transport.device_config_u32(4)? as usize;
        let control = transport
            .configure_queue(CONTROL_QUEUE, unsafe {
                core::ptr::addr_of_mut!(CONTROL_MEMORY.0)
            })
            .ok()?;
        let event = transport
            .configure_queue(EVENT_QUEUE, unsafe {
                core::ptr::addr_of_mut!(EVENT_MEMORY.0)
            })
            .ok()?;
        let tx = transport
            .configure_queue(TX_QUEUE, unsafe { core::ptr::addr_of_mut!(TX_MEMORY.0) })
            .ok()?;
        let rx = transport
            .configure_queue(RX_QUEUE, unsafe { core::ptr::addr_of_mut!(RX_MEMORY.0) })
            .ok()?;
        Some((streams, control, event, tx, rx))
    })();
    let Some((streams, control, event, tx, rx)) = configured else {
        transport.fail();
        transport.reset();
        let _ = asl::release(function, asl::Owner::ExpAudio);
        return None;
    };
    if streams == 0 || streams > 8 || control.size() < 2 || event.size() < 1 || tx.size() < 3 {
        transport.fail();
        transport.reset();
        let _ = asl::release(function, asl::Owner::ExpAudio);
        return None;
    }
    transport.finish();
    let mut device = Device {
        function,
        transport,
        control,
        event,
        tx,
        _rx: rx,
        stream: 0,
        started: false,
        in_flight: false,
        completions: 0,
        errors: 0,
        last_status: 0,
    };
    if !device.arm_event_queue() {
        device.shutdown();
        return None;
    }
    let Some(stream) = device.find_output_stream(streams) else {
        device.shutdown();
        return None;
    };
    device.stream = stream;
    if !device.set_params() || !device.command(R_PCM_PREPARE) {
        device.shutdown();
        return None;
    }
    let queue_size = device.tx.size();
    crate::slog!(
        "EXPOS_VIRTIO_SOUND_READY transport=modern-pci stream={} rate=48000 channels=2 sample=s16le tx_queue={} completion=used-ring\r\n",
        stream,
        queue_size
    );
    *DEVICE.lock() = Some(device);
    Some(Ready { stream, queue_size })
}

pub fn submit(sample_address: usize, bytes: usize) -> bool {
    let mut guard = DEVICE.lock();
    let Some(device) = guard.as_mut() else {
        return false;
    };
    if device.in_flight || bytes == 0 || bytes > u32::MAX as usize {
        return false;
    }
    let dma = core::ptr::addr_of_mut!(TX_DMA);
    unsafe {
        ptr::write_volatile(
            core::ptr::addr_of_mut!((*dma).stream_id),
            device.stream.to_le(),
        );
        ptr::write_volatile(core::ptr::addr_of_mut!((*dma).status), 0);
        ptr::write_volatile(core::ptr::addr_of_mut!((*dma).latency_bytes), 0);
    }
    let header = unsafe { core::ptr::addr_of!((*dma).stream_id) as u64 };
    let status = unsafe { core::ptr::addr_of!((*dma).status) as u64 };
    if device
        .tx
        .set_descriptor(
            0,
            virtio::Descriptor {
                address: header,
                length: 4,
                flags: virtio::DESC_NEXT,
                next: 1,
            },
        )
        .is_err()
        || device
            .tx
            .set_descriptor(
                1,
                virtio::Descriptor {
                    address: sample_address as u64,
                    length: bytes as u32,
                    flags: virtio::DESC_NEXT,
                    next: 2,
                },
            )
            .is_err()
        || device
            .tx
            .set_descriptor(
                2,
                virtio::Descriptor {
                    address: status,
                    length: 8,
                    flags: virtio::DESC_WRITE,
                    next: 0,
                },
            )
            .is_err()
        || device.tx.submit(0).is_err()
    {
        return false;
    }
    device.in_flight = true;
    if !device.started {
        if !device.command(R_PCM_START) {
            device.in_flight = false;
            return false;
        }
        device.started = true;
    }
    true
}

pub fn take_events() -> u8 {
    let mut guard = DEVICE.lock();
    let Some(device) = guard.as_mut() else {
        return 0;
    };
    let Some((head, _)) = device.tx.pop_used() else {
        return 0;
    };
    device.in_flight = false;
    let status = unsafe { ptr::read_volatile(core::ptr::addr_of!(TX_DMA.status)) };
    device.last_status = u32::from_le(status);
    if head == 0 && device.last_status == S_OK {
        device.completions = device.completions.saturating_add(1);
        1 << 2
    } else {
        device.errors = device.errors.saturating_add(1);
        1 << 3
    }
}

pub fn stop() {
    let mut guard = DEVICE.lock();
    let Some(device) = guard.as_mut() else {
        return;
    };
    if device.started {
        let _ = device.command(R_PCM_STOP);
        device.started = false;
    }
    device.in_flight = false;
    while device.tx.pop_used().is_some() {}
}

pub fn last_status() -> u16 {
    DEVICE
        .lock()
        .as_ref()
        .map_or(0, |device| device.last_status as u16)
}

impl Device {
    fn execute(&mut self, request_bytes: usize, response_bytes: usize) -> bool {
        if request_bytes > CONTROL_DMA_BYTES || response_bytes > CONTROL_DMA_BYTES {
            return false;
        }
        let dma = core::ptr::addr_of_mut!(CONTROL_DMA);
        unsafe { ptr::write_bytes((*dma).response.as_mut_ptr(), 0, CONTROL_DMA_BYTES) };
        let request = unsafe { core::ptr::addr_of!((*dma).request) as u64 };
        let response = unsafe { core::ptr::addr_of!((*dma).response) as u64 };
        if self
            .control
            .set_descriptor(
                0,
                virtio::Descriptor {
                    address: request,
                    length: request_bytes as u32,
                    flags: virtio::DESC_NEXT,
                    next: 1,
                },
            )
            .is_err()
            || self
                .control
                .set_descriptor(
                    1,
                    virtio::Descriptor {
                        address: response,
                        length: response_bytes as u32,
                        flags: virtio::DESC_WRITE,
                        next: 0,
                    },
                )
                .is_err()
            || self.control.submit(0).is_err()
        {
            return false;
        }
        for _ in 0..CONTROL_POLL_LIMIT {
            if let Some((head, length)) = self.control.pop_used() {
                if head != 0 || length < 4 {
                    return false;
                }
                let status = unsafe {
                    u32::from_le(ptr::read_volatile(
                        core::ptr::addr_of!(CONTROL_DMA.response).cast::<u32>(),
                    ))
                };
                return status == S_OK;
            }
            spin_loop();
        }
        false
    }

    fn find_output_stream(&mut self, streams: usize) -> Option<u32> {
        let dma = core::ptr::addr_of_mut!(CONTROL_DMA);
        unsafe {
            ptr::write_bytes((*dma).request.as_mut_ptr(), 0, CONTROL_DMA_BYTES);
            put_u32(&mut (*dma).request, 0, R_PCM_INFO);
            put_u32(&mut (*dma).request, 4, 0);
            put_u32(&mut (*dma).request, 8, streams as u32);
            put_u32(&mut (*dma).request, 12, PCM_INFO_BYTES as u32);
        }
        let response_bytes = 4_usize.checked_add(streams.checked_mul(PCM_INFO_BYTES)?)?;
        if !self.execute(16, response_bytes) {
            return None;
        }
        for stream in 0..streams {
            let offset = 4 + stream * PCM_INFO_BYTES;
            let response = unsafe { core::ptr::addr_of!(CONTROL_DMA.response) };
            let formats = unsafe { read_u64(response, offset + 8)? };
            let rates = unsafe { read_u64(response, offset + 16)? };
            let direction = unsafe { ptr::read_volatile((*response).as_ptr().add(offset + 24)) };
            let channels_min = unsafe { ptr::read_volatile((*response).as_ptr().add(offset + 25)) };
            let channels_max = unsafe { ptr::read_volatile((*response).as_ptr().add(offset + 26)) };
            if direction == PCM_DIRECTION_OUTPUT
                && formats & (1 << PCM_FMT_S16) != 0
                && rates & (1 << PCM_RATE_48000) != 0
                && channels_min <= 2
                && channels_max >= 2
            {
                return Some(stream as u32);
            }
        }
        None
    }

    fn set_params(&mut self) -> bool {
        let dma = core::ptr::addr_of_mut!(CONTROL_DMA);
        unsafe {
            ptr::write_bytes((*dma).request.as_mut_ptr(), 0, CONTROL_DMA_BYTES);
            put_u32(&mut (*dma).request, 0, R_PCM_SET_PARAMS);
            put_u32(&mut (*dma).request, 4, self.stream);
            put_u32(&mut (*dma).request, 8, BUFFER_BYTES);
            put_u32(&mut (*dma).request, 12, PERIOD_BYTES);
            put_u32(&mut (*dma).request, 16, 0);
            (*dma).request[20] = 2;
            (*dma).request[21] = PCM_FMT_S16;
            (*dma).request[22] = PCM_RATE_48000;
        }
        self.execute(24, 4)
    }

    fn command(&mut self, code: u32) -> bool {
        let dma = core::ptr::addr_of_mut!(CONTROL_DMA);
        unsafe {
            ptr::write_bytes((*dma).request.as_mut_ptr(), 0, CONTROL_DMA_BYTES);
            put_u32(&mut (*dma).request, 0, code);
            put_u32(&mut (*dma).request, 4, self.stream);
        }
        self.execute(8, 4)
    }

    fn arm_event_queue(&mut self) -> bool {
        let event = core::ptr::addr_of_mut!(EVENT_DMA) as u64;
        self.event
            .set_descriptor(
                0,
                virtio::Descriptor {
                    address: event,
                    length: 8,
                    flags: virtio::DESC_WRITE,
                    next: 0,
                },
            )
            .is_ok()
            && self.event.submit(0).is_ok()
    }

    fn shutdown(&mut self) {
        if self.started {
            let _ = self.command(R_PCM_STOP);
        }
        let _ = self.command(R_PCM_RELEASE);
        self.transport.fail();
        self.transport.reset();
        let _ = asl::release(self.function, asl::Owner::ExpAudio);
    }
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

unsafe fn read_u64(input: *const [u8; CONTROL_DMA_BYTES], offset: usize) -> Option<u64> {
    (offset.checked_add(8)? <= CONTROL_DMA_BYTES).then(|| unsafe {
        let mut bytes = [0_u8; 8];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = ptr::read_volatile((*input).as_ptr().add(offset + index));
        }
        u64::from_le_bytes(bytes)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtio_sound_command_layout_matches_spec() {
        let mut request = [0_u8; 24];
        put_u32(&mut request, 0, R_PCM_SET_PARAMS);
        put_u32(&mut request, 4, 3);
        put_u32(&mut request, 8, BUFFER_BYTES);
        put_u32(&mut request, 12, PERIOD_BYTES);
        request[20] = 2;
        request[21] = PCM_FMT_S16;
        request[22] = PCM_RATE_48000;
        assert_eq!(
            u32::from_le_bytes(request[0..4].try_into().unwrap()),
            0x0101
        );
        assert_eq!(request[20..23], [2, 5, 7]);
    }
}
