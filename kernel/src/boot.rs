//! Versioned native firmware handoff and session startup policy.
//! The handoff is distinct from the CFC persistence/Genesis storage format.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub const UEFI_MAGIC: u32 = 0x4558_5055;
const INFO_MAGIC: u64 = 0x4558_504f_5345_4649;
static SINGLE_USER: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicU8 = AtomicU8::new(0);

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
}

pub fn single_user() -> bool {
    SINGLE_USER.load(Ordering::Acquire)
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
    assert_eq!(magic, 0x36d7_6289, "unknown firmware handoff");
    crate::slog!("EXPOS_BIOS_HANDOFF protocol=multiboot2\r\n");
}
