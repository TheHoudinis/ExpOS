//! Shared bounded VirtIO 0.9 PCI transport and split-queue primitives.
//!
//! ExpOS deliberately starts with the transitional I/O-port transport exposed
//! by QEMU (`virtio-*-pci,disable-modern=on`).  Both net and block use this
//! implementation, so feature negotiation, queue bounds and interrupt
//! acknowledgement have one auditable definition. Modern capability-only
//! VirtIO 1.x functions remain inventoried but are not claimed by this module.

use crate::{pci, port};
use core::{mem::size_of, ptr};

pub const VENDOR_ID: u16 = 0x1AF4;
pub const LEGACY_NET_DEVICE: u16 = 0x1000;
pub const LEGACY_BLOCK_DEVICE: u16 = 0x1001;
pub const MODERN_GPU_DEVICE: u16 = 0x1050;
pub const MODERN_SOUND_DEVICE: u16 = 0x1059;
pub const MAX_QUEUE_SIZE: u16 = 256;
pub const QUEUE_BYTES: usize = 16 * 1024;

const HOST_FEATURES: u16 = 0x00;
const GUEST_FEATURES: u16 = 0x04;
const QUEUE_PFN: u16 = 0x08;
const QUEUE_SIZE: u16 = 0x0C;
const QUEUE_SELECT: u16 = 0x0E;
const QUEUE_NOTIFY: u16 = 0x10;
const DEVICE_STATUS: u16 = 0x12;
pub const DEVICE_CONFIG: u16 = 0x14;

const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const STATUS_FEATURES_OK: u8 = 8;
const STATUS_FAILED: u8 = 128;

const PCI_CAP_VENDOR: u8 = 0x09;
const PCI_STATUS_CAPABILITIES: u32 = 1 << 20;
const CFG_COMMON: u8 = 1;
const CFG_NOTIFY: u8 = 2;
const CFG_DEVICE: u8 = 4;
const COMMON_DEVICE_FEATURE_SELECT: usize = 0x00;
const COMMON_DEVICE_FEATURE: usize = 0x04;
const COMMON_DRIVER_FEATURE_SELECT: usize = 0x08;
const COMMON_DRIVER_FEATURE: usize = 0x0C;
const COMMON_DEVICE_STATUS: usize = 0x14;
const COMMON_QUEUE_SELECT: usize = 0x16;
const COMMON_QUEUE_SIZE: usize = 0x18;
const COMMON_QUEUE_ENABLE: usize = 0x1C;
const COMMON_QUEUE_NOTIFY_OFF: usize = 0x1E;
const COMMON_QUEUE_DESC: usize = 0x20;
const COMMON_QUEUE_DRIVER: usize = 0x28;
const COMMON_QUEUE_DEVICE: usize = 0x30;
const VIRTIO_F_VERSION_1: u32 = 1;

pub const DESC_NEXT: u16 = 1;
pub const DESC_WRITE: u16 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Descriptor {
    pub address: u64,
    pub length: u32,
    pub flags: u16,
    pub next: u16,
}

#[repr(C, align(4096))]
pub struct QueueMemory(pub [u8; QUEUE_BYTES]);

impl QueueMemory {
    pub const fn new() -> Self {
        Self([0; QUEUE_BYTES])
    }
}

#[derive(Clone, Copy)]
pub struct LegacyTransport {
    io_base: u16,
}

