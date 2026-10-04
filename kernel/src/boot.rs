//! Versioned native firmware handoff and session startup policy.
//! The handoff is distinct from the CFC persistence/Genesis storage format.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};

pub const UEFI_MAGIC: u32 = 0x4558_5055;
const INFO_MAGIC: u64 = 0x4558_504f_5345_4649;
static SINGLE_USER: AtomicBool = AtomicBool::new(false);
static NATIVE_UEFI: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicU8 = AtomicU8::new(0);
static GOP_VALID: AtomicBool = AtomicBool::new(false);
static GOP_ADDRESS: AtomicU64 = AtomicU64::new(0);
static GOP_BYTES: AtomicU64 = AtomicU64::new(0);
static GOP_WIDTH: AtomicU32 = AtomicU32::new(0);
static GOP_HEIGHT: AtomicU32 = AtomicU32::new(0);
static GOP_STRIDE: AtomicU32 = AtomicU32::new(0);
static GOP_FORMAT: AtomicU32 = AtomicU32::new(u32::MAX);
static ACPI_RSDP: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirmwareFramebuffer {
    pub address: u64,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// UEFI GOP pixel format: zero is RGBX, one is BGRX.
    pub format: u32,
}

#[repr(C)]
struct NativeInfo {
    signature: u64,
    version: u32,
    size: u32,
    memory_map: u64,
    memory_map_size: u64,
    descriptor_size: u64,
    descriptor_version: u32,
    boot_mode: u32,
    framebuffer: u64,
    framebuffer_size: u64,
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
    rsdp: u64,
}

pub fn single_user() -> bool {
    SINGLE_USER.load(Ordering::Acquire)
}

pub fn native_uefi() -> bool {
    NATIVE_UEFI.load(Ordering::Acquire)
}

pub fn firmware_framebuffer() -> Option<FirmwareFramebuffer> {
    GOP_VALID
        .load(Ordering::Acquire)
        .then(|| FirmwareFramebuffer {
            address: GOP_ADDRESS.load(Ordering::Relaxed),
            bytes: GOP_BYTES.load(Ordering::Relaxed),
            width: GOP_WIDTH.load(Ordering::Relaxed),
            height: GOP_HEIGHT.load(Ordering::Relaxed),
            stride: GOP_STRIDE.load(Ordering::Relaxed),
            format: GOP_FORMAT.load(Ordering::Relaxed),
        })
}

pub fn acpi_rsdp() -> Option<u64> {
    let address = ACPI_RSDP.load(Ordering::Acquire);
    (address != 0).then_some(address)
}

/// Set once at startup. Login/logout cannot enable services disabled by boot.
pub fn set_single_user(value: bool) {
    SINGLE_USER.store(value, Ordering::Release);
}

pub fn requested_mode() -> Option<crate::session::BootMode> {
    match REQUESTED.load(Ordering::Acquire) {
        1 => Some(crate::session::BootMode::SingleUser),
        2 => Some(crate::session::BootMode::Console),
        3 => Some(crate::session::BootMode::Graphical),
        _ => None,
    }
}

/// Validate the native loader's fixed-width, identity-mapped handoff.
/// The firmware memory map stays reserved and is handed to future ASL memory
/// ownership work; the current allocator uses a separately reserved kernel heap.
pub unsafe fn initialize(magic: u32, address: u64) {
    if magic == UEFI_MAGIC {
        NATIVE_UEFI.store(true, Ordering::Release);
        assert!(
            address >= 0x1000
                && address <= 0x4000_0000 - core::mem::size_of::<NativeInfo>() as u64
                && address.is_multiple_of(core::mem::align_of::<NativeInfo>() as u64)
        );
        let info = &*(address as *const NativeInfo);
        assert_eq!(
            info.signature, INFO_MAGIC,
            "invalid native handoff signature"
        );
        assert_eq!(info.version, 1, "unsupported native handoff version");
        assert!(info.size as usize >= core::mem::size_of::<NativeInfo>());
        assert!(info.descriptor_size >= 40 && info.descriptor_size <= 4096);
        assert!(info.memory_map >= 0x1000 && info.memory_map < 0x4000_0000);
        assert!(info.memory_map_size <= 0x4000_0000 - info.memory_map);
        assert_eq!(info.memory_map_size % info.descriptor_size, 0);
        assert!(info.boot_mode <= 3);
        REQUESTED.store(info.boot_mode as u8, Ordering::Release);
        if valid_framebuffer(info) {
            GOP_ADDRESS.store(info.framebuffer, Ordering::Relaxed);
            GOP_BYTES.store(info.framebuffer_size, Ordering::Relaxed);
            GOP_WIDTH.store(info.width, Ordering::Relaxed);
            GOP_HEIGHT.store(info.height, Ordering::Relaxed);
            GOP_STRIDE.store(info.stride, Ordering::Relaxed);
            GOP_FORMAT.store(info.format, Ordering::Relaxed);
            GOP_VALID.store(true, Ordering::Release);
        }
        if info.rsdp >= 0x1000 && info.rsdp < 0x1_0000_0000 {
            ACPI_RSDP.store(info.rsdp, Ordering::Release);
        }
        crate::slog!("EXPOS_UEFI_HANDOFF version=1 boot_services=exited descriptors={} descriptor_size={}\r\n", info.memory_map_size / info.descriptor_size, info.descriptor_size);
        crate::slog!(
            "EXPOS_UEFI_GOP width={} height={} stride={} format={} address={:#x} bytes={}\r\n",
            info.width,
            info.height,
            info.stride,
            info.format,
            info.framebuffer,
            info.framebuffer_size
        );
        return;
    }
    NATIVE_UEFI.store(false, Ordering::Release);
    GOP_VALID.store(false, Ordering::Release);
    assert_eq!(magic, 0x36d7_6289, "unknown firmware handoff");
    if let Some(rsdp) = multiboot_rsdp(address) {
        ACPI_RSDP.store(rsdp, Ordering::Release);
    }
    crate::slog!("EXPOS_BIOS_HANDOFF protocol=multiboot2\r\n");
}

unsafe fn multiboot_rsdp(address: u64) -> Option<u64> {
    if !(0x1000..0x1_0000_0000).contains(&address) {
        return None;
    }
    let total = core::ptr::read_unaligned(address as *const u32) as u64;
    if !(16..=16 * 1024 * 1024).contains(&total) || address.checked_add(total)? > 0x1_0000_0000 {
        return None;
    }
    let mut cursor = address + 8;
    while cursor + 8 <= address + total {
        let kind = core::ptr::read_unaligned(cursor as *const u32);
        let size = core::ptr::read_unaligned((cursor + 4) as *const u32) as u64;
        if size < 8 || cursor + size > address + total {
            return None;
        }
        if matches!(kind, 14 | 15) && size >= 28 {
            return Some(cursor + 8);
        }
        if kind == 0 {
            break;
        }
        cursor = (cursor + size + 7) & !7;
    }
    None
}

fn valid_framebuffer(info: &NativeInfo) -> bool {
    if info.framebuffer < 0x10_0000
        || info.framebuffer >= 0x1_0000_0000
        || info.framebuffer_size == 0
        || info.width == 0
        || info.height == 0
        || info.stride < info.width
        || !matches!(info.format, 0 | 1)
    {
        return false;
    }
    let Some(required) = (info.stride as u64)
        .checked_mul(info.height as u64)
        .and_then(|pixels| pixels.checked_mul(4))
    else {
        return false;
    };
    required <= info.framebuffer_size
        && info
            .framebuffer
            .checked_add(info.framebuffer_size)
            .is_some_and(|end| end <= 0x1_0000_0000)
}
