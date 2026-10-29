//! VirtIO GPU 2D scanout for QEMU virtual machines.
//!
//! The compositor remains CPU-rendered into a bounded kernel shadow surface.
//! VirtIO GPU replaces framebuffer MMIO copies with resource-backed DMA plus
//! explicit transfer/flush commands. This is real device-backed 2D scanout;
//! it does not claim VirGL, Venus, OpenGL or 3D acceleration.

use crate::{asl, pci, sync::SpinMutex, virtio};
use core::{hint::spin_loop, mem::size_of, ptr};

const RESOURCE_ID: u32 = 1;
const SCANOUT_ID: u32 = 0;
const FORMAT_B8G8R8X8_UNORM: u32 = 2;
const CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const CMD_RESOURCE_UNREF: u32 = 0x0102;
const CMD_SET_SCANOUT: u32 = 0x0103;
const CMD_RESOURCE_FLUSH: u32 = 0x0104;
const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const RESP_OK_NODATA: u32 = 0x1100;
const RESPONSE_BYTES: usize = 24;
const COMMAND_CAPACITY: usize = 64;
const COMPLETION_POLL_LIMIT: usize = 5_000_000;

#[repr(C, align(4096))]
struct ControlQueue(virtio::QueueMemory);

#[repr(C, align(64))]
struct CommandDma {
    request: [u8; COMMAND_CAPACITY],
    response: [u8; COMMAND_CAPACITY],
}

static mut CONTROL_QUEUE: ControlQueue = ControlQueue(virtio::QueueMemory::new());
static mut COMMAND_DMA: CommandDma = CommandDma {
    request: [0; COMMAND_CAPACITY],
    response: [0; COMMAND_CAPACITY],
};

struct Device {
    function: pci::Function,
    transport: virtio::ModernTransport,
    queue: virtio::ModernSplitQueue,
    width: u32,
    height: u32,
    backing_bytes: u32,
    transfers: u64,
    flushes: u64,
}

static DEVICE: SpinMutex<Option<Device>> = SpinMutex::new(None);