impl LegacyTransport {
    pub fn claim(function: pci::Function, accepted_features: u32) -> Option<Self> {
        if function.vendor_id != VENDOR_ID
            || !matches!(function.device_id, LEGACY_NET_DEVICE | LEGACY_BLOCK_DEVICE)
        {
            return None;
        }
        let io_base = function.io_bar(0)?.address;
        function.enable_io_bus_master();
        function.write(0x04, function.read(0x04) & !(1 << 10));
        let transport = Self { io_base };
        unsafe {
            port::outb(io_base + DEVICE_STATUS, 0);
            port::outb(io_base + DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
            let offered = port::inl(io_base + HOST_FEATURES);
            port::outl(io_base + GUEST_FEATURES, offered & accepted_features);
        }
        Some(transport)
    }

    pub const fn io_base(self) -> u16 {
        self.io_base
    }

    pub fn configure_queue(
        self,
        index: u16,
        memory: *mut QueueMemory,
    ) -> Result<SplitQueue, QueueError> {
        let address = memory as u64;
        if address == 0 || address > u32::MAX as u64 || address & 0xFFF != 0 {
            return Err(QueueError::DmaAddress);
        }
        unsafe {
            port::outw(self.io_base + QUEUE_SELECT, index);
            let size = port::inw(self.io_base + QUEUE_SIZE);
            if size == 0 || size > MAX_QUEUE_SIZE || !size.is_power_of_two() {
                return Err(QueueError::UnsupportedSize(size));
            }
            ptr::write_bytes(memory.cast::<u8>(), 0, size_of::<QueueMemory>());
            port::outl(self.io_base + QUEUE_PFN, (address >> 12) as u32);
            Ok(SplitQueue {
                transport: self,
                index,
                size,
                memory,
                next_avail: 0,
                last_used: 0,
            })
        }
    }

    pub fn finish(self) {
        unsafe {
            let status = port::inb(self.io_base + DEVICE_STATUS);
            port::outb(self.io_base + DEVICE_STATUS, status | STATUS_DRIVER_OK);
        }
    }

    pub fn fail(self) {
        unsafe {
            let status = port::inb(self.io_base + DEVICE_STATUS);
            port::outb(self.io_base + DEVICE_STATUS, status | STATUS_FAILED);
        }
    }

    pub fn config_u8(self, offset: u16) -> u8 {
        unsafe { port::inb(self.io_base + DEVICE_CONFIG + offset) }
    }

    pub fn config_u32(self, offset: u16) -> u32 {
        unsafe { port::inl(self.io_base + DEVICE_CONFIG + offset) }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ModernCapability {
    address: usize,
    length: usize,
    notify_multiplier: u32,
}

/// Bounded VirtIO 1.x PCI capability transport.
///
/// This is intentionally small: it maps only the common configuration and
/// notification capabilities needed by split queues. Device-specific drivers
/// still own their feature policy and command formats.
#[derive(Clone, Copy)]
pub struct ModernTransport {
    common: *mut u8,
    notify: *mut u8,
    notify_length: usize,
    notify_multiplier: u32,
    device: Option<(*mut u8, usize)>,
}

unsafe impl Send for ModernTransport {}

impl ModernTransport {
    pub fn claim(
        function: pci::Function,
        expected_device: u16,
        accepted_features_low: u32,
        accepted_features_high: u32,
    ) -> Option<Self> {
        if function.vendor_id != VENDOR_ID || function.device_id != expected_device {
            return None;
        }
        let common = find_modern_capability(function, CFG_COMMON)?;
        let notify = find_modern_capability(function, CFG_NOTIFY)?;
        let device = find_modern_capability(function, CFG_DEVICE)
            .map(|capability| (capability.address as *mut u8, capability.length));
        if common.length < 0x38 || notify.length < 2 || notify.notify_multiplier == 0 {
            return None;
        }
        function.enable_memory_bus_master();
        let transport = Self {
            common: common.address as *mut u8,
            notify: notify.address as *mut u8,
            notify_length: notify.length,
            notify_multiplier: notify.notify_multiplier,
            device,
        };
        transport.write_status(0);
        transport.write_status(STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        transport.write_common_u32(COMMON_DEVICE_FEATURE_SELECT, 0);
        let offered_low = transport.read_common_u32(COMMON_DEVICE_FEATURE);
        transport.write_common_u32(COMMON_DEVICE_FEATURE_SELECT, 1);
        let offered_high = transport.read_common_u32(COMMON_DEVICE_FEATURE);
        if offered_high & VIRTIO_F_VERSION_1 == 0 {
            transport.fail();
            return None;
        }
        transport.write_common_u32(COMMON_DRIVER_FEATURE_SELECT, 0);
        transport.write_common_u32(COMMON_DRIVER_FEATURE, offered_low & accepted_features_low);
        transport.write_common_u32(COMMON_DRIVER_FEATURE_SELECT, 1);
        transport.write_common_u32(
            COMMON_DRIVER_FEATURE,
            offered_high & (accepted_features_high | VIRTIO_F_VERSION_1),
        );
        transport.write_status(STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK);
        if transport.read_status() & STATUS_FEATURES_OK == 0 {
            transport.fail();
            return None;
        }
        Some(transport)
    }

    pub fn configure_queue(
        self,
        index: u16,
        memory: *mut QueueMemory,
    ) -> Result<ModernSplitQueue, QueueError> {
        let address = memory as u64;
        if address == 0 || address & 0xFFF != 0 {
            return Err(QueueError::DmaAddress);
        }
        self.write_common_u16(COMMON_QUEUE_SELECT, index);
        let size = self.read_common_u16(COMMON_QUEUE_SIZE);
        if size == 0 || size > MAX_QUEUE_SIZE || !size.is_power_of_two() {
            return Err(QueueError::UnsupportedSize(size));
        }
        let descriptor_bytes = size as usize * size_of::<Descriptor>();
        let avail_offset = descriptor_bytes;
        let used_offset = (avail_offset + 6 + size as usize * 2 + 4095) & !4095;
        let used_bytes = 6 + size as usize * 8;
        if used_offset.saturating_add(used_bytes) > QUEUE_BYTES {
            return Err(QueueError::UnsupportedSize(size));
        }
        unsafe { ptr::write_bytes(memory.cast::<u8>(), 0, size_of::<QueueMemory>()) };
        let notify_offset = self.read_common_u16(COMMON_QUEUE_NOTIFY_OFF) as usize;
        let notify_bytes = notify_offset
            .checked_mul(self.notify_multiplier as usize)
            .ok_or(QueueError::NotifyAddress)?;
        if notify_bytes.saturating_add(size_of::<u16>()) > self.notify_length {
            return Err(QueueError::NotifyAddress);
        }
        self.write_common_u64(COMMON_QUEUE_DESC, address);
        self.write_common_u64(COMMON_QUEUE_DRIVER, address + avail_offset as u64);
        self.write_common_u64(COMMON_QUEUE_DEVICE, address + used_offset as u64);
        self.write_common_u16(COMMON_QUEUE_ENABLE, 1);
        Ok(ModernSplitQueue {
            notify: unsafe { self.notify.add(notify_bytes).cast::<u16>() },
            index,
            size,
            memory,
            next_avail: 0,
            last_used: 0,
        })
    }

    pub fn finish(self) {
        self.write_status(self.read_status() | STATUS_DRIVER_OK);
    }

    pub fn fail(self) {
        self.write_status(self.read_status() | STATUS_FAILED);
    }

    pub fn reset(self) {
        self.write_status(0);
    }

    pub fn device_config_u32(self, offset: usize) -> Option<u32> {
        let (device, length) = self.device?;
        (offset.checked_add(size_of::<u32>())? <= length)
            .then(|| unsafe { ptr::read_volatile(device.add(offset).cast::<u32>()) })
    }

    fn read_status(self) -> u8 {
        unsafe { ptr::read_volatile(self.common.add(COMMON_DEVICE_STATUS)) }
    }

    fn write_status(self, value: u8) {
        unsafe { ptr::write_volatile(self.common.add(COMMON_DEVICE_STATUS), value) }
    }

    fn read_common_u16(self, offset: usize) -> u16 {
        unsafe { ptr::read_volatile(self.common.add(offset).cast::<u16>()) }
    }

    fn write_common_u16(self, offset: usize, value: u16) {
        unsafe { ptr::write_volatile(self.common.add(offset).cast::<u16>(), value) }
    }

    fn read_common_u32(self, offset: usize) -> u32 {
        unsafe { ptr::read_volatile(self.common.add(offset).cast::<u32>()) }
    }

    fn write_common_u32(self, offset: usize, value: u32) {
        unsafe { ptr::write_volatile(self.common.add(offset).cast::<u32>(), value) }
    }

    fn write_common_u64(self, offset: usize, value: u64) {
        unsafe { ptr::write_volatile(self.common.add(offset).cast::<u64>(), value) }
    }
}

fn find_modern_capability(function: pci::Function, requested_type: u8) -> Option<ModernCapability> {
    if function.read(0x04) & PCI_STATUS_CAPABILITIES == 0 {
        return None;
    }
    let mut offset = (function.read(0x34) as u8) & !3;
    let mut visited = [false; 64];
    for _ in 0..48 {
        if !(0x40..=0xE8).contains(&offset) {
            return None;
        }
        let slot = (offset / 4) as usize;
        if visited[slot] {
            return None;
        }
        visited[slot] = true;
        let header = function.read(offset);
        let id = header as u8;
        let next = ((header >> 8) as u8) & !3;
        let length = (header >> 16) as u8;
        let cfg_type = (header >> 24) as u8;
        if id == PCI_CAP_VENDOR && cfg_type == requested_type {
            let minimum = if requested_type == CFG_NOTIFY { 20 } else { 16 };
            if length < minimum {
                return None;
            }
            let bar = function.read(offset + 4) as u8;
            let capability_offset = function.read(offset + 8) as usize;
            let capability_length = function.read(offset + 12) as usize;
            let memory = function.memory_bar(bar)?;
            let address = (memory.address as usize).checked_add(capability_offset)?;
            if memory.address > u32::MAX as u64
                || address > u32::MAX as usize
                || capability_length == 0
                || address.checked_add(capability_length)? > 0x1_0000_0000
            {
                return None;
            }
            return Some(ModernCapability {
                address,
                length: capability_length,
                notify_multiplier: if requested_type == CFG_NOTIFY {
                    function.read(offset + 16)
                } else {
                    0
                },
            });
        }
        if next == 0 {
            return None;
        }
        offset = next;
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueError {
    DmaAddress,
    UnsupportedSize(u16),
    DescriptorOutOfRange,
    NotifyAddress,
}

pub struct SplitQueue {
    transport: LegacyTransport,
    index: u16,
    size: u16,
    memory: *mut QueueMemory,
    next_avail: u16,
    last_used: u16,
}

unsafe impl Send for SplitQueue {}

impl SplitQueue {
    const fn descriptor_offset(index: u16) -> usize {
        index as usize * size_of::<Descriptor>()
    }

    const fn avail_offset(&self) -> usize {
        self.size as usize * size_of::<Descriptor>()
    }

    const fn used_offset(&self) -> usize {
        (self.avail_offset() + 6 + self.size as usize * 2 + 4095) & !4095
    }

    fn base(&self) -> *mut u8 {
        self.memory.cast::<u8>()
    }

    pub const fn size(&self) -> u16 {
        self.size
    }

    pub fn set_descriptor(&mut self, index: u16, descriptor: Descriptor) -> Result<(), QueueError> {
        if index >= self.size {
            return Err(QueueError::DescriptorOutOfRange);
        }
        unsafe {
            ptr::write_volatile(
                self.base()
                    .add(Self::descriptor_offset(index))
                    .cast::<Descriptor>(),
                descriptor,
            );
        }
        Ok(())
    }

    pub fn submit(&mut self, head: u16) -> Result<u16, QueueError> {
        if head >= self.size {
            return Err(QueueError::DescriptorOutOfRange);
        }
        let ticket = self.next_avail;
        let ring = ticket & (self.size - 1);
        unsafe {
            let avail = self.base().add(self.avail_offset());
            ptr::write_volatile(avail.add(4 + ring as usize * 2).cast::<u16>(), head);
            core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
            self.next_avail = ticket.wrapping_add(1);
            ptr::write_volatile(avail.add(2).cast::<u16>(), self.next_avail);
            port::outw(self.transport.io_base + QUEUE_NOTIFY, self.index);
        }
        Ok(ticket)
    }

    pub fn pop_used(&mut self) -> Option<(u32, u32)> {
        unsafe {
            let used = self.base().add(self.used_offset());
            let device_index = ptr::read_volatile(used.add(2).cast::<u16>());
            if self.last_used == device_index {
                return None;
            }
            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
            let ring = self.last_used & (self.size - 1);
            let element = used.add(4 + ring as usize * 8);
            let id = ptr::read_volatile(element.cast::<u32>());
            let length = ptr::read_volatile(element.add(4).cast::<u32>());
            self.last_used = self.last_used.wrapping_add(1);
            Some((id, length))
        }
    }
}

pub struct ModernSplitQueue {
    notify: *mut u16,
    index: u16,
    size: u16,
    memory: *mut QueueMemory,
    next_avail: u16,
    last_used: u16,
}

unsafe impl Send for ModernSplitQueue {}

impl ModernSplitQueue {
    const fn descriptor_offset(index: u16) -> usize {
        index as usize * size_of::<Descriptor>()
    }

    const fn avail_offset(&self) -> usize {
        self.size as usize * size_of::<Descriptor>()
    }

    const fn used_offset(&self) -> usize {
        (self.avail_offset() + 6 + self.size as usize * 2 + 4095) & !4095
    }

    fn base(&self) -> *mut u8 {
        self.memory.cast::<u8>()
    }

    pub const fn size(&self) -> u16 {
        self.size
    }

    pub fn set_descriptor(&mut self, index: u16, descriptor: Descriptor) -> Result<(), QueueError> {
        if index >= self.size {
            return Err(QueueError::DescriptorOutOfRange);
        }
        unsafe {
            ptr::write_volatile(
                self.base()
                    .add(Self::descriptor_offset(index))
                    .cast::<Descriptor>(),
                descriptor,
            );
        }
        Ok(())
    }

    pub fn submit(&mut self, head: u16) -> Result<u16, QueueError> {
        if head >= self.size {
            return Err(QueueError::DescriptorOutOfRange);
        }
        let ticket = self.next_avail;
        let ring = ticket & (self.size - 1);
        unsafe {
            let avail = self.base().add(self.avail_offset());
            ptr::write_volatile(avail.add(4 + ring as usize * 2).cast::<u16>(), head);
            core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
            self.next_avail = ticket.wrapping_add(1);
            ptr::write_volatile(avail.add(2).cast::<u16>(), self.next_avail);
            ptr::write_volatile(self.notify, self.index);
        }
        Ok(ticket)
    }

    pub fn pop_used(&mut self) -> Option<(u32, u32)> {
        unsafe {
            let used = self.base().add(self.used_offset());
            let device_index = ptr::read_volatile(used.add(2).cast::<u16>());
            if self.last_used == device_index {
                return None;
            }
            core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
            let ring = self.last_used & (self.size - 1);
            let element = used.add(4 + ring as usize * 8);
            let id = ptr::read_volatile(element.cast::<u32>());
            let length = ptr::read_volatile(element.add(4).cast::<u32>());
            self.last_used = self.last_used.wrapping_add(1);
            Some((id, length))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_queue_layout_fits_bounded_dma_page_set() {
        let queue = SplitQueue {
            transport: LegacyTransport { io_base: 0 },
            index: 0,
            size: MAX_QUEUE_SIZE,
            memory: ptr::null_mut(),
            next_avail: 0,
            last_used: 0,
        };
        assert_eq!(queue.avail_offset(), 4096);
        assert_eq!(queue.used_offset(), 8192);
        assert!(queue.used_offset() + 6 + MAX_QUEUE_SIZE as usize * 8 <= QUEUE_BYTES);
    }
}