pub fn initialize(width: usize, height: usize, backing: *mut u32, bytes: usize) -> bool {
    if width == 0
        || height == 0
        || width > u32::MAX as usize
        || height > u32::MAX as usize
        || bytes == 0
        || bytes > u32::MAX as usize
        || backing.is_null()
    {
        return false;
    }
    shutdown();
    let Some(function) = pci::find(|candidate| {
        candidate.vendor_id == virtio::VENDOR_ID && candidate.device_id == virtio::MODERN_GPU_DEVICE
    }) else {
        return false;
    };
    let Some(transport) = virtio::ModernTransport::claim(function, virtio::MODERN_GPU_DEVICE, 0, 0)
    else {
        crate::slog!(
            "EXPOS_VIRTIO_GPU_UNAVAILABLE reason=transport vendor={:04x} device={:04x}\r\n",
            function.vendor_id,
            function.device_id
        );
        return false;
    };
    if !asl::claim(function, asl::Owner::ExpDisplay) {
        transport.reset();
        return false;
    }
    let queue_memory = unsafe { core::ptr::addr_of_mut!(CONTROL_QUEUE.0) };
    let queue = match transport.configure_queue(0, queue_memory) {
        Ok(queue) if queue.size() >= 2 => queue,
        Ok(_) | Err(_) => {
            transport.fail();
            transport.reset();
            let _ = asl::release(function, asl::Owner::ExpDisplay);
            return false;
        }
    };
    transport.finish();
    let mut device = Device {
        function,
        transport,
        queue,
        width: width as u32,
        height: height as u32,
        backing_bytes: bytes as u32,
        transfers: 0,
        flushes: 0,
    };
    if !device.create_resource(backing as u64) {
        device.transport.fail();
        device.transport.reset();
        let _ = asl::release(function, asl::Owner::ExpDisplay);
        return false;
    }
    crate::slog!(
        "EXPOS_VIRTIO_GPU_READY transport=modern-pci mode=2d width={} height={} bytes={} queue={} raster=cpu\r\n",
        width,
        height,
        bytes,
        device.queue.size()
    );
    *DEVICE.lock() = Some(device);
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DamageRect {
    pub left: usize,
    pub top: usize,
    pub right: usize,
    pub bottom: usize,
}

impl DamageRect {
    pub const fn new(left: usize, top: usize, right: usize, bottom: usize) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

/// Upload separated compositor damage while holding the GPU lock once. Each
/// rectangle gets an exact transfer, followed by one bounding resource flush.
/// This avoids turning a small cursor plus a distant window update into a
/// nearly full-screen DMA transfer.
pub fn present_regions(regions: &[DamageRect]) -> bool {
    let mut guard = DEVICE.lock();
    let Some(device) = guard.as_mut() else {
        return false;
    };
    let Some(first) = regions.first().copied() else {
        return false;
    };
    let valid = |region: DamageRect| {
        region.left < region.right
            && region.top < region.bottom
            && region.right <= device.width as usize
            && region.bottom <= device.height as usize
    };
    if !valid(first) || regions.iter().copied().any(|region| !valid(region)) {
        return false;
    }
    let mut flush = first;
    for region in regions.iter().copied() {
        flush.left = flush.left.min(region.left);
        flush.top = flush.top.min(region.top);
        flush.right = flush.right.max(region.right);
        flush.bottom = flush.bottom.max(region.bottom);
        let rect = Rect {
            x: region.left as u32,
            y: region.top as u32,
            width: (region.right - region.left) as u32,
            height: (region.bottom - region.top) as u32,
        };
        if !device.transfer(rect) {
            crate::slog!(
                "EXPOS_VIRTIO_GPU_PRESENT_FAILED stage=transfer left={} top={} right={} bottom={}\r\n",
                region.left,
                region.top,
                region.right,
                region.bottom
            );
            return false;
        }
    }
    let flush = Rect {
        x: flush.left as u32,
        y: flush.top as u32,
        width: (flush.right - flush.left) as u32,
        height: (flush.bottom - flush.top) as u32,
    };
    if !device.flush(flush) {
        crate::slog!(
            "EXPOS_VIRTIO_GPU_PRESENT_FAILED stage=flush left={} top={} right={} bottom={}\r\n",
            flush.x,
            flush.y,
            flush.x + flush.width,
            flush.y + flush.height
        );
        return false;
    }
    true
}

pub fn shutdown() {
    let Some(mut device) = DEVICE.lock().take() else {
        return;
    };
    let mut request = Request::new(CMD_RESOURCE_UNREF);
    request.put_u32(24, RESOURCE_ID);
    let _ = device.execute(&request, 32);
    crate::slog!(
        "EXPOS_VIRTIO_GPU_STOP transfers={} flushes={}\r\n",
        device.transfers,
        device.flushes
    );
    device.transport.reset();
    let _ = asl::release(device.function, asl::Owner::ExpDisplay);
}

#[derive(Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl Device {
    fn create_resource(&mut self, backing: u64) -> bool {
        let mut create = Request::new(CMD_RESOURCE_CREATE_2D);
        create.put_u32(24, RESOURCE_ID);
        create.put_u32(28, FORMAT_B8G8R8X8_UNORM);
        create.put_u32(32, self.width);
        create.put_u32(36, self.height);
        if !self.execute(&create, 40) {
            return false;
        }

        let mut attach = Request::new(CMD_RESOURCE_ATTACH_BACKING);
        attach.put_u32(24, RESOURCE_ID);
        attach.put_u32(28, 1);
        attach.put_u64(32, backing);
        attach.put_u32(40, self.backing_bytes);
        if !self.execute(&attach, 48) {
            return false;
        }

        let mut scanout = Request::new(CMD_SET_SCANOUT);
        scanout.put_rect(
            24,
            Rect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
        );
        scanout.put_u32(40, SCANOUT_ID);
        scanout.put_u32(44, RESOURCE_ID);
        self.execute(&scanout, 48)
    }

    fn transfer(&mut self, rect: Rect) -> bool {
        let mut request = Request::new(CMD_TRANSFER_TO_HOST_2D);
        request.put_rect(24, rect);
        request.put_u64(40, 0);
        request.put_u32(48, RESOURCE_ID);
        let completed = self.execute(&request, 56);
        if completed {
            self.transfers = self.transfers.saturating_add(1);
        }
        completed
    }

    fn flush(&mut self, rect: Rect) -> bool {
        let mut request = Request::new(CMD_RESOURCE_FLUSH);
        request.put_rect(24, rect);
        request.put_u32(40, RESOURCE_ID);
        let completed = self.execute(&request, 48);
        if completed {
            self.flushes = self.flushes.saturating_add(1);
        }
        completed
    }

    fn execute(&mut self, request: &Request, length: usize) -> bool {
        if length > COMMAND_CAPACITY || self.queue.size() < 2 {
            return false;
        }
        let dma = core::ptr::addr_of_mut!(COMMAND_DMA);
        unsafe {
            ptr::copy_nonoverlapping(request.bytes.as_ptr(), (*dma).request.as_mut_ptr(), length);
            ptr::write_bytes((*dma).response.as_mut_ptr(), 0, COMMAND_CAPACITY);
        }
        let request_address = unsafe { core::ptr::addr_of!((*dma).request) as u64 };
        let response_address = unsafe { core::ptr::addr_of!((*dma).response) as u64 };
        if self
            .queue
            .set_descriptor(
                0,
                virtio::Descriptor {
                    address: request_address,
                    length: length as u32,
                    flags: virtio::DESC_NEXT,
                    next: 1,
                },
            )
            .is_err()
            || self
                .queue
                .set_descriptor(
                    1,
                    virtio::Descriptor {
                        address: response_address,
                        length: RESPONSE_BYTES as u32,
                        flags: virtio::DESC_WRITE,
                        next: 0,
                    },
                )
                .is_err()
            || self.queue.submit(0).is_err()
        {
            return false;
        }
        for _ in 0..COMPLETION_POLL_LIMIT {
            if let Some((head, response_length)) = self.queue.pop_used() {
                if head != 0 || response_length < RESPONSE_BYTES as u32 {
                    return false;
                }
                let response_type = unsafe {
                    ptr::read_volatile(core::ptr::addr_of!((*dma).response).cast::<u32>())
                };
                return response_type == RESP_OK_NODATA;
            }
            spin_loop();
        }
        false
    }
}

struct Request {
    bytes: [u8; COMMAND_CAPACITY],
}

impl Request {
    fn new(command: u32) -> Self {
        let mut request = Self {
            bytes: [0; COMMAND_CAPACITY],
        };
        request.put_u32(0, command);
        request
    }

    fn put_u32(&mut self, offset: usize, value: u32) {
        self.bytes[offset..offset + size_of::<u32>()].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(&mut self, offset: usize, value: u64) {
        self.bytes[offset..offset + size_of::<u64>()].copy_from_slice(&value.to_le_bytes());
    }

    fn put_rect(&mut self, offset: usize, rect: Rect) {
        self.put_u32(offset, rect.x);
        self.put_u32(offset + 4, rect.y);
        self.put_u32(offset + 8, rect.width);
        self.put_u32(offset + 12, rect.height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_command_uses_spec_offsets() {
        let mut request = Request::new(CMD_TRANSFER_TO_HOST_2D);
        request.put_rect(
            24,
            Rect {
                x: 7,
                y: 9,
                width: 320,
                height: 200,
            },
        );
        request.put_u64(40, 0);
        request.put_u32(48, RESOURCE_ID);
        assert_eq!(
            u32::from_le_bytes(request.bytes[0..4].try_into().unwrap()),
            0x0105
        );
        assert_eq!(
            u32::from_le_bytes(request.bytes[24..28].try_into().unwrap()),
            7
        );
        assert_eq!(
            u32::from_le_bytes(request.bytes[32..36].try_into().unwrap()),
            320
        );
        assert_eq!(
            u32::from_le_bytes(request.bytes[48..52].try_into().unwrap()),
            1
        );
    }

    #[test]
    fn command_dma_buffers_fit_two_descriptor_chain() {
        assert_eq!(size_of::<CommandDma>(), COMMAND_CAPACITY * 2);
        assert!(RESPONSE_BYTES <= COMMAND_CAPACITY);
    }
}
