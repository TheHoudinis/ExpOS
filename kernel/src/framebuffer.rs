use crate::port;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8, AtomicUsize, Ordering};

/// Maximum scanout geometry supported by the mapped Bochs/QEMU framebuffer.
///
/// These compatibility constants intentionally describe the largest mode, not
/// the mode that is currently selected. New code should use [`width`],
/// [`height`], and [`stride_bytes`] for runtime layout and drawing.
pub const WIDTH: usize = 1920;
pub const HEIGHT: usize = 1080;
pub const MAX_WIDTH: usize = WIDTH;
pub const MAX_HEIGHT: usize = HEIGHT;
pub const BITS_PER_PIXEL: usize = 32;
pub const BYTES_PER_PIXEL: usize = BITS_PER_PIXEL / 8;
pub const STRIDE_BYTES: usize = WIDTH * BYTES_PER_PIXEL;
pub const SCANOUT_BYTES: usize = STRIDE_BYTES * HEIGHT;

// Maximum VRAM used by this driver. The physical aperture is discovered from
// the supported adapter's PCI BAR0: OVMF and SeaBIOS assign different addresses.
// The bootstrap identity-maps the lower 4 GiB, including both placements.
pub const LFB_APERTURE_BYTES: usize = 16 * 1024 * 1024;
const _: () = assert!(SCANOUT_BYTES <= LFB_APERTURE_BYTES);

/// Firmware GOP normally exposes only one visible scanout. Rendering directly
/// into that memory makes the user watch the compositor rebuild the wallpaper,
/// windows, taskbar, and cursor one primitive at a time. Keep a kernel-owned
/// shadow surface and publish only completed damage during `present_damage`.
/// The buffer is sized to the same bounded aperture accepted by this driver,
/// so validated GOP strides cannot overrun it.
#[repr(C, align(64))]
struct ShadowBuffer([u32; LFB_APERTURE_BYTES / BYTES_PER_PIXEL]);

static mut GOP_SHADOW: ShadowBuffer = ShadowBuffer([0; LFB_APERTURE_BYTES / BYTES_PER_PIXEL]);

static LFB_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static LFB_DEVICE_BYTES: AtomicUsize = AtomicUsize::new(0);
static GOP_ACTIVE: AtomicBool = AtomicBool::new(false);
static GOP_RGB_ORDER: AtomicBool = AtomicBool::new(false);
static ACTIVE_WIDTH: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_HEIGHT: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_STRIDE_BYTES: AtomicUsize = AtomicUsize::new(0);
const VBE_INDEX: u16 = 0x01CE;
const VBE_DATA: u16 = 0x01CF;

const INDEX_ID: u16 = 0;
const INDEX_XRES: u16 = 1;
const INDEX_YRES: u16 = 2;
const INDEX_BPP: u16 = 3;
const INDEX_ENABLE: u16 = 4;
const INDEX_VIRT_WIDTH: u16 = 6;
const INDEX_VIRT_HEIGHT: u16 = 7;
const INDEX_X_OFFSET: u16 = 8;
const INDEX_Y_OFFSET: u16 = 9;
const INDEX_VIDEO_MEMORY_64K: u16 = 10;
const DISABLED: u16 = 0;
const ENABLED: u16 = 0x01;
const LFB_ENABLED: u16 = 0x40;
const NO_ACTIVE_MODE: u8 = u8::MAX;
const VGA_INPUT_STATUS: u16 = 0x03DA;
const VGA_VERTICAL_RETRACE: u8 = 1 << 3;
const VBLANK_POLL_LIMIT: usize = 250_000;
/// Upper bound for damage bookkeeping on the kernel stack.
///
/// Desktop commits normally submit only a handful of regions. If a caller
/// exceeds this bound, normalization safely collapses all damage into one
/// bounding rectangle instead of allocating or losing pixels.
const MAX_DAMAGE_REGIONS: usize = 32;
/// A partial GOP stream is periodically reconciled from the complete shadow
/// image. This bounds the lifetime of any caller-side missing damage without
/// turning normal cursor motion into full-screen traffic.
const GOP_RECONCILE_INTERVAL: u64 = 120;
/// Above this visible-pixel ratio one sequential full publication is cheaper
/// and simpler than many row fragments.
const FULL_DAMAGE_PROMOTION_NUMERATOR: u64 = 2;
const FULL_DAMAGE_PROMOTION_DENOMINATOR: u64 = 3;

const FONT_FACE_MASK: u8 = 0b0000_0111;
const FONT_WEIGHT_SHIFT: u8 = 3;
const FONT_WEIGHT_MASK: u8 = 0b0001_1000;
const DEFAULT_FONT_STYLE: u8 =
    FontFace::System as u8 | ((FontWeight::Regular as u8) << FONT_WEIGHT_SHIFT);

/// A compact, built-in bitmap face used by all graphical text.
///
/// The discriminants are stable because desktop preferences persist them.
/// Every face remains inside the same 8x8 cell, so changing the face cannot
/// invalidate existing layouts or push a terminal column beyond 480p.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FontFace {
    /// The original IBM-style ExpOS bitmap face.
    #[default]
    System = 0,
    /// Softened cap and baseline terminals.
    Rounded = 1,
    /// Expanded top and baseline terminals.
    Serif = 2,
    /// A narrow six-pixel drawing centered in the standard cell.
    Compact = 3,
    /// A sheared, italic-like drawing.
    Slanted = 4,
}

impl FontFace {
    pub const fn persisted(self) -> u8 {
        self as u8
    }

    pub const fn from_persisted(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::System),
            1 => Some(Self::Rounded),
            2 => Some(Self::Serif),
            3 => Some(Self::Compact),
            4 => Some(Self::Slanted),
            _ => None,
        }
    }
}

/// Stroke weight applied after the selected face transformation.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FontWeight {
    Light = 0,
    #[default]
    Regular = 1,
    Bold = 2,
}

impl FontWeight {
    pub const fn persisted(self) -> u8 {
        self as u8
    }

    pub const fn from_persisted(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Light),
            1 => Some(Self::Regular),
            2 => Some(Self::Bold),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FontStyle {
    pub face: FontFace,
    pub weight: FontWeight,
}

impl FontStyle {
    pub const fn new(face: FontFace, weight: FontWeight) -> Self {
        Self { face, weight }
    }

    const fn encoded(self) -> u8 {
        self.face.persisted() | (self.weight.persisted() << FONT_WEIGHT_SHIFT)
    }

    fn decode(value: u8) -> Self {
        let face = FontFace::from_persisted(value & FONT_FACE_MASK).unwrap_or_default();
        let weight = FontWeight::from_persisted((value & FONT_WEIGHT_MASK) >> FONT_WEIGHT_SHIFT)
            .unwrap_or_default();
        Self { face, weight }
    }
}

/// A user-selectable progressive display mode.
///
/// The discriminants are stable because settings persistence may store them.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DisplayMode {
    #[default]
    P480 = 0,
    P720 = 1,
    P1080 = 2,
}

impl DisplayMode {
    pub const ALL: [Self; 3] = [Self::P480, Self::P720, Self::P1080];

    pub const fn width(self) -> usize {
        match self {
            Self::P480 => 640,
            Self::P720 => 1280,
            Self::P1080 => 1920,
        }
    }

    pub const fn height(self) -> usize {
        match self {
            Self::P480 => 480,
            Self::P720 => 720,
            Self::P1080 => 1080,
        }
    }

    pub const fn dimensions(self) -> (usize, usize) {
        (self.width(), self.height())
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::P480 => "480p",
            Self::P720 => "720p",
            Self::P1080 => "1080p",
        }
    }

    pub const fn stride_bytes(self) -> usize {
        self.width() * BYTES_PER_PIXEL
    }

    pub const fn scanout_bytes(self) -> usize {
        self.stride_bytes() * self.height()
    }

    pub const fn double_buffer_bytes(self) -> usize {
        self.scanout_bytes() * 2
    }

    pub const fn fits_aperture(self) -> bool {
        self.width() <= MAX_WIDTH
            && self.height() <= MAX_HEIGHT
            && self.scanout_bytes() <= LFB_APERTURE_BYTES
    }

    pub const fn hardware_mode(self) -> Mode {
        Mode {
            width: self.width() as u16,
            height: self.height() as u16,
            bits_per_pixel: BITS_PER_PIXEL as u16,
            virtual_width: self.width() as u16,
            virtual_height: (self.height() * 2) as u16,
        }
    }

    pub const fn from_dimensions(width: usize, height: usize) -> Option<Self> {
        match (width, height) {
            (640, 480) => Some(Self::P480),
            (1280, 720) => Some(Self::P720),
            (1920, 1080) => Some(Self::P1080),
            _ => None,
        }
    }

    pub const fn from_persisted(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::P480),
            1 => Some(Self::P720),
            2 => Some(Self::P1080),
            _ => None,
        }
    }

    pub const fn persisted(self) -> u8 {
        self as u8
    }
}

const _: () = assert!(DisplayMode::P480.fits_aperture());
const _: () = assert!(DisplayMode::P720.fits_aperture());
const _: () = assert!(DisplayMode::P1080.fits_aperture());
const _: () = assert!(DisplayMode::P480.double_buffer_bytes() <= LFB_APERTURE_BYTES);
const _: () = assert!(DisplayMode::P720.double_buffer_bytes() <= LFB_APERTURE_BYTES);
const _: () = assert!(DisplayMode::P1080.double_buffer_bytes() <= LFB_APERTURE_BYTES);

static REQUESTED_MODE: AtomicU8 = AtomicU8::new(DisplayMode::P480 as u8);
static ACTIVE_FONT_STYLE: AtomicU8 = AtomicU8::new(DEFAULT_FONT_STYLE);
static ACTIVE_DISPLAY_MODE: AtomicU8 = AtomicU8::new(NO_ACTIVE_MODE);
static PAGE_FLIP_AVAILABLE: AtomicBool = AtomicBool::new(false);
static DRAW_Y: AtomicU16 = AtomicU16::new(0);
static FRONT_Y: AtomicU16 = AtomicU16::new(0);
static HARDWARE_Y_OFFSET: AtomicU16 = AtomicU16::new(0);
static FRONT_CONTENT_VISIBLE: AtomicBool = AtomicBool::new(false);
static PRESENTED_FRAMES: AtomicU64 = AtomicU64::new(0);
static VBLANK_TIMEOUTS: AtomicU64 = AtomicU64::new(0);
static PAGE_FLIP_FAILURES: AtomicU64 = AtomicU64::new(0);
static SUBMITTED_DAMAGE_REGIONS: AtomicU64 = AtomicU64::new(0);
static SUBMITTED_DAMAGE_PIXELS: AtomicU64 = AtomicU64::new(0);
static COPIED_DAMAGE_REGIONS: AtomicU64 = AtomicU64::new(0);
static COPIED_DAMAGE_PIXELS: AtomicU64 = AtomicU64::new(0);
static DAMAGE_COLLAPSES: AtomicU64 = AtomicU64::new(0);
static DAMAGE_PROMOTIONS: AtomicU64 = AtomicU64::new(0);
static EMPTY_PRESENTS: AtomicU64 = AtomicU64::new(0);
static GOP_FULL_PRESENTS: AtomicU64 = AtomicU64::new(0);
static GOP_PARTIAL_PRESENTS: AtomicU64 = AtomicU64::new(0);
static GOP_READBACK_FAILURES: AtomicU64 = AtomicU64::new(0);
static GOP_RECOVERIES: AtomicU64 = AtomicU64::new(0);
static GOP_PARTIAL_SINCE_FULL: AtomicU64 = AtomicU64::new(0);
static FORCE_FULL_RECONCILE: AtomicBool = AtomicBool::new(false);
static LAST_COPY_TICKS: AtomicU64 = AtomicU64::new(0);
static MAX_COPY_TICKS: AtomicU64 = AtomicU64::new(0);
static LAST_COPIED_REGIONS: AtomicU64 = AtomicU64::new(0);
static LAST_COPIED_PIXELS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mode {
    pub width: u16,
    pub height: u16,
    pub bits_per_pixel: u16,
    pub virtual_width: u16,
    pub virtual_height: u16,
}

impl Mode {
    pub const fn stride_bytes(self) -> usize {
        self.virtual_width as usize * self.bits_per_pixel as usize / 8
    }

    pub const fn scanout_bytes(self) -> usize {
        self.stride_bytes() * self.height as usize
    }

    pub const fn aperture_bytes(self) -> usize {
        self.stride_bytes() * self.virtual_height as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationStats {
    pub frames: u64,
    pub vblank_timeouts: u64,
    pub page_flip_failures: u64,
    pub page_flip_available: bool,
    /// Last hardware-read value of the VBE Y-offset register.
    pub hardware_y_offset: u16,
    /// True only after presented pixels are known to occupy the hardware front page.
    pub visible_content: bool,
    /// Damage rectangles supplied by callers, including clipped-out entries.
    pub submitted_regions: u64,
    /// Visible pixels represented by submitted rectangles before coalescing.
    /// Overlapping submissions therefore contribute more than once here.
    pub submitted_pixels: u64,
    /// Normalized, non-overlapping rectangles actually copied between pages.
    pub copied_regions: u64,
    /// Pixels actually copied between pages after clipping and coalescing.
    pub copied_pixels: u64,
    /// Frames whose damage exceeded the bounded region set and was collapsed
    /// into one safe bounding rectangle.
    pub damage_collapses: u64,
    /// Partial damage promoted to a full publication by the v3 cost model or
    /// periodic reconciliation policy.
    pub damage_promotions: u64,
    pub empty_presents: u64,
    pub gop_full_presents: u64,
    pub gop_partial_presents: u64,
    pub gop_readback_failures: u64,
    pub gop_recoveries: u64,
    pub last_copy_ticks: u64,
    pub max_copy_ticks: u64,
    pub last_copied_regions: u64,
    pub last_copied_pixels: u64,
}

/// A clipped scanout area that must be copied to the newly hidden page after
/// a flip. Keeping the two pages coherent only where pixels changed avoids a
/// full 16 MiB read/write round trip for cursor and terminal updates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DamageRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl DamageRegion {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn union(self, other: Self) -> Self {
        let left = if self.x < other.x { self.x } else { other.x };
        let top = if self.y < other.y { self.y } else { other.y };
        let self_right = self.x.saturating_add(self.width);
        let other_right = other.x.saturating_add(other.width);
        let right = if self_right > other_right {
            self_right
        } else {
            other_right
        };
        let self_bottom = self.y.saturating_add(self.height);
        let other_bottom = other.y.saturating_add(other.height);
        let bottom = if self_bottom > other_bottom {
            self_bottom
        } else {
            other_bottom
        };
        Self::new(
            left,
            top,
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClippedRegion {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

impl ClippedRegion {
    const EMPTY: Self = Self {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };

    const fn width(self) -> usize {
        self.right - self.left
    }

    const fn height(self) -> usize {
        self.bottom - self.top
    }

    const fn pixels(self) -> usize {
        self.width() * self.height()
    }

    const fn touches_or_overlaps(self, other: Self) -> bool {
        self.left <= other.right
            && other.left <= self.right
            && self.top <= other.bottom
            && other.top <= self.bottom
    }

    const fn union(self, other: Self) -> Self {
        Self {
            left: if self.left < other.left {
                self.left
            } else {
                other.left
            },
            top: if self.top < other.top {
                self.top
            } else {
                other.top
            },
            right: if self.right > other.right {
                self.right
            } else {
                other.right
            },
            bottom: if self.bottom > other.bottom {
                self.bottom
            } else {
                other.bottom
            },
        }
    }

    const fn can_merge_without_extra_copy(self, other: Self) -> bool {
        if !self.touches_or_overlaps(other) {
            return false;
        }
        let union_pixels = self.union(other).pixels();
        let separate_pixels = self.pixels().saturating_add(other.pixels());
        union_pixels <= separate_pixels
    }
}

#[derive(Debug, Eq, PartialEq)]
struct NormalizedDamage {
    regions: [ClippedRegion; MAX_DAMAGE_REGIONS],
    len: usize,
    submitted_regions: u64,
    submitted_pixels: u64,
    collapsed: bool,
}

impl NormalizedDamage {
    const fn empty() -> Self {
        Self {
            regions: [ClippedRegion::EMPTY; MAX_DAMAGE_REGIONS],
            len: 0,
            submitted_regions: 0,
            submitted_pixels: 0,
            collapsed: false,
        }
    }

    fn copied_regions(&self) -> u64 {
        self.len as u64
    }

    fn copied_pixels(&self) -> u64 {
        self.regions[..self.len]
            .iter()
            .fold(0_u64, |total, region| {
                total.saturating_add(region.pixels() as u64)
            })
    }

    fn remove(&mut self, index: usize) {
        for current in index..self.len - 1 {
            self.regions[current] = self.regions[current + 1];
        }
        self.len -= 1;
        self.regions[self.len] = ClippedRegion::EMPTY;
    }

    fn collapse_with(&mut self, candidate: ClippedRegion) {
        let mut combined = candidate;
        for region in &self.regions[..self.len] {
            combined = combined.union(*region);
        }
        self.regions = [ClippedRegion::EMPTY; MAX_DAMAGE_REGIONS];
        self.regions[0] = combined;
        self.len = 1;
        self.collapsed = true;
    }

    fn add(&mut self, mut candidate: ClippedRegion) {
        if self.collapsed {
            self.regions[0] = self.regions[0].union(candidate);
            return;
        }

        // Restart after every merge: growing the candidate can make another
        // bounding merge cost-effective. This reaches a fixed point while the
        // hard region bound keeps work and stack use deterministic.
        let mut index = 0;
        while index < self.len {
            if candidate.can_merge_without_extra_copy(self.regions[index]) {
                candidate = candidate.union(self.regions[index]);
                self.remove(index);
                index = 0;
            } else {
                index += 1;
            }
        }

        if self.len == MAX_DAMAGE_REGIONS {
            self.collapse_with(candidate);
        } else {
            self.regions[self.len] = candidate;
            self.len += 1;
        }
    }
}

fn clip_region(region: DamageRegion, mode_width: i32, mode_height: i32) -> Option<ClippedRegion> {
    let left = region.x.max(0).min(mode_width);
    let top = region.y.max(0).min(mode_height);
    let right = region.x.saturating_add(region.width).max(0).min(mode_width);
    let bottom = region
        .y
        .saturating_add(region.height)
        .max(0)
        .min(mode_height);
    if right <= left || bottom <= top {
        return None;
    }
    Some(ClippedRegion {
        left: left as usize,
        top: top as usize,
        right: right as usize,
        bottom: bottom as usize,
    })
}

fn normalize_damage(
    damage: &[DamageRegion],
    mode_width: i32,
    mode_height: i32,
) -> NormalizedDamage {
    let mut normalized = NormalizedDamage::empty();
    normalized.submitted_regions = damage.len() as u64;
    for region in damage {
        let Some(clipped) = clip_region(*region, mode_width, mode_height) else {
            continue;
        };
        normalized.submitted_pixels = normalized
            .submitted_pixels
            .saturating_add(clipped.pixels() as u64);
        normalized.add(clipped);
    }
    normalized
}

pub mod color {
    pub const BACKGROUND: u32 = 0x0010_1212;
    pub const PANEL: u32 = 0x0018_1B1B;
    pub const WINDOW: u32 = 0x0012_1414;
    pub const INK: u32 = 0x00D9_DDD9;
    pub const MUTED: u32 = 0x0080_8984;
    pub const PURPLE: u32 = 0x006D_8494;
    pub const GREEN: u32 = 0x006F_9B73;
    pub const CYAN: u32 = 0x0074_949A;
    pub const RED: u32 = 0x00B7_6E6E;
    pub const WHITE: u32 = 0x00F0_F2F0;
    pub const BORDER: u32 = 0x0035_3A38;
}

pub fn available() -> bool {
    let id = read(INDEX_ID);
    (0xB0C0..=0xB0C5).contains(&id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FramebufferAperture {
    address: usize,
    bytes: usize,
}

/// Decode the memory BAR of a supported VGA adapter without probing or moving
/// a live PCI resource. QEMU/Bochs reports its VRAM size in VBE register 10.
fn decode_aperture(bar_low: u32, bar_high: u32, memory_64k: u16) -> Option<FramebufferAperture> {
    if bar_low & 1 != 0 || memory_64k == 0 || memory_64k == u16::MAX {
        return None;
    }
    let address = match (bar_low >> 1) & 3 {
        0 => (bar_low & !0xF) as u64,
        2 => ((bar_high as u64) << 32) | (bar_low & !0xF) as u64,
        // Below-1-MiB and reserved encodings cannot describe our aperture.
        _ => return None,
    };
    let bytes = (memory_64k as u64) << 16;
    // This driver supports PCI MMIO above the kernel's low 1-GiB RAM window.
    // BAR alignment and size must agree, and the entire device must be mapped.
    if !bytes.is_power_of_two()
        || address < 0x4000_0000
        || !address.is_multiple_of(bytes)
        || address.checked_add(bytes)? > 0x1_0000_0000
    {
        return None;
    }
    Some(FramebufferAperture {
        address: address as usize,
        bytes: bytes as usize,
    })
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

fn discover_aperture() -> bool {
    if LFB_ADDRESS.load(Ordering::Acquire) != 0 {
        return true;
    }
    for bus in 0..=u8::MAX {
        for slot in 0..32 {
            if pci_read(bus, slot, 0, 0) as u16 == 0xFFFF {
                continue;
            }
            let functions = if pci_read(bus, slot, 0, 0x0C) & 0x0080_0000 != 0 {
                8
            } else {
                1
            };
            for function in 0..functions {
                // Standard QEMU/Bochs VGA only. A VBE-compatible register ID
                // by itself does not establish which PCI device owns VRAM.
                if pci_read(bus, slot, function, 0) != 0x1111_1234
                    || pci_read(bus, slot, function, 0x08) >> 16 != 0x0300
                    || pci_read(bus, slot, function, 0x04) & 2 == 0
                {
                    continue;
                }
                let Some(aperture) = decode_aperture(
                    pci_read(bus, slot, function, 0x10),
                    pci_read(bus, slot, function, 0x14),
                    read(INDEX_VIDEO_MEMORY_64K),
                ) else {
                    continue;
                };
                LFB_DEVICE_BYTES.store(aperture.bytes, Ordering::Relaxed);
                LFB_ADDRESS.store(aperture.address, Ordering::Release);
                crate::slog!(
                    "EXPOS_FRAMEBUFFER_READY source=pci-bar0 bus={} slot={} function={} address={:#x} bytes={}\r\n",
                    bus, slot, function, aperture.address, aperture.bytes
                );
                return true;
            }
        }
    }
    crate::slog!("EXPOS_FRAMEBUFFER_UNAVAILABLE reason=no-safe-supported-pci-aperture\r\n");
    false
}

fn framebuffer_pointer() -> Option<*mut u32> {
    active_display_mode()?;
    if GOP_ACTIVE.load(Ordering::Acquire) {
        // SAFETY: the compositor owns the shadow surface while graphics mode
        // is active; presentation only reads it after drawing completes.
        return Some(unsafe { core::ptr::addr_of_mut!(GOP_SHADOW.0).cast::<u32>() });
    }
    scanout_pointer()
}

fn scanout_pointer() -> Option<*mut u32> {
    let address = LFB_ADDRESS.load(Ordering::Acquire);
    (address != 0).then_some(address as *mut u32)
}

/// Select the mode that the next [`enter`] call will program.
///
/// Requesting a mode while graphics are active does not silently invalidate
/// the current scanout. The current mode remains active until the caller exits
/// or calls [`enter`] again, at which point the requested mode is applied.
pub fn request_mode(mode: DisplayMode) -> bool {
    if !mode.fits_aperture() {
        return false;
    }
    REQUESTED_MODE.store(mode.persisted(), Ordering::Release);
    true
}

pub fn requested_mode() -> DisplayMode {
    DisplayMode::from_persisted(REQUESTED_MODE.load(Ordering::Acquire)).unwrap_or_default()
}

/// Return the mode whose geometry is safe for current framebuffer access.
pub fn active_display_mode() -> Option<DisplayMode> {
    DisplayMode::from_persisted(ACTIVE_DISPLAY_MODE.load(Ordering::Acquire))
}

/// Return the active mode, or the requested mode before graphics are entered.
///
/// Layout code can use this before and after [`enter`]. Drawing code normally
/// calls it while a mode is active, so an un-applied request cannot change its
/// stride underneath the current scanout.
pub fn current_mode() -> DisplayMode {
    active_display_mode().unwrap_or_else(requested_mode)
}

/// Human-readable active output. Firmware GOP geometry may not match one of
/// the three user-selectable Bochs presets.
pub fn active_output_label() -> &'static str {
    if GOP_ACTIVE.load(Ordering::Acquire) {
        "firmware"
    } else {
        current_mode().label()
    }
}

pub fn width() -> usize {
    let active = ACTIVE_WIDTH.load(Ordering::Acquire);
    if active == 0 {
        current_mode().width()
    } else {
        active
    }
}

pub fn height() -> usize {
    let active = ACTIVE_HEIGHT.load(Ordering::Acquire);
    if active == 0 {
        current_mode().height()
    } else {
        active
    }
}

pub fn stride_bytes() -> usize {
    let active = ACTIVE_STRIDE_BYTES.load(Ordering::Acquire);
    if active == 0 {
        current_mode().stride_bytes()
    } else {
        active
    }
}

pub fn scanout_bytes() -> usize {
    stride_bytes() * height()
}

/// Return the font style used by subsequent [`text`] and [`glyph`] calls.
pub fn font_style() -> FontStyle {
    FontStyle::decode(ACTIVE_FONT_STYLE.load(Ordering::Acquire))
}

/// Atomically change both face and stroke weight for subsequent drawing.
/// Existing pixels are not redrawn; the compositor decides which surfaces to
/// damage after applying a preference.
pub fn set_font_style(style: FontStyle) {
    ACTIVE_FONT_STYLE.store(style.encoded(), Ordering::Release);
}

pub fn presentation_stats() -> PresentationStats {
    PresentationStats {
        frames: PRESENTED_FRAMES.load(Ordering::Acquire),
        vblank_timeouts: VBLANK_TIMEOUTS.load(Ordering::Acquire),
        page_flip_failures: PAGE_FLIP_FAILURES.load(Ordering::Acquire),
        page_flip_available: PAGE_FLIP_AVAILABLE.load(Ordering::Acquire),
        hardware_y_offset: HARDWARE_Y_OFFSET.load(Ordering::Acquire),
        visible_content: FRONT_CONTENT_VISIBLE.load(Ordering::Acquire),
        submitted_regions: SUBMITTED_DAMAGE_REGIONS.load(Ordering::Acquire),
        submitted_pixels: SUBMITTED_DAMAGE_PIXELS.load(Ordering::Acquire),
        copied_regions: COPIED_DAMAGE_REGIONS.load(Ordering::Acquire),
        copied_pixels: COPIED_DAMAGE_PIXELS.load(Ordering::Acquire),
        damage_collapses: DAMAGE_COLLAPSES.load(Ordering::Acquire),
        damage_promotions: DAMAGE_PROMOTIONS.load(Ordering::Acquire),
        empty_presents: EMPTY_PRESENTS.load(Ordering::Acquire),
        gop_full_presents: GOP_FULL_PRESENTS.load(Ordering::Acquire),
        gop_partial_presents: GOP_PARTIAL_PRESENTS.load(Ordering::Acquire),
        gop_readback_failures: GOP_READBACK_FAILURES.load(Ordering::Acquire),
        gop_recoveries: GOP_RECOVERIES.load(Ordering::Acquire),
        last_copy_ticks: LAST_COPY_TICKS.load(Ordering::Acquire),
        max_copy_ticks: MAX_COPY_TICKS.load(Ordering::Acquire),
        last_copied_regions: LAST_COPIED_REGIONS.load(Ordering::Acquire),
        last_copied_pixels: LAST_COPIED_PIXELS.load(Ordering::Acquire),
    }
}

/// Ask the next GOP presentation to reconcile the complete shadow image.
/// Used by the v3 debug portal and safe recovery command.
pub fn request_full_reconcile() {
    FORCE_FULL_RECONCILE.store(true, Ordering::Release);
}

/// Return the raw geometry reported by the adapter while graphics are enabled.
/// Callers can distinguish the requested mode from the active hardware mode
/// instead of assuming that register programming succeeded.
pub fn active_mode() -> Option<Mode> {
    if !available() || read(INDEX_ENABLE) & ENABLED == 0 {
        return None;
    }
    Some(read_mode())
}

pub fn enter() -> bool {
    let requested = requested_mode();
    crate::slog!("EXPOS_DISPLAY_ENTER requested={}\r\n", requested.label());
    if program_firmware_framebuffer() {
        return true;
    }
    if program_mode(requested) {
        return true;
    }
    if requested != DisplayMode::P480 {
        crate::slog!(
            "EXPOS_DISPLAY_FALLBACK from={} to=480p reason=mode-rejected\r\n",
            requested.label()
        );
        REQUESTED_MODE.store(DisplayMode::P480.persisted(), Ordering::Release);
        if program_mode(DisplayMode::P480) {
            return true;
        }
    }
    crate::slog!("EXPOS_DISPLAY_UNAVAILABLE reason=no-supported-vbe-mode\r\n");
    false
}

pub fn print_diagnostics() {
    let adapter_id = read(INDEX_ID);
    let requested = requested_mode();
    let active = active_display_mode();
    let presentation = presentation_stats();
    crate::println!("DISPLAY DIAGNOSTICS");
    crate::println!(
        "adapter: id={:#06X} bochs-vbe={} uefi-gop={}",
        adapter_id,
        available(),
        GOP_ACTIVE.load(Ordering::Acquire)
    );
    crate::println!(
        "requested: {} {}x{}x{} required={} bytes double-buffer={} bytes",
        requested.label(),
        requested.width(),
        requested.height(),
        BITS_PER_PIXEL,
        requested.scanout_bytes(),
        requested.double_buffer_bytes()
    );
    if let Some(mode) = active {
        crate::println!(
            "active: {} {}x{} front-y={} draw-y={}",
            mode.label(),
            mode.width(),
            mode.height(),
            FRONT_Y.load(Ordering::Acquire),
            DRAW_Y.load(Ordering::Acquire)
        );
    } else {
        crate::println!("active: text mode (graphical scanout disabled)");
    }
    crate::println!(
        "presentation: pageflip={} hardware-y={} visible={} frames={} vblank-timeouts={} flip-failures={}",
        presentation.page_flip_available,
        presentation.hardware_y_offset,
        presentation.visible_content,
        presentation.frames,
        presentation.vblank_timeouts,
        presentation.page_flip_failures
    );
    crate::println!(
        "damage: submitted-regions={} submitted-pixels={} copied-regions={} copied-pixels={} collapses={}",
        presentation.submitted_regions,
        presentation.submitted_pixels,
        presentation.copied_regions,
        presentation.copied_pixels,
        presentation.damage_collapses
    );
    crate::println!(
        "portal-v3: promotions={} empty={} gop-full={} gop-partial={} readback-failures={} recoveries={}",
        presentation.damage_promotions,
        presentation.empty_presents,
        presentation.gop_full_presents,
        presentation.gop_partial_presents,
        presentation.gop_readback_failures,
        presentation.gop_recoveries
    );
    crate::println!(
        "copy-cost: last-ticks={} max-ticks={} last-regions={} last-pixels={}",
        presentation.last_copy_ticks,
        presentation.max_copy_ticks,
        presentation.last_copied_regions,
        presentation.last_copied_pixels
    );
    crate::println!(
        "aperture: address={:#x} device={} bytes driver-limit={} bytes",
        LFB_ADDRESS.load(Ordering::Acquire),
        LFB_DEVICE_BYTES.load(Ordering::Acquire),
        LFB_APERTURE_BYTES
    );
}

fn program_mode(requested: DisplayMode) -> bool {
    GOP_ACTIVE.store(false, Ordering::Release);
    reset_presentation_state();
    if !available() || !discover_aperture() {
        return false;
    }
    let aperture_bytes = LFB_DEVICE_BYTES.load(Ordering::Acquire);
    if requested.scanout_bytes() > aperture_bytes.min(LFB_APERTURE_BYTES) {
        return false;
    }
    let desired = requested.hardware_mode();
    write(INDEX_ENABLE, DISABLED);
    write(INDEX_XRES, desired.width);
    write(INDEX_YRES, desired.height);
    write(INDEX_BPP, desired.bits_per_pixel);
    write(INDEX_VIRT_WIDTH, desired.virtual_width);
    write(INDEX_VIRT_HEIGHT, desired.virtual_height);
    write(INDEX_X_OFFSET, 0);
    write(INDEX_Y_OFFSET, 0);
    write(INDEX_ENABLE, ENABLED | LFB_ENABLED);

    // Bochs-compatible adapters may expose a taller virtual canvas than the
    // visible mode even after VIRT_HEIGHT is programmed. That is harmless: the
    // visible height and virtual width determine every address we draw. Reject
    // any change in visible geometry, pixel format, or stride.
    let active = active_mode();
    let configured = active.is_some_and(|mode| mode_fits_aperture(requested, mode, aperture_bytes));
    if !configured {
        if let Some(mode) = active {
            crate::slog!(
                "EXPOS_DISPLAY_MODE_REJECTED requested={}x{}x{} actual={}x{}x{} virtual={}x{}\r\n",
                desired.width,
                desired.height,
                BITS_PER_PIXEL,
                mode.width,
                mode.height,
                mode.bits_per_pixel,
                mode.virtual_width,
                mode.virtual_height
            );
        } else {
            crate::slog!(
                "EXPOS_DISPLAY_MODE_REJECTED requested={}x{}x{} actual=disabled\r\n",
                desired.width,
                desired.height,
                BITS_PER_PIXEL
            );
        }
        write(INDEX_ENABLE, DISABLED);
        restore_vga_text_mode();
        return false;
    }

    let mode = active.expect("configured mode must be readable");
    let page_flip = mode.virtual_height as usize >= requested.height() * 2
        && requested.double_buffer_bytes() <= aperture_bytes.min(LFB_APERTURE_BYTES);
    ACTIVE_DISPLAY_MODE.store(requested.persisted(), Ordering::Release);
    ACTIVE_WIDTH.store(mode.width as usize, Ordering::Release);
    ACTIVE_HEIGHT.store(mode.height as usize, Ordering::Release);
    ACTIVE_STRIDE_BYTES.store(mode.stride_bytes(), Ordering::Release);
    PAGE_FLIP_AVAILABLE.store(page_flip, Ordering::Release);
    DRAW_Y.store(if page_flip { mode.height } else { 0 }, Ordering::Release);
    FRONT_Y.store(0, Ordering::Release);
    write(INDEX_Y_OFFSET, 0);
    HARDWARE_Y_OFFSET.store(read(INDEX_Y_OFFSET), Ordering::Release);
    crate::slog!(
        "EXPOS_DISPLAY_MODE width={} height={} bpp={} stride={} bytes={} preset={} pageflip={} virtual_height={}\r\n",
        mode.width,
        mode.height,
        mode.bits_per_pixel,
        mode.stride_bytes(),
        mode.scanout_bytes(),
        requested.label(),
        page_flip,
        mode.virtual_height
    );
    clear(color::BACKGROUND);
    true
}

fn program_firmware_framebuffer() -> bool {
    let Some(info) = crate::boot::firmware_framebuffer() else {
        return false;
    };
    let width = info.width as usize;
    let height = info.height as usize;
    let stride_bytes = info.stride as usize * BYTES_PER_PIXEL;
    let required = match stride_bytes.checked_mul(height) {
        Some(value) => value,
        None => return false,
    };
    if width > MAX_WIDTH
        || height > MAX_HEIGHT
        || required > info.bytes as usize
        || info.address > usize::MAX as u64
    {
        crate::slog!(
            "EXPOS_GOP_REJECTED width={} height={} stride={} bytes={}\r\n",
            info.width,
            info.height,
            info.stride,
            info.bytes
        );
        return false;
    }
    reset_presentation_state();
    let mode = DisplayMode::from_dimensions(width, height).unwrap_or(DisplayMode::P480);
    LFB_ADDRESS.store(info.address as usize, Ordering::Relaxed);
    LFB_DEVICE_BYTES.store(info.bytes as usize, Ordering::Relaxed);
    GOP_RGB_ORDER.store(info.format == 0, Ordering::Relaxed);
    GOP_ACTIVE.store(true, Ordering::Release);
    crate::asl::claim_firmware_framebuffer();
    ACTIVE_DISPLAY_MODE.store(mode.persisted(), Ordering::Release);
    ACTIVE_WIDTH.store(width, Ordering::Release);
    ACTIVE_HEIGHT.store(height, Ordering::Release);
    ACTIVE_STRIDE_BYTES.store(stride_bytes, Ordering::Release);
    FRONT_CONTENT_VISIBLE.store(true, Ordering::Release);
    crate::slog!(
        "EXPOS_FRAMEBUFFER_READY source=uefi-gop address={:#x} bytes={} width={} height={} stride={} format={}\r\n",
        info.address,
        info.bytes,
        width,
        height,
        stride_bytes,
        if info.format == 0 { "rgbx" } else { "bgrx" }
    );
    crate::slog!(
        "EXPOS_DISPLAY_MODE width={} height={} bpp=32 stride={} bytes={} preset=firmware pageflip=false virtual_height={}\r\n",
        width,
        height,
        stride_bytes,
        required,
        height
    );
    clear(color::BACKGROUND);
    true
}

fn reset_presentation_state() {
    ACTIVE_DISPLAY_MODE.store(NO_ACTIVE_MODE, Ordering::Release);
    ACTIVE_WIDTH.store(0, Ordering::Release);
    ACTIVE_HEIGHT.store(0, Ordering::Release);
    ACTIVE_STRIDE_BYTES.store(0, Ordering::Release);
    PAGE_FLIP_AVAILABLE.store(false, Ordering::Release);
    DRAW_Y.store(0, Ordering::Release);
    FRONT_Y.store(0, Ordering::Release);
    HARDWARE_Y_OFFSET.store(0, Ordering::Release);
    FRONT_CONTENT_VISIBLE.store(false, Ordering::Release);
    PRESENTED_FRAMES.store(0, Ordering::Release);
    VBLANK_TIMEOUTS.store(0, Ordering::Release);
    PAGE_FLIP_FAILURES.store(0, Ordering::Release);
    SUBMITTED_DAMAGE_REGIONS.store(0, Ordering::Release);
    SUBMITTED_DAMAGE_PIXELS.store(0, Ordering::Release);
    COPIED_DAMAGE_REGIONS.store(0, Ordering::Release);
    COPIED_DAMAGE_PIXELS.store(0, Ordering::Release);
    DAMAGE_COLLAPSES.store(0, Ordering::Release);
    DAMAGE_PROMOTIONS.store(0, Ordering::Release);
    EMPTY_PRESENTS.store(0, Ordering::Release);
    GOP_FULL_PRESENTS.store(0, Ordering::Release);
    GOP_PARTIAL_PRESENTS.store(0, Ordering::Release);
    GOP_READBACK_FAILURES.store(0, Ordering::Release);
    GOP_RECOVERIES.store(0, Ordering::Release);
    GOP_PARTIAL_SINCE_FULL.store(0, Ordering::Release);
    FORCE_FULL_RECONCILE.store(false, Ordering::Release);
    LAST_COPY_TICKS.store(0, Ordering::Release);
    MAX_COPY_TICKS.store(0, Ordering::Release);
    LAST_COPIED_REGIONS.store(0, Ordering::Release);
    LAST_COPIED_PIXELS.store(0, Ordering::Release);
}

fn mode_fits_aperture(requested: DisplayMode, mode: Mode, aperture_bytes: usize) -> bool {
    let desired = requested.hardware_mode();
    mode.width == desired.width
        && mode.height == desired.height
        && mode.bits_per_pixel == desired.bits_per_pixel
        && mode.virtual_width == desired.virtual_width
        && mode.virtual_height >= desired.height
        && mode.stride_bytes() == requested.stride_bytes()
        && mode.scanout_bytes() == requested.scanout_bytes()
        && mode.scanout_bytes() <= aperture_bytes.min(LFB_APERTURE_BYTES)
        && mode.aperture_bytes() <= aperture_bytes
}

pub fn exit() {
    ACTIVE_DISPLAY_MODE.store(NO_ACTIVE_MODE, Ordering::Release);
    ACTIVE_WIDTH.store(0, Ordering::Release);
    ACTIVE_HEIGHT.store(0, Ordering::Release);
    ACTIVE_STRIDE_BYTES.store(0, Ordering::Release);
    if GOP_ACTIVE.swap(false, Ordering::AcqRel) {
        FRONT_CONTENT_VISIBLE.store(false, Ordering::Release);
        PAGE_FLIP_AVAILABLE.store(false, Ordering::Release);
        return;
    }
    write(INDEX_Y_OFFSET, 0);
    HARDWARE_Y_OFFSET.store(read(INDEX_Y_OFFSET), Ordering::Release);
    FRONT_CONTENT_VISIBLE.store(false, Ordering::Release);
    write(INDEX_ENABLE, DISABLED);
    PAGE_FLIP_AVAILABLE.store(false, Ordering::Release);
    DRAW_Y.store(0, Ordering::Release);
    FRONT_Y.store(0, Ordering::Release);
    restore_vga_text_mode();
}

/// Present the page currently being composed.
///
/// Bochs/QEMU exposes enough virtual VRAM for two complete pages at every
/// supported resolution. When available, drawing happens on the hidden page
/// and this function switches `Y_OFFSET` atomically. The old front page is
/// then refreshed from the new one so partial window redraws remain correct.
/// A bounded VGA retrace wait is used when requested; failure never hangs the
/// kernel and is visible through [`presentation_stats`].
/// Return `true` only when the completed content is confirmed on the hardware
/// front page. A rejected page flip can still succeed after the changed pixels
/// are copied back and the previous front page is restored.
pub fn present(vsync: bool) -> bool {
    let damage = DamageRegion::new(0, 0, width() as i32, height() as i32);
    present_damage(vsync, &[damage])
}

/// Present the composed page and synchronize only the damaged rectangles to
/// the next back page. Damage is clipped and coalesced in a bounded stack
/// buffer first. Regions merge only when the resulting bounding copy is no
/// larger than copying them separately.
/// Callers must include every modified area, including software-cursor pixels,
/// or use [`present`] after a full-screen redraw.
/// Return `true` only when the completed content is confirmed on the hardware
/// front page. Submitted/copy damage counters still record attempted work when
/// scanout confirmation fails, while the presented-frame counter does not.
pub fn present_damage(vsync: bool, damage: &[DamageRegion]) -> bool {
    if active_display_mode().is_none() {
        return false;
    }
    let mut normalized = normalize_damage(damage, width() as i32, height() as i32);
    SUBMITTED_DAMAGE_REGIONS.fetch_add(normalized.submitted_regions, Ordering::AcqRel);
    SUBMITTED_DAMAGE_PIXELS.fetch_add(normalized.submitted_pixels, Ordering::AcqRel);
    if normalized.collapsed {
        DAMAGE_COLLAPSES.fetch_add(1, Ordering::AcqRel);
    }
    if normalized.len == 0 {
        EMPTY_PRESENTS.fetch_add(1, Ordering::AcqRel);
        LAST_COPIED_REGIONS.store(0, Ordering::Release);
        LAST_COPIED_PIXELS.store(0, Ordering::Release);
        return FRONT_CONTENT_VISIBLE.load(Ordering::Acquire);
    }
    if GOP_ACTIVE.load(Ordering::Acquire) && should_promote_gop_damage(&normalized) {
        promote_to_full_damage(&mut normalized);
        DAMAGE_PROMOTIONS.fetch_add(1, Ordering::AcqRel);
    }
    let page_flip = PAGE_FLIP_AVAILABLE.load(Ordering::Acquire);
    if vsync && page_flip && !wait_for_vertical_retrace() {
        VBLANK_TIMEOUTS.fetch_add(1, Ordering::AcqRel);
    }
    let visible = if page_flip {
        let height = height() as u16;
        let prior_front = FRONT_Y.load(Ordering::Acquire);
        let next_front = DRAW_Y.load(Ordering::Acquire);
        write(INDEX_Y_OFFSET, next_front);
        let hardware_y_offset = read(INDEX_Y_OFFSET);
        HARDWARE_Y_OFFSET.store(hardware_y_offset, Ordering::Release);
        if hardware_y_offset == next_front {
            FRONT_Y.store(next_front, Ordering::Release);
            let next_draw = if next_front == 0 { height } else { 0 };
            DRAW_Y.store(next_draw, Ordering::Release);
            copy_damage(next_front, next_draw, &normalized);
            FRONT_CONTENT_VISIBLE.store(true, Ordering::Release);
            true
        } else {
            // Some VBE implementations accept a two-page virtual mode but do
            // not honor later Y-offset flips. Preserve the completed frame by
            // copying its damage back to the page that was visible before the
            // rejected flip, then remain in direct-to-front rendering mode.
            PAGE_FLIP_FAILURES.fetch_add(1, Ordering::AcqRel);
            if next_front != prior_front {
                copy_damage(next_front, prior_front, &normalized);
            }
            write(INDEX_Y_OFFSET, prior_front);
            let recovered_y_offset = read(INDEX_Y_OFFSET);
            HARDWARE_Y_OFFSET.store(recovered_y_offset, Ordering::Release);
            FRONT_Y.store(prior_front, Ordering::Release);
            DRAW_Y.store(prior_front, Ordering::Release);
            PAGE_FLIP_AVAILABLE.store(false, Ordering::Release);
            let visible = recovered_y_offset == prior_front;
            FRONT_CONTENT_VISIBLE.store(visible, Ordering::Release);
            crate::slog!(
                "EXPOS_PAGE_FLIP_DISABLED requested_y={} actual_y={} recovered_y={} visible={}\r\n",
                next_front,
                hardware_y_offset,
                recovered_y_offset,
                visible
            );
            visible
        }
    } else {
        if GOP_ACTIVE.load(Ordering::Acquire) {
            let started = crate::hardware::timestamp();
            let visible = copy_gop_damage(&normalized);
            let elapsed = crate::hardware::timestamp().saturating_sub(started);
            LAST_COPY_TICKS.store(elapsed, Ordering::Release);
            MAX_COPY_TICKS.fetch_max(elapsed, Ordering::AcqRel);
            FRONT_CONTENT_VISIBLE.store(visible, Ordering::Release);
            if visible {
                PRESENTED_FRAMES.fetch_add(1, Ordering::AcqRel);
            }
            return visible;
        }
        let hardware_y_offset = read(INDEX_Y_OFFSET);
        HARDWARE_Y_OFFSET.store(hardware_y_offset, Ordering::Release);
        let visible = hardware_y_offset == FRONT_Y.load(Ordering::Acquire);
        FRONT_CONTENT_VISIBLE.store(visible, Ordering::Release);
        visible
    };
    if visible {
        PRESENTED_FRAMES.fetch_add(1, Ordering::AcqRel);
    }
    visible
}

pub fn clear(value: u32) {
    let Some(pointer) = framebuffer_pointer() else {
        return;
    };
    let pixels = scanout_bytes() / BYTES_PER_PIXEL;
    let page = draw_page_offset_pixels();
    // SAFETY: the validated scanout geometry keeps this complete draw page
    // inside the mapped LFB aperture. The framebuffer is exclusively owned by
    // this module while graphics mode is active.
    unsafe { fill_dwords(pointer.add(page), encode_pixel(value), pixels) };
}

pub fn pixel(x: i32, y: i32, value: u32) {
    let Some(pointer) = framebuffer_pointer() else {
        return;
    };
    if x < 0 || y < 0 || x >= width() as i32 || y >= height() as i32 {
        return;
    }
    let stride_pixels = stride_bytes() / BYTES_PER_PIXEL;
    let offset = draw_page_offset_pixels() + y as usize * stride_pixels + x as usize;
    unsafe { core::ptr::write_volatile(pointer.add(offset), encode_pixel(value)) };
}

pub fn read_pixel(x: i32, y: i32) -> u32 {
    let Some(pointer) = framebuffer_pointer() else {
        return 0;
    };
    if x < 0 || y < 0 || x >= width() as i32 || y >= height() as i32 {
        return 0;
    }
    let stride_pixels = stride_bytes() / BYTES_PER_PIXEL;
    let offset = draw_page_offset_pixels() + y as usize * stride_pixels + x as usize;
    decode_pixel(unsafe { core::ptr::read_volatile(pointer.add(offset)) })
}

pub fn rect(x: i32, y: i32, width: i32, height: i32, value: u32) {
    let Some(pointer) = framebuffer_pointer() else {
        return;
    };
    let mode_width = self::width() as i32;
    let mode_height = self::height() as i32;
    let Some(clipped) = clip_region(
        DamageRegion::new(x, y, width, height),
        mode_width,
        mode_height,
    ) else {
        return;
    };
    let stride_pixels = stride_bytes() / BYTES_PER_PIXEL;
    let value = encode_pixel(value);
    let page = draw_page_offset_pixels();

    // A full-width rectangle is contiguous and can be emitted as one string
    // operation. Narrow rectangles retain their original row/stride geometry.
    if clipped.left == 0 && clipped.right == stride_pixels {
        let offset = page + clipped.top * stride_pixels;
        // SAFETY: clipping bounds every row to the selected draw page.
        unsafe { fill_dwords(pointer.add(offset), value, clipped.height() * stride_pixels) };
    } else {
        for row in clipped.top..clipped.bottom {
            let offset = page + row * stride_pixels + clipped.left;
            // SAFETY: clipping bounds the start and width to this scanline.
            unsafe { fill_dwords(pointer.add(offset), value, clipped.width()) };
        }
    }
}

fn draw_page_offset_pixels() -> usize {
    DRAW_Y.load(Ordering::Acquire) as usize * (stride_bytes() / BYTES_PER_PIXEL)
}

fn encode_pixel(value: u32) -> u32 {
    if GOP_ACTIVE.load(Ordering::Relaxed) && GOP_RGB_ORDER.load(Ordering::Relaxed) {
        (value & 0xFF00_FF00) | ((value & 0x00FF_0000) >> 16) | ((value & 0x0000_00FF) << 16)
    } else {
        value
    }
}

fn decode_pixel(value: u32) -> u32 {
    encode_pixel(value)
}

/// Fill exactly `count` dwords using an architecturally visible x86 string
/// operation. Inline assembly has an implicit memory clobber here, preventing
/// the compiler from removing or moving framebuffer writes across the call.
///
/// # Safety
///
/// `destination..destination.add(count)` must be writable and mapped.
#[inline(always)]
unsafe fn fill_dwords(destination: *mut u32, value: u32, count: usize) {
    if count == 0 {
        return;
    }
    unsafe {
        core::arch::asm!(
            "cld",
            "rep stosd",
            inout("rdi") destination => _,
            inout("rcx") count => _,
            in("eax") value,
            options(nostack)
        );
    }
}

/// Copy exactly `count` non-overlapping dwords using an x86 string operation.
/// Unlike an ordinary Rust slice copy, the inline assembly is an explicit
/// memory side effect suitable for the mapped framebuffer aperture.
///
/// # Safety
///
/// Both ranges must be mapped for `count` dwords and must not overlap.
#[inline(always)]
unsafe fn copy_dwords(source: *const u32, destination: *mut u32, count: usize) {
    if count == 0 {
        return;
    }
    unsafe {
        core::arch::asm!(
            "cld",
            "rep movsd",
            inout("rsi") source => _,
            inout("rdi") destination => _,
            inout("rcx") count => _,
            options(nostack)
        );
    }
}

fn copy_damage(source_y: u16, target_y: u16, damage: &NormalizedDamage) {
    debug_assert_ne!(source_y, target_y);
    for region in &damage.regions[..damage.len] {
        copy_clipped_region(source_y, target_y, *region);
    }
    COPIED_DAMAGE_REGIONS.fetch_add(damage.copied_regions(), Ordering::AcqRel);
    COPIED_DAMAGE_PIXELS.fetch_add(damage.copied_pixels(), Ordering::AcqRel);
}

fn copy_gop_damage(damage: &NormalizedDamage) -> bool {
    let Some(scanout) = scanout_pointer() else {
        return false;
    };
    // SAFETY: publication reads from the compositor-owned shadow surface only
    // after the frame has been fully composed.
    let shadow = unsafe { core::ptr::addr_of!(GOP_SHADOW.0).cast::<u32>() };
    let stride_pixels = stride_bytes() / BYTES_PER_PIXEL;
    for clipped in &damage.regions[..damage.len] {
        if clipped.left == 0 && clipped.right == stride_pixels {
            let offset = clipped.top * stride_pixels;
            // SAFETY: GOP geometry and stride were validated against both the
            // firmware aperture and the bounded shadow surface.
            unsafe {
                copy_dwords(
                    shadow.add(offset),
                    scanout.add(offset),
                    clipped.height() * stride_pixels,
                )
            };
        } else {
            for row in clipped.top..clipped.bottom {
                let offset = row * stride_pixels + clipped.left;
                // SAFETY: normalized damage is clipped to active geometry.
                unsafe { copy_dwords(shadow.add(offset), scanout.add(offset), clipped.width()) };
            }
        }
    }
    scanout_fence();
    COPIED_DAMAGE_REGIONS.fetch_add(damage.copied_regions(), Ordering::AcqRel);
    COPIED_DAMAGE_PIXELS.fetch_add(damage.copied_pixels(), Ordering::AcqRel);
    LAST_COPIED_REGIONS.store(damage.copied_regions(), Ordering::Release);
    LAST_COPIED_PIXELS.store(damage.copied_pixels(), Ordering::Release);
    let full = is_full_damage(damage);
    if full {
        GOP_FULL_PRESENTS.fetch_add(1, Ordering::AcqRel);
        GOP_PARTIAL_SINCE_FULL.store(0, Ordering::Release);
    } else {
        GOP_PARTIAL_PRESENTS.fetch_add(1, Ordering::AcqRel);
        GOP_PARTIAL_SINCE_FULL.fetch_add(1, Ordering::AcqRel);
    }
    if verify_gop_damage(shadow, scanout, damage) {
        return true;
    }

    GOP_READBACK_FAILURES.fetch_add(1, Ordering::AcqRel);
    crate::slog!(
        "EXPOS_DISPLAY_V3_READBACK_MISMATCH regions={} pixels={} recovery=full\r\n",
        damage.copied_regions(),
        damage.copied_pixels()
    );
    let pixels = scanout_bytes() / BYTES_PER_PIXEL;
    // SAFETY: validated firmware geometry bounds both complete surfaces.
    unsafe { copy_dwords(shadow, scanout, pixels) };
    scanout_fence();
    GOP_RECOVERIES.fetch_add(1, Ordering::AcqRel);
    GOP_PARTIAL_SINCE_FULL.store(0, Ordering::Release);
    verify_gop_samples(shadow, scanout)
}

fn should_promote_gop_damage(damage: &NormalizedDamage) -> bool {
    let forced = FORCE_FULL_RECONCILE.swap(false, Ordering::AcqRel);
    if is_full_damage(damage) {
        return false;
    }
    if forced || GOP_PARTIAL_SINCE_FULL.load(Ordering::Acquire) >= GOP_RECONCILE_INTERVAL {
        return true;
    }
    let visible_pixels = width() as u64 * height() as u64;
    damage
        .copied_pixels()
        .saturating_mul(FULL_DAMAGE_PROMOTION_DENOMINATOR)
        >= visible_pixels.saturating_mul(FULL_DAMAGE_PROMOTION_NUMERATOR)
}

fn promote_to_full_damage(damage: &mut NormalizedDamage) {
    damage.regions = [ClippedRegion::EMPTY; MAX_DAMAGE_REGIONS];
    damage.regions[0] = ClippedRegion {
        left: 0,
        top: 0,
        right: width(),
        bottom: height(),
    };
    damage.len = 1;
}

fn is_full_damage(damage: &NormalizedDamage) -> bool {
    damage.len == 1
        && damage.regions[0]
            == (ClippedRegion {
                left: 0,
                top: 0,
                right: width(),
                bottom: height(),
            })
}

fn verify_gop_damage(shadow: *const u32, scanout: *const u32, damage: &NormalizedDamage) -> bool {
    let stride = stride_bytes() / BYTES_PER_PIXEL;
    for region in &damage.regions[..damage.len] {
        let first = region.top * stride + region.left;
        let last = (region.bottom - 1) * stride + region.right - 1;
        let middle =
            ((region.top + region.bottom - 1) / 2) * stride + (region.left + region.right - 1) / 2;
        for offset in [first, middle, last] {
            // SAFETY: every sample belongs to a clipped visible region.
            let source = unsafe { core::ptr::read_volatile(shadow.add(offset)) };
            let target = unsafe { core::ptr::read_volatile(scanout.add(offset)) };
            if source != target {
                return false;
            }
        }
    }
    true
}

fn verify_gop_samples(shadow: *const u32, scanout: *const u32) -> bool {
    let pixels = scanout_bytes() / BYTES_PER_PIXEL;
    verify_samples(shadow, scanout, pixels)
}

fn verify_samples(shadow: *const u32, scanout: *const u32, pixels: usize) -> bool {
    if pixels == 0 {
        return false;
    }
    for offset in [0, pixels / 2, pixels.saturating_sub(1)] {
        // SAFETY: active scanout contains at least one validated pixel.
        let source = unsafe { core::ptr::read_volatile(shadow.add(offset)) };
        let target = unsafe { core::ptr::read_volatile(scanout.add(offset)) };
        if source != target {
            return false;
        }
    }
    true
}

#[inline(always)]
fn scanout_fence() {
    // Framebuffer apertures can be write-combined. Complete every published
    // row before readback or returning control to the compositor.
    unsafe { core::arch::asm!("sfence", options(nostack, preserves_flags)) };
}

fn copy_clipped_region(source_y: u16, target_y: u16, clipped: ClippedRegion) {
    let Some(pointer) = framebuffer_pointer() else {
        return;
    };
    let stride_pixels = stride_bytes() / BYTES_PER_PIXEL;
    let source = source_y as usize * stride_pixels;
    let target = target_y as usize * stride_pixels;

    debug_assert_ne!(source_y, target_y);
    if clipped.left == 0 && clipped.right == stride_pixels {
        let row_offset = clipped.top * stride_pixels;
        // SAFETY: page flipping selects disjoint source and target pages, and
        // clipping keeps this contiguous copy within their visible rows.
        unsafe {
            copy_dwords(
                pointer.add(source + row_offset),
                pointer.add(target + row_offset),
                clipped.height() * stride_pixels,
            )
        };
    } else {
        for row in clipped.top..clipped.bottom {
            let row_offset = row * stride_pixels + clipped.left;
            // SAFETY: source and target pages are disjoint; clipping bounds
            // this row to the visible scanout width.
            unsafe {
                copy_dwords(
                    pointer.add(source + row_offset),
                    pointer.add(target + row_offset),
                    clipped.width(),
                )
            };
        }
    }
}

fn wait_for_vertical_retrace() -> bool {
    let mut left_retrace = false;
    for _ in 0..VBLANK_POLL_LIMIT {
        let status = unsafe { port::inb(VGA_INPUT_STATUS) };
        if status & VGA_VERTICAL_RETRACE == 0 {
            left_retrace = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !left_retrace {
        return false;
    }
    for _ in 0..VBLANK_POLL_LIMIT {
        let status = unsafe { port::inb(VGA_INPUT_STATUS) };
        if status & VGA_VERTICAL_RETRACE != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

pub fn outline(x: i32, y: i32, width: i32, height: i32, value: u32) {
    rect(x, y, width, 1, value);
    rect(x, y + height - 1, width, 1, value);
    rect(x, y, 1, height, value);
    rect(x + width - 1, y, 1, height, value);
}

#[allow(dead_code)]
pub fn vertical_gradient(x: i32, y: i32, width: i32, height: i32, top: u32, bottom: u32) {
    if height <= 0 {
        return;
    }
    let denominator = (height - 1).max(1) as u32;
    for row in 0..height {
        rect(
            x,
            y + row,
            width,
            1,
            lerp_color(top, bottom, row as u32, denominator),
        );
    }
}

pub fn rounded_rect(x: i32, y: i32, width: i32, height: i32, radius: i32, value: u32) {
    if width <= 0 || height <= 0 {
        return;
    }
    let radius = radius.max(0).min(width / 2).min(height / 2);
    if radius == 0 {
        rect(x, y, width, height, value);
        return;
    }
    rect(x + radius, y, width - radius * 2, height, value);
    rect(x, y + radius, width, height - radius * 2, value);
    for row in 0..radius {
        let dy = radius - row;
        let mut dx = 0;
        while (dx + 1) * (dx + 1) + dy * dy <= radius * radius {
            dx += 1;
        }
        let inset = radius - dx;
        rect(x + inset, y + row, width - inset * 2, 1, value);
        rect(x + inset, y + height - row - 1, width - inset * 2, 1, value);
    }
}

pub fn alpha_rect(x: i32, y: i32, width: i32, height: i32, value: u32, alpha: u8) {
    let mode_width = self::width() as i32;
    let mode_height = self::height() as i32;
    let left = x.max(0).min(mode_width);
    let top = y.max(0).min(mode_height);
    let right = x.saturating_add(width).max(0).min(mode_width);
    let bottom = y.saturating_add(height).max(0).min(mode_height);
    for row in top..bottom {
        for column in left..right {
            pixel(column, row, blend(read_pixel(column, row), value, alpha));
        }
    }
}

/// Blend a rounded rectangle without allocating an intermediate surface.
pub fn alpha_rounded_rect(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    radius: i32,
    value: u32,
    alpha: u8,
) {
    if width <= 0 || height <= 0 {
        return;
    }
    let radius = radius.max(0).min(width / 2).min(height / 2);
    if radius == 0 {
        alpha_rect(x, y, width, height, value, alpha);
        return;
    }
    for row in 0..height {
        let corner_row = if row < radius {
            row
        } else if row >= height - radius {
            height - row - 1
        } else {
            radius
        };
        let inset = if corner_row < radius {
            let dy = radius - corner_row;
            let mut dx = 0;
            while (dx + 1) * (dx + 1) + dy * dy <= radius * radius {
                dx += 1;
            }
            radius - dx
        } else {
            0
        };
        if width > inset * 2 {
            alpha_rect(x + inset, y + row, width - inset * 2, 1, value, alpha);
        }
    }
}

/// Draw a one-pixel rounded outline. The row geometry matches
/// `rounded_rect`, so concentric calls can build thicker borders without
/// squaring off the selected window radius.
pub fn rounded_outline(x: i32, y: i32, width: i32, height: i32, radius: i32, value: u32) {
    if width <= 0 || height <= 0 {
        return;
    }
    let radius = radius.max(0).min(width / 2).min(height / 2);
    if radius == 0 {
        outline(x, y, width, height, value);
        return;
    }
    for row in 0..height {
        let corner_row = if row < radius {
            row
        } else if row >= height - radius {
            height - row - 1
        } else {
            radius
        };
        let inset = if corner_row < radius {
            let dy = radius - corner_row;
            let mut dx = 0;
            while (dx + 1) * (dx + 1) + dy * dy <= radius * radius {
                dx += 1;
            }
            radius - dx
        } else {
            0
        };
        let row_width = width - inset * 2;
        if row_width <= 0 {
            continue;
        }
        if row == 0 || row == height - 1 {
            rect(x + inset, y + row, row_width, 1, value);
        } else {
            pixel(x + inset, y + row, value);
            if row_width > 1 {
                pixel(x + inset + row_width - 1, y + row, value);
            }
        }
    }
}

pub fn line(mut x0: i32, mut y0: i32, x1: i32, y1: i32, value: u32) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        pixel(x0, y0, value);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice = error * 2;
        if twice >= dy {
            error += dy;
            x0 += sx;
        }
        if twice <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

fn lerp_color(from: u32, to: u32, step: u32, total: u32) -> u32 {
    let channel = |shift: u32| {
        let start = ((from >> shift) & 0xFF) as i32;
        let end = ((to >> shift) & 0xFF) as i32;
        (start + (end - start) * step as i32 / total as i32) as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn blend(background: u32, foreground: u32, alpha: u8) -> u32 {
    let inverse = 255_u32 - alpha as u32;
    let mix = |shift: u32| {
        ((((background >> shift) & 0xFF) * inverse + ((foreground >> shift) & 0xFF) * alpha as u32)
            / 255)
            << shift
    };
    mix(16) | mix(8) | mix(0)
}

pub const fn text_advance(scale: i32) -> i32 {
    match scale {
        i32::MIN..=1 => 8,
        2 => 9,
        _ => 17,
    }
}

const fn text_line_height(scale: i32) -> i32 {
    match scale {
        i32::MIN..=1 => 10,
        2 => 18,
        _ => 18,
    }
}

pub fn text(x: i32, mut y: i32, value: &str, color: u32, scale: i32) {
    let advance = text_advance(scale);
    let style = font_style();
    for line in value.split('\n') {
        let mut cursor = x;
        // Hebrew strings are stored in logical order. This compact renderer
        // has no shaping engine, so reverse an RTL line into visual order.
        // Hebrew has no joined letter forms, making this deterministic and
        // sufficient for the system UI without a heap-backed text stack.
        if line.chars().any(is_hebrew) {
            for character in line.chars().rev() {
                glyph_char_with_style(cursor, y, character, color, scale, style);
                cursor += advance;
            }
        } else {
            for character in line.chars() {
                if character == '\t' {
                    cursor += advance * 4;
                } else {
                    glyph_char_with_style(cursor, y, character, color, scale, style);
                    cursor += advance;
                }
            }
        }
        y += text_line_height(scale);
    }
}

pub fn glyph(x: i32, y: i32, byte: u8, color: u32, scale: i32) {
    glyph_char_with_style(x, y, byte as char, color, scale, font_style());
}

pub fn glyph_char(x: i32, y: i32, character: char, color: u32, scale: i32) {
    glyph_char_with_style(x, y, character, color, scale, font_style());
}

fn glyph_char_with_style(
    x: i32,
    y: i32,
    character: char,
    color: u32,
    scale: i32,
    style: FontStyle,
) {
    let rows = styled_glyph_rows(character, style);
    let (pixel_width, pixel_height) = match scale {
        i32::MIN..=1 => (1, 1),
        2 => (1, 2),
        _ => (2, 2),
    };
    for (row, bits) in rows.iter().enumerate() {
        for column in 0..8 {
            if bits & (1 << column) != 0 {
                rect(
                    x + column * pixel_width,
                    y + row as i32 * pixel_height,
                    pixel_width,
                    pixel_height,
                    color,
                );
            }
        }
    }
}

fn write(index: u16, value: u16) {
    unsafe {
        port::outw(VBE_INDEX, index);
        port::outw(VBE_DATA, value);
    }
}

fn read(index: u16) -> u16 {
    unsafe {
        port::outw(VBE_INDEX, index);
        port::inw(VBE_DATA)
    }
}

fn read_mode() -> Mode {
    Mode {
        width: read(INDEX_XRES),
        height: read(INDEX_YRES),
        bits_per_pixel: read(INDEX_BPP),
        virtual_width: read(INDEX_VIRT_WIDTH),
        virtual_height: read(INDEX_VIRT_HEIGHT),
    }
}

fn restore_vga_text_mode() {
    const SEQUENCER: [u8; 5] = [0x03, 0x00, 0x03, 0x00, 0x02];
    const CRTC: [u8; 25] = [
        0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F, 0x00, 0x4F, 0x0D, 0x0E, 0x00, 0x00, 0x00,
        0x50, 0x9C, 0x0E, 0x8F, 0x28, 0x1F, 0x96, 0xB9, 0xA3, 0xFF,
    ];
    const GRAPHICS: [u8; 9] = [0x00, 0x00, 0x10, 0x00, 0x00, 0x10, 0x0E, 0x00, 0xFF];
    const ATTRIBUTE: [u8; 21] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E,
        0x3F, 0x0C, 0x00, 0x0F, 0x08, 0x00,
    ];
    unsafe {
        port::outb(0x3C2, 0x67);
        for (index, value) in SEQUENCER.iter().enumerate() {
            port::outb(0x3C4, index as u8);
            port::outb(0x3C5, *value);
        }
        port::outb(0x3D4, 0x03);
        port::outb(0x3D5, port::inb(0x3D5) | 0x80);
        port::outb(0x3D4, 0x11);
        port::outb(0x3D5, port::inb(0x3D5) & !0x80);
        for (index, value) in CRTC.iter().enumerate() {
            port::outb(0x3D4, index as u8);
            port::outb(0x3D5, *value);
        }
        for (index, value) in GRAPHICS.iter().enumerate() {
            port::outb(0x3CE, index as u8);
            port::outb(0x3CF, *value);
        }
        load_text_font();
        for (index, value) in ATTRIBUTE.iter().enumerate() {
            let _ = port::inb(0x3DA);
            port::outb(0x3C0, index as u8);
            port::outb(0x3C0, *value);
        }
        let _ = port::inb(0x3DA);
        port::outb(0x3C0, 0x20);
    }
}

unsafe fn load_text_font() {
    // VBE does not promise to preserve VGA plane 2. Select the character
    // generator plane, rebuild a readable 8x16 font from the kernel glyphs,
    // then restore normal interleaved text-memory access.
    unsafe {
        port::outb(0x3C4, 0x00);
        port::outb(0x3C5, 0x01);
        port::outb(0x3C4, 0x02);
        port::outb(0x3C5, 0x04);
        port::outb(0x3C4, 0x04);
        port::outb(0x3C5, 0x07);
        port::outb(0x3C4, 0x00);
        port::outb(0x3C5, 0x03);

        port::outb(0x3CE, 0x04);
        port::outb(0x3CF, 0x02);
        port::outb(0x3CE, 0x05);
        port::outb(0x3CF, 0x00);
        port::outb(0x3CE, 0x06);
        port::outb(0x3CF, 0x04);

        let font_plane = 0xA0000 as *mut u8;
        let style = font_style();
        for character in 0..=u8::MAX {
            let rows = styled_glyph_rows(character as char, style);
            let glyph = font_plane.add(character as usize * 32);
            for scanline in 0..32 {
                let value = if scanline < 16 {
                    rows[scanline / 2].reverse_bits()
                } else {
                    0
                };
                core::ptr::write_volatile(glyph.add(scanline), value);
            }
        }

        port::outb(0x3C4, 0x00);
        port::outb(0x3C5, 0x01);
        port::outb(0x3C4, 0x02);
        port::outb(0x3C5, 0x03);
        port::outb(0x3C4, 0x04);
        port::outb(0x3C5, 0x02);
        port::outb(0x3C4, 0x00);
        port::outb(0x3C5, 0x03);

        port::outb(0x3CE, 0x04);
        port::outb(0x3CF, 0x00);
        port::outb(0x3CE, 0x05);
        port::outb(0x3CF, 0x10);
        port::outb(0x3CE, 0x06);
        port::outb(0x3CF, 0x0E);
    }
}

// Public-domain IBM VGA glyphs, adapted from Daniel Hepper's font8x8_basic.
// Keeping the real lower-case rows is important: the former 5x7 renderer
// upper-cased every byte and made normal prose look like display lettering.
#[rustfmt::skip]
const FONT8X8_BASIC: [[u8; 8]; 95] = [
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00], // space
    [0x18,0x3C,0x3C,0x18,0x18,0x00,0x18,0x00], // !
    [0x36,0x36,0x00,0x00,0x00,0x00,0x00,0x00], // "
    [0x36,0x36,0x7F,0x36,0x7F,0x36,0x36,0x00], // #
    [0x0C,0x3E,0x03,0x1E,0x30,0x1F,0x0C,0x00], // $
    [0x00,0x63,0x33,0x18,0x0C,0x66,0x63,0x00], // %
    [0x1C,0x36,0x1C,0x6E,0x3B,0x33,0x6E,0x00], // &
    [0x06,0x06,0x03,0x00,0x00,0x00,0x00,0x00], // '
    [0x18,0x0C,0x06,0x06,0x06,0x0C,0x18,0x00], // (
    [0x06,0x0C,0x18,0x18,0x18,0x0C,0x06,0x00], // )
    [0x00,0x66,0x3C,0xFF,0x3C,0x66,0x00,0x00], // *
    [0x00,0x0C,0x0C,0x3F,0x0C,0x0C,0x00,0x00], // +
    [0x00,0x00,0x00,0x00,0x00,0x0C,0x0C,0x06], // ,
    [0x00,0x00,0x00,0x3F,0x00,0x00,0x00,0x00], // -
    [0x00,0x00,0x00,0x00,0x00,0x0C,0x0C,0x00], // .
    [0x60,0x30,0x18,0x0C,0x06,0x03,0x01,0x00], // /
    [0x3E,0x63,0x73,0x7B,0x6F,0x67,0x3E,0x00], // 0
    [0x0C,0x0E,0x0C,0x0C,0x0C,0x0C,0x3F,0x00], // 1
    [0x1E,0x33,0x30,0x1C,0x06,0x33,0x3F,0x00], // 2
    [0x1E,0x33,0x30,0x1C,0x30,0x33,0x1E,0x00], // 3
    [0x38,0x3C,0x36,0x33,0x7F,0x30,0x78,0x00], // 4
    [0x3F,0x03,0x1F,0x30,0x30,0x33,0x1E,0x00], // 5
    [0x1C,0x06,0x03,0x1F,0x33,0x33,0x1E,0x00], // 6
    [0x3F,0x33,0x30,0x18,0x0C,0x0C,0x0C,0x00], // 7
    [0x1E,0x33,0x33,0x1E,0x33,0x33,0x1E,0x00], // 8
    [0x1E,0x33,0x33,0x3E,0x30,0x18,0x0E,0x00], // 9
    [0x00,0x0C,0x0C,0x00,0x00,0x0C,0x0C,0x00], // :
    [0x00,0x0C,0x0C,0x00,0x00,0x0C,0x0C,0x06], // ;
    [0x18,0x0C,0x06,0x03,0x06,0x0C,0x18,0x00], // <
    [0x00,0x00,0x3F,0x00,0x00,0x3F,0x00,0x00], // =
    [0x06,0x0C,0x18,0x30,0x18,0x0C,0x06,0x00], // >
    [0x1E,0x33,0x30,0x18,0x0C,0x00,0x0C,0x00], // ?
    [0x3E,0x63,0x7B,0x7B,0x7B,0x03,0x1E,0x00], // @
    [0x0C,0x1E,0x33,0x33,0x3F,0x33,0x33,0x00], // A
    [0x3F,0x66,0x66,0x3E,0x66,0x66,0x3F,0x00], // B
    [0x3C,0x66,0x03,0x03,0x03,0x66,0x3C,0x00], // C
    [0x1F,0x36,0x66,0x66,0x66,0x36,0x1F,0x00], // D
    [0x7F,0x46,0x16,0x1E,0x16,0x46,0x7F,0x00], // E
    [0x7F,0x46,0x16,0x1E,0x16,0x06,0x0F,0x00], // F
    [0x3C,0x66,0x03,0x03,0x73,0x66,0x7C,0x00], // G
    [0x33,0x33,0x33,0x3F,0x33,0x33,0x33,0x00], // H
    [0x1E,0x0C,0x0C,0x0C,0x0C,0x0C,0x1E,0x00], // I
    [0x78,0x30,0x30,0x30,0x33,0x33,0x1E,0x00], // J
    [0x67,0x66,0x36,0x1E,0x36,0x66,0x67,0x00], // K
    [0x0F,0x06,0x06,0x06,0x46,0x66,0x7F,0x00], // L
    [0x63,0x77,0x7F,0x7F,0x6B,0x63,0x63,0x00], // M
    [0x63,0x67,0x6F,0x7B,0x73,0x63,0x63,0x00], // N
    [0x1C,0x36,0x63,0x63,0x63,0x36,0x1C,0x00], // O
    [0x3F,0x66,0x66,0x3E,0x06,0x06,0x0F,0x00], // P
    [0x1E,0x33,0x33,0x33,0x3B,0x1E,0x38,0x00], // Q
    [0x3F,0x66,0x66,0x3E,0x36,0x66,0x67,0x00], // R
    [0x1E,0x33,0x07,0x0E,0x38,0x33,0x1E,0x00], // S
    [0x3F,0x2D,0x0C,0x0C,0x0C,0x0C,0x1E,0x00], // T
    [0x33,0x33,0x33,0x33,0x33,0x33,0x3F,0x00], // U
    [0x33,0x33,0x33,0x33,0x33,0x1E,0x0C,0x00], // V
    [0x63,0x63,0x63,0x6B,0x7F,0x77,0x63,0x00], // W
    [0x63,0x63,0x36,0x1C,0x1C,0x36,0x63,0x00], // X
    [0x33,0x33,0x33,0x1E,0x0C,0x0C,0x1E,0x00], // Y
    [0x7F,0x63,0x31,0x18,0x4C,0x66,0x7F,0x00], // Z
    [0x1E,0x06,0x06,0x06,0x06,0x06,0x1E,0x00], // [
    [0x03,0x06,0x0C,0x18,0x30,0x60,0x40,0x00], // backslash
    [0x1E,0x18,0x18,0x18,0x18,0x18,0x1E,0x00], // ]
    [0x08,0x1C,0x36,0x63,0x00,0x00,0x00,0x00], // ^
    [0x00,0x00,0x00,0x00,0x00,0x00,0x00,0xFF], // _
    [0x0C,0x0C,0x18,0x00,0x00,0x00,0x00,0x00], // `
    [0x00,0x00,0x1E,0x30,0x3E,0x33,0x6E,0x00], // a
    [0x07,0x06,0x06,0x3E,0x66,0x66,0x3B,0x00], // b
    [0x00,0x00,0x1E,0x33,0x03,0x33,0x1E,0x00], // c
    [0x38,0x30,0x30,0x3E,0x33,0x33,0x6E,0x00], // d
    [0x00,0x00,0x1E,0x33,0x3F,0x03,0x1E,0x00], // e
    [0x1C,0x36,0x06,0x0F,0x06,0x06,0x0F,0x00], // f
    [0x00,0x00,0x6E,0x33,0x33,0x3E,0x30,0x1F], // g
    [0x07,0x06,0x36,0x6E,0x66,0x66,0x67,0x00], // h
    [0x0C,0x00,0x0E,0x0C,0x0C,0x0C,0x1E,0x00], // i
    [0x30,0x00,0x30,0x30,0x30,0x33,0x33,0x1E], // j
    [0x07,0x06,0x66,0x36,0x1E,0x36,0x67,0x00], // k
    [0x0E,0x0C,0x0C,0x0C,0x0C,0x0C,0x1E,0x00], // l
    [0x00,0x00,0x33,0x7F,0x7F,0x6B,0x63,0x00], // m
    [0x00,0x00,0x1F,0x33,0x33,0x33,0x33,0x00], // n
    [0x00,0x00,0x1E,0x33,0x33,0x33,0x1E,0x00], // o
    [0x00,0x00,0x3B,0x66,0x66,0x3E,0x06,0x0F], // p
    [0x00,0x00,0x6E,0x33,0x33,0x3E,0x30,0x78], // q
    [0x00,0x00,0x3B,0x6E,0x66,0x06,0x0F,0x00], // r
    [0x00,0x00,0x3E,0x03,0x1E,0x30,0x1F,0x00], // s
    [0x08,0x0C,0x3E,0x0C,0x0C,0x2C,0x18,0x00], // t
    [0x00,0x00,0x33,0x33,0x33,0x33,0x6E,0x00], // u
    [0x00,0x00,0x33,0x33,0x33,0x1E,0x0C,0x00], // v
    [0x00,0x00,0x63,0x6B,0x7F,0x7F,0x36,0x00], // w
    [0x00,0x00,0x63,0x36,0x1C,0x36,0x63,0x00], // x
    [0x00,0x00,0x33,0x33,0x33,0x3E,0x30,0x1F], // y
    [0x00,0x00,0x3F,0x19,0x0C,0x26,0x3F,0x00], // z
    [0x38,0x0C,0x0C,0x07,0x0C,0x0C,0x38,0x00], // {
    [0x18,0x18,0x18,0x00,0x18,0x18,0x18,0x00], // |
    [0x07,0x0C,0x0C,0x38,0x0C,0x0C,0x07,0x00], // }
    [0x6E,0x3B,0x00,0x00,0x00,0x00,0x00,0x00], // ~
];

// Eight-pixel console glyphs for every non-ASCII character used by the five
// built-in locales. Rows use the conventional PSF most-significant-bit-first
// encoding and are reversed once by `glyph_rows` for this renderer.
#[rustfmt::skip]
const fn unicode_glyph_rows(character: char) -> [u8; 8] {
    match character {
        'А' => [0x00,0x0E,0x36,0xC6,0xFE,0xC6,0xC6,0xC6], 'Б' => [0x00,0xFC,0xC0,0xC0,0xFC,0xC6,0xC6,0xFC],
        'В' => [0x00,0xF8,0xCC,0xCC,0xFC,0xC6,0xC6,0xFC], 'Г' => [0x00,0xFE,0xC0,0xC0,0xC0,0xC0,0xC0,0xC0],
        'Д' => [0x00,0x3C,0x6C,0x6C,0x6C,0x6C,0xFE,0xC6], 'Е' => [0x00,0xFC,0xC0,0xC0,0xF8,0xC0,0xC0,0xFE],
        'Ж' => [0x00,0xD6,0xD6,0x7C,0x10,0x7C,0xD6,0xD6], 'З' => [0x00,0x3C,0x66,0x06,0x3C,0x06,0xC6,0x7C],
        'И' => [0x00,0xC6,0xC6,0xCE,0xDE,0xF6,0xE6,0xC6], 'Й' => [0x6C,0xBA,0xC6,0xCE,0xDE,0xF6,0xE6,0xC6],
        'К' => [0x00,0xC6,0xCC,0xD8,0xF8,0xCC,0xC6,0xC6], 'Л' => [0x00,0x3E,0x76,0x66,0x66,0x66,0xE6,0xC6],
        'М' => [0x00,0xC6,0xFE,0xD6,0xC6,0xC6,0xC6,0xC6], 'Н' => [0x00,0xC6,0xC6,0xC6,0xC6,0xFE,0xC6,0xC6],
        'О' => [0x00,0x7C,0xEE,0xC6,0xC6,0xC6,0xEE,0x7C], 'П' => [0x00,0xFE,0xC6,0xC6,0xC6,0xC6,0xC6,0xC6],
        'Р' => [0x00,0xFC,0xC6,0xC6,0xFC,0xC0,0xC0,0xC0], 'С' => [0x00,0x7C,0xC6,0xC0,0xC0,0xC0,0xC6,0x7C],
        'Т' => [0x00,0x7E,0x7E,0x18,0x18,0x18,0x18,0x18], 'У' => [0x00,0xC6,0xC6,0xC6,0x7E,0x06,0xC6,0x7C],
        'Ф' => [0x10,0x7C,0xD6,0xD6,0xD6,0x7C,0x10,0x38], 'Х' => [0x00,0xC6,0x6C,0x38,0x10,0x38,0x6C,0xC6],
        'Ц' => [0x00,0xCC,0xCC,0xCC,0xCC,0xCC,0xFE,0x06], 'Ч' => [0x00,0xC6,0xC6,0xC6,0x7E,0x06,0x06,0x06],
        'Ш' => [0x00,0xD6,0xD6,0xD6,0xD6,0xD6,0xD6,0xFE], 'Щ' => [0x00,0xD6,0xD6,0xD6,0xD6,0xD6,0xFE,0x06],
        'Ъ' => [0x00,0xF0,0x30,0x38,0x3C,0x36,0x36,0x3C], 'Ы' => [0x00,0xC2,0xC2,0xE2,0xF2,0xDA,0xDA,0xF2],
        'Ь' => [0x00,0xC0,0xC0,0xF0,0xD8,0xCC,0xCC,0xF8], 'Э' => [0x00,0x7C,0xC6,0x06,0x7E,0x06,0xC6,0x7C],
        'Ю' => [0x00,0x9C,0xB6,0xB6,0xF6,0xB6,0xB6,0x9C], 'Я' => [0x00,0x7E,0xC6,0xC6,0x7E,0x36,0xE6,0xC6],
        'Ё' => [0x6C,0xFE,0x62,0x60,0x78,0x60,0x62,0xFE],
        'а' => [0x00,0x00,0x00,0x78,0x0C,0x7C,0xCC,0x7E], 'б' => [0x00,0x0C,0x38,0xE0,0xF8,0xCC,0xCC,0x78],
        'в' => [0x00,0x00,0x00,0xFC,0xCC,0xFC,0xC6,0xFC], 'г' => [0x00,0x00,0x00,0xFE,0xC0,0xC0,0xC0,0xC0],
        'д' => [0x00,0x00,0x00,0x3C,0x6C,0x6C,0xFE,0xC6], 'е' => [0x00,0x00,0x00,0x7C,0xC6,0xFE,0xC0,0x7E],
        'ж' => [0x00,0x00,0x00,0xD6,0x7C,0x10,0x7C,0xD6], 'з' => [0x00,0x00,0x00,0x7C,0xC6,0x1C,0xC6,0x7C],
        'и' => [0x00,0x00,0x00,0xC6,0xC6,0xDE,0xF6,0xC6], 'й' => [0x00,0x6C,0x7C,0x82,0xC6,0xDE,0xF6,0xC6],
        'к' => [0x00,0x00,0x00,0xC6,0xCC,0xF8,0xCC,0xC6], 'л' => [0x00,0x00,0x00,0x3E,0x76,0x66,0xE6,0xC6],
        'м' => [0x00,0x00,0x00,0xC6,0xEE,0xFE,0xD6,0xC6], 'н' => [0x00,0x00,0x00,0xCC,0xCC,0xFC,0xCC,0xCC],
        'о' => [0x00,0x00,0x00,0x7C,0xEE,0xC6,0xEE,0x7C], 'п' => [0x00,0x00,0x00,0xFE,0xC6,0xC6,0xC6,0xC6],
        'р' => [0x00,0x00,0xFC,0xC6,0xC6,0xFC,0xC0,0xC0], 'с' => [0x00,0x00,0x00,0x7C,0xE6,0xC0,0xE6,0x7C],
        'т' => [0x00,0x00,0x00,0x7E,0x7E,0x18,0x18,0x18], 'у' => [0x00,0x00,0xC6,0xC6,0x7E,0x06,0xC6,0x7C],
        'ф' => [0x00,0x10,0x7C,0xD6,0xD6,0x7C,0x10,0x10], 'х' => [0x00,0x00,0x00,0xEE,0x6C,0x38,0x6C,0xC6],
        'ц' => [0x00,0x00,0xCC,0xCC,0xCC,0xCC,0xFE,0x06], 'ч' => [0x00,0x00,0x00,0xC6,0xC6,0x7E,0x06,0x06],
        'ш' => [0x00,0x00,0x00,0xD6,0xD6,0xD6,0xD6,0xFE], 'щ' => [0x00,0x00,0xD6,0xD6,0xD6,0xD6,0xFE,0x06],
        'ъ' => [0x00,0x00,0x00,0xF0,0x30,0x3C,0x36,0x3C], 'ы' => [0x00,0x00,0x00,0xC2,0xC2,0xF2,0xDA,0xF2],
        'ь' => [0x00,0x00,0x00,0xC0,0xC0,0xFC,0xC6,0xFC], 'э' => [0x00,0x00,0x00,0x7C,0xC6,0x1E,0xC6,0x7C],
        'ю' => [0x00,0x00,0x00,0x9C,0xB6,0xF6,0xB6,0x9C], 'я' => [0x00,0x00,0x00,0x7E,0xC6,0x7E,0x36,0xE6],
        'ё' => [0x00,0x6C,0x00,0x7C,0xC6,0xFE,0xC0,0x7E],
        'א' => [0x00,0xC6,0x66,0x76,0xDC,0xCC,0xC6,0x00], 'ב' => [0x00,0xF8,0x0C,0x0C,0x0C,0x0C,0xFE,0x00],
        'ג' => [0x00,0x38,0x0C,0x0C,0x1C,0x34,0xE6,0x00], 'ד' => [0x00,0xFE,0x0C,0x0C,0x0C,0x0C,0x0C,0x00],
        'ה' => [0x00,0xFC,0x06,0x06,0xC6,0xC6,0xC6,0x00], 'ו' => [0x00,0x70,0x18,0x18,0x18,0x18,0x18,0x00],
        'ז' => [0x00,0x3C,0x18,0x18,0x0C,0x18,0x30,0x00], 'ח' => [0x00,0xFC,0x66,0xC6,0xC6,0xC6,0xC6,0x00],
        'ט' => [0x00,0xCC,0xD6,0xD6,0xC6,0xCC,0x78,0x00], 'י' => [0x00,0x38,0x0C,0x0C,0x18,0x00,0x00,0x00],
        'ך' => [0x00,0xFC,0x06,0x06,0x0C,0x0C,0x0C,0x0E], 'כ' => [0x00,0xFC,0x06,0x06,0x06,0x06,0xFC,0x00],
        'ל' => [0xC0,0xFC,0x06,0x06,0x0C,0x18,0x18,0x00], 'ם' => [0x00,0xFC,0x66,0xC6,0xC6,0xC6,0xFE,0x00],
        'מ' => [0x00,0xDC,0x76,0x66,0xC6,0xC6,0xDE,0x00], 'ן' => [0x00,0x38,0x0C,0x18,0x18,0x18,0x18,0x1C],
        'נ' => [0x00,0x38,0x0C,0x0C,0x0C,0x0C,0x7C,0x00], 'ס' => [0x00,0xFC,0x66,0xC6,0xC6,0xCC,0x78,0x00],
        'ע' => [0x00,0x66,0x66,0x66,0x66,0x36,0xFC,0x00], 'ף' => [0x00,0xF8,0x4C,0xCC,0xEC,0x0C,0x0C,0x0E],
        'פ' => [0x00,0xFC,0x46,0xC6,0xE6,0x06,0xFE,0x00], 'ץ' => [0x00,0x66,0x66,0x66,0x7C,0x60,0x60,0x70],
        'צ' => [0x00,0x66,0x36,0x1C,0x0C,0x06,0x7E,0x00], 'ק' => [0x00,0xFC,0x06,0x66,0x6C,0x6E,0x60,0x60],
        'ר' => [0x00,0x00,0xFC,0x06,0x06,0x06,0x06,0x06], 'ש' => [0x00,0x00,0xD6,0xD6,0xD6,0xF6,0xC6,0x7C],
        'ת' => [0x00,0x00,0xFC,0x66,0x66,0x66,0xE6,0xE6],
        'Ä' => [0xC6,0x38,0x6C,0xC6,0xC6,0xFE,0xC6,0xC6], 'Ö' => [0xC6,0x7C,0xC6,0xC6,0xC6,0xC6,0xC6,0x7C],
        'Ü' => [0xC6,0x00,0xC6,0xC6,0xC6,0xC6,0xC6,0x7C], 'ä' => [0x00,0xCC,0x00,0x78,0x0C,0x7C,0xCC,0x76],
        'ö' => [0x00,0x6C,0x00,0x7C,0xC6,0xC6,0xC6,0x7C], 'ü' => [0x00,0xCC,0x00,0xCC,0xCC,0xCC,0xCC,0x76],
        'ß' => [0x00,0x7C,0xC6,0xFC,0xC6,0xC6,0xFC,0xC0],
        'Ĉ' => [0x7C,0x82,0x7C,0xC6,0xC0,0xC0,0xC6,0x7C], 'Ĝ' => [0x3C,0x42,0x3C,0x66,0xC0,0xCE,0x66,0x3E],
        'Ĥ' => [0x38,0x44,0xC6,0xC6,0xFE,0xC6,0xC6,0xC6], 'Ĵ' => [0x3C,0x42,0x3C,0x18,0x18,0x18,0xD8,0x70],
        'Ŝ' => [0x38,0x6C,0x38,0x6C,0x60,0x3C,0xC6,0x7C], 'Ŭ' => [0xC6,0x7C,0x00,0xC6,0xC6,0xC6,0xC6,0x7C],
        'ĉ' => [0x00,0x78,0x84,0x78,0xCC,0xC0,0xCC,0x78], 'ĝ' => [0x38,0x6C,0x00,0x7E,0xCC,0x7C,0x0C,0xF8],
        'ĥ' => [0x06,0xE9,0x60,0x60,0x6C,0x76,0x66,0xE6], 'ĵ' => [0x1C,0x36,0x00,0x1C,0x0C,0x0C,0xCC,0x78],
        'ŝ' => [0x20,0x70,0x88,0x70,0xC0,0x78,0x0C,0xF8], 'ŭ' => [0x00,0xCC,0x78,0x00,0xCC,0xCC,0xCC,0x76],
        _ => [0; 8],
    }
}

fn glyph_rows(character: char) -> [u8; 8] {
    if character.is_ascii() && (b' '..=b'~').contains(&(character as u8)) {
        return FONT8X8_BASIC[(character as u8 - b' ') as usize];
    }
    let rows = unicode_glyph_rows(character);
    if rows == [0; 8] {
        FONT8X8_BASIC[(b'?' - b' ') as usize]
    } else {
        rows.map(u8::reverse_bits)
    }
}

fn styled_glyph_rows(character: char, style: FontStyle) -> [u8; 8] {
    let mut rows = glyph_rows(character);
    apply_font_face(&mut rows, style.face);
    for row in &mut rows {
        *row = apply_font_weight(*row, style.weight);
    }
    rows
}

const fn is_hebrew(character: char) -> bool {
    matches!(character as u32, 0x0590..=0x05FF)
}

fn apply_font_face(rows: &mut [u8; 8], face: FontFace) {
    match face {
        FontFace::System => {}
        FontFace::Rounded => {
            if let Some((top, bottom)) = glyph_extents(rows) {
                rows[top] = trim_terminal_pixels(rows[top]);
                if bottom != top {
                    rows[bottom] = trim_terminal_pixels(rows[bottom]);
                }
            }
        }
        FontFace::Serif => {
            if let Some((top, bottom)) = glyph_extents(rows) {
                rows[top] = expand_row(rows[top]);
                rows[bottom] = expand_row(rows[bottom]);
            }
        }
        FontFace::Compact => {
            for row in rows {
                *row = compact_row(*row);
            }
        }
        FontFace::Slanted => {
            for (index, row) in rows.iter_mut().enumerate() {
                *row = match index {
                    0..=2 => *row >> 1,
                    3..=4 => *row,
                    _ => *row << 1,
                };
            }
        }
    }
}

fn glyph_extents(rows: &[u8; 8]) -> Option<(usize, usize)> {
    let top = rows.iter().position(|row| *row != 0)?;
    let bottom = rows.iter().rposition(|row| *row != 0)?;
    Some((top, bottom))
}

fn trim_terminal_pixels(row: u8) -> u8 {
    if row.count_ones() < 4 {
        return row;
    }
    let first = row.trailing_zeros() as u8;
    let last = 7 - row.leading_zeros() as u8;
    row & !(1_u8 << first) & !(1_u8 << last)
}

fn expand_row(row: u8) -> u8 {
    row | (row << 1) | (row >> 1)
}

fn compact_row(row: u8) -> u8 {
    let mut compact = 0_u8;
    for source in 0..8 {
        if row & (1_u8 << source) != 0 {
            let target = 1 + source * 6 / 8;
            compact |= 1_u8 << target;
        }
    }
    compact
}

fn apply_font_weight(row: u8, weight: FontWeight) -> u8 {
    match weight {
        FontWeight::Light => lighten_row(row),
        FontWeight::Regular => row,
        FontWeight::Bold => row | (row << 1),
    }
}

fn lighten_row(row: u8) -> u8 {
    let mut result = row;
    let mut column = 0_u8;
    while column < 8 {
        if row & (1_u8 << column) == 0 {
            column += 1;
            continue;
        }
        let start = column;
        while column < 8 && row & (1_u8 << column) != 0 {
            column += 1;
        }
        if column - start >= 3 {
            result &= !(1_u8 << (column - 1));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firmware_assigned_vga_apertures_replace_the_old_fixed_address() {
        for address in [0x8000_0000, 0xFD00_0000] {
            assert_eq!(
                decode_aperture(address | 8, 0, 256),
                Some(FramebufferAperture {
                    address: address as usize,
                    bytes: 16 * 1024 * 1024,
                })
            );
        }
        // A 32-bit BAR must not interpret the next independent BAR as its
        // upper address. A 64-bit BAR must include its actual upper dword.
        assert_eq!(
            decode_aperture(0x8000_0008, 0xFFFF_FFFF, 256),
            decode_aperture(0x8000_000C, 0, 256)
        );
        assert_eq!(decode_aperture(0x8000_000C, 1, 256), None);
    }

    #[test]
    fn unsafe_or_unmapped_vga_apertures_are_rejected() {
        for (low, high, memory) in [
            (0x8000_0001, 0, 256),      // I/O BAR
            (0x8000_0002, 0, 256),      // below-1-MiB memory BAR
            (0x8000_0006, 0, 256),      // reserved BAR encoding
            (0, 0, 256),                // unassigned BAR
            (0x2000_0008, 0, 256),      // low RAM window
            (0x8000_1008, 0, 256),      // not aligned to the reported VRAM size
            (0xFF00_0008, 0, 512),      // aperture crosses the 4-GiB mapping limit
            (0x8000_0008, 0, 0),        // absent size register
            (0x8000_0008, 0, u16::MAX), // unsupported size register
            (0x8000_0008, 0, 255),      // impossible PCI aperture size
        ] {
            assert_eq!(decode_aperture(low, high, memory), None);
        }
        // An aperture ending exactly at 4 GiB stays completely mapped.
        assert!(decode_aperture(0xFF00_0008, 0, 256).is_some());
    }

    #[test]
    fn actual_vram_bounds_control_mode_acceptance() {
        let requested = DisplayMode::P480;
        let mut mode = requested.hardware_mode();
        // The reported canvas can be taller than the two requested pages.
        mode.virtual_height = 6553;
        assert!(mode_fits_aperture(requested, mode, 16 * 1024 * 1024));
        assert!(!mode_fits_aperture(requested, mode, 8 * 1024 * 1024));
        mode.virtual_height = 6554;
        assert!(!mode_fits_aperture(requested, mode, 16 * 1024 * 1024));
        mode = requested.hardware_mode();
        assert!(!mode_fits_aperture(requested, mode, 1024 * 1024));
        mode.virtual_width = 800;
        assert!(!mode_fits_aperture(requested, mode, 16 * 1024 * 1024));
        assert!(mode_fits_aperture(
            DisplayMode::P1080,
            DisplayMode::P1080.hardware_mode(),
            16 * 1024 * 1024
        ));
    }

    const FONT_FACES: [FontFace; 5] = [
        FontFace::System,
        FontFace::Rounded,
        FontFace::Serif,
        FontFace::Compact,
        FontFace::Slanted,
    ];
    const FONT_WEIGHTS: [FontWeight; 3] =
        [FontWeight::Light, FontWeight::Regular, FontWeight::Bold];

    #[test]
    fn progressive_modes_have_expected_geometry() {
        assert_eq!(DisplayMode::default(), DisplayMode::P480);
        assert_eq!(DisplayMode::P480.dimensions(), (640, 480));
        assert_eq!(DisplayMode::P720.dimensions(), (1280, 720));
        assert_eq!(DisplayMode::P1080.dimensions(), (1920, 1080));
        assert_eq!(DisplayMode::ALL.len(), 3);
    }

    #[test]
    fn font_style_ids_and_defaults_are_stable() {
        assert_eq!(FontFace::default(), FontFace::System);
        assert_eq!(FontWeight::default(), FontWeight::Regular);
        assert_eq!(
            FontStyle::default(),
            FontStyle::new(FontFace::System, FontWeight::Regular)
        );
        for (id, face) in FONT_FACES.iter().copied().enumerate() {
            assert_eq!(face.persisted(), id as u8);
            assert_eq!(FontFace::from_persisted(id as u8), Some(face));
        }
        for (id, weight) in FONT_WEIGHTS.iter().copied().enumerate() {
            assert_eq!(weight.persisted(), id as u8);
            assert_eq!(FontWeight::from_persisted(id as u8), Some(weight));
        }
        assert_eq!(FontFace::from_persisted(5), None);
        assert_eq!(FontWeight::from_persisted(3), None);
        assert_eq!(FontStyle::decode(u8::MAX), FontStyle::default());
        assert_eq!(
            styled_glyph_rows('A', FontStyle::default()),
            glyph_rows('A')
        );
    }

    #[test]
    fn every_font_face_changes_real_glyph_pixels() {
        let regular = FontWeight::Regular;
        let mut drawings = [[0_u8; 8]; FONT_FACES.len()];
        for (index, face) in FONT_FACES.iter().copied().enumerate() {
            drawings[index] = styled_glyph_rows('A', FontStyle::new(face, regular));
            assert!(drawings[index].iter().any(|row| *row != 0));
        }
        for left in 0..drawings.len() {
            for right in left + 1..drawings.len() {
                assert_ne!(drawings[left], drawings[right]);
            }
        }
    }

    #[test]
    fn font_weights_change_stroke_pixel_count() {
        let lit_pixels = |rows: [u8; 8]| {
            rows.iter()
                .fold(0_u32, |total, row| total + row.count_ones())
        };
        let light = lit_pixels(styled_glyph_rows(
            'E',
            FontStyle::new(FontFace::System, FontWeight::Light),
        ));
        let regular = lit_pixels(styled_glyph_rows(
            'E',
            FontStyle::new(FontFace::System, FontWeight::Regular),
        ));
        let bold = lit_pixels(styled_glyph_rows(
            'E',
            FontStyle::new(FontFace::System, FontWeight::Bold),
        ));
        assert!(light < regular);
        assert!(regular < bold);
    }

    #[test]
    fn font_faces_keep_the_original_bounded_cell_metrics() {
        for face in FONT_FACES {
            for weight in FONT_WEIGHTS {
                let rows = styled_glyph_rows('W', FontStyle::new(face, weight));
                assert_eq!(rows.len(), 8);
            }
        }
        assert_eq!(640 / text_advance(1), 80);
        assert!(80 * text_advance(1) <= DisplayMode::P480.width() as i32);
        assert!(26 * text_line_height(2) <= DisplayMode::P480.height() as i32);
        assert!(27 * text_line_height(2) > DisplayMode::P480.height() as i32);
    }

    #[test]
    fn every_selectable_mode_fits_the_mapped_aperture() {
        for mode in DisplayMode::ALL {
            assert!(mode.fits_aperture());
            assert_eq!(mode.stride_bytes(), mode.width() * BYTES_PER_PIXEL);
            assert_eq!(
                mode.scanout_bytes(),
                mode.width() * mode.height() * BYTES_PER_PIXEL
            );
            assert!(mode.scanout_bytes() <= LFB_APERTURE_BYTES);
            assert!(mode.double_buffer_bytes() <= LFB_APERTURE_BYTES);
        }
    }

    #[test]
    fn persisted_mode_values_are_stable_and_validated() {
        for mode in DisplayMode::ALL {
            assert_eq!(DisplayMode::from_persisted(mode.persisted()), Some(mode));
            assert_eq!(
                DisplayMode::from_dimensions(mode.width(), mode.height()),
                Some(mode)
            );
        }
        assert_eq!(DisplayMode::from_persisted(3), None);
        assert_eq!(DisplayMode::from_persisted(u8::MAX), None);
        assert_eq!(DisplayMode::from_dimensions(800, 600), None);
    }

    #[test]
    fn requested_mode_round_trips_without_changing_active_scanout() {
        ACTIVE_DISPLAY_MODE.store(NO_ACTIVE_MODE, Ordering::Release);
        assert!(request_mode(DisplayMode::P480));
        assert_eq!(requested_mode(), DisplayMode::P480);
        assert_eq!(current_mode(), DisplayMode::P480);
        assert_eq!(width(), 640);
        assert_eq!(height(), 480);
        assert_eq!(active_display_mode(), None);
        assert!(request_mode(DisplayMode::P480));
    }

    #[test]
    fn damage_clipping_preserves_visible_rectangle_geometry() {
        assert_eq!(
            clip_region(DamageRegion::new(10, 20, 30, 40), 640, 480),
            Some(ClippedRegion {
                left: 10,
                top: 20,
                right: 40,
                bottom: 60,
            })
        );
        assert_eq!(
            clip_region(DamageRegion::new(-8, -6, 20, 18), 640, 480),
            Some(ClippedRegion {
                left: 0,
                top: 0,
                right: 12,
                bottom: 12,
            })
        );
        assert_eq!(
            clip_region(DamageRegion::new(630, 470, 40, 30), 640, 480),
            Some(ClippedRegion {
                left: 630,
                top: 470,
                right: 640,
                bottom: 480,
            })
        );
    }

    #[test]
    fn damage_clipping_rejects_empty_or_offscreen_rectangles() {
        assert_eq!(
            clip_region(DamageRegion::new(20, 20, 0, 10), 640, 480),
            None
        );
        assert_eq!(
            clip_region(DamageRegion::new(20, 20, 10, -1), 640, 480),
            None
        );
        assert_eq!(
            clip_region(DamageRegion::new(700, 20, 10, 10), 640, 480),
            None
        );
        assert_eq!(
            clip_region(
                DamageRegion::new(i32::MAX, i32::MAX, i32::MAX, i32::MAX),
                640,
                480
            ),
            None
        );
    }

    #[test]
    fn damage_normalization_coalesces_overlapping_rectangles() {
        let normalized = normalize_damage(
            &[
                DamageRegion::new(10, 20, 30, 10),
                DamageRegion::new(25, 20, 30, 10),
            ],
            640,
            480,
        );
        assert_eq!(normalized.submitted_regions, 2);
        assert_eq!(normalized.submitted_pixels, 600);
        assert_eq!(normalized.len, 1);
        assert_eq!(
            normalized.regions[0],
            ClippedRegion {
                left: 10,
                top: 20,
                right: 55,
                bottom: 30,
            }
        );
        assert_eq!(normalized.copied_regions(), 1);
        assert_eq!(normalized.copied_pixels(), 450);
        assert!(!normalized.collapsed);
    }

    #[test]
    fn damage_normalization_merges_aligned_edges_without_expanding_corner_damage() {
        let edge = normalize_damage(
            &[
                DamageRegion::new(0, 0, 10, 10),
                DamageRegion::new(10, 0, 5, 10),
            ],
            640,
            480,
        );
        assert_eq!(edge.len, 1);
        assert_eq!(edge.copied_pixels(), 150);

        let corner = normalize_damage(
            &[
                DamageRegion::new(0, 0, 10, 10),
                DamageRegion::new(10, 10, 5, 5),
            ],
            640,
            480,
        );
        assert_eq!(corner.len, 2);
        assert_eq!(corner.copied_pixels(), 125);
        assert_eq!(
            corner.regions[0],
            ClippedRegion {
                left: 0,
                top: 0,
                right: 10,
                bottom: 10,
            }
        );
        assert_eq!(
            corner.regions[1],
            ClippedRegion {
                left: 10,
                top: 10,
                right: 15,
                bottom: 15,
            }
        );
    }

    #[test]
    fn damage_normalization_does_not_turn_crossed_lines_into_a_full_screen_copy() {
        let normalized = normalize_damage(
            &[
                DamageRegion::new(0, 100, 640, 1),
                DamageRegion::new(320, 0, 1, 480),
            ],
            640,
            480,
        );

        assert_eq!(normalized.len, 2);
        assert_eq!(normalized.submitted_pixels, 1_120);
        assert_eq!(normalized.copied_pixels(), 1_120);
        assert!(!normalized.regions[0].can_merge_without_extra_copy(normalized.regions[1]));
    }

    #[test]
    fn damage_normalization_restarts_until_transitive_merges_finish() {
        let normalized = normalize_damage(
            &[
                DamageRegion::new(0, 5, 10, 10),
                DamageRegion::new(20, 5, 10, 10),
                DamageRegion::new(10, 5, 10, 10),
            ],
            640,
            480,
        );
        assert_eq!(normalized.len, 1);
        assert_eq!(
            normalized.regions[0],
            ClippedRegion {
                left: 0,
                top: 5,
                right: 30,
                bottom: 15,
            }
        );
        assert_eq!(normalized.submitted_pixels, 300);
        assert_eq!(normalized.copied_pixels(), 300);
    }

    #[test]
    fn damage_normalization_clips_before_merging_and_ignores_empty_entries() {
        let normalized = normalize_damage(
            &[
                DamageRegion::new(-5, 4, 10, 6),
                DamageRegion::new(5, 4, 8, 6),
                DamageRegion::new(700, 20, 10, 10),
                DamageRegion::new(1, 1, 0, 8),
            ],
            640,
            480,
        );
        assert_eq!(normalized.submitted_regions, 4);
        assert_eq!(normalized.submitted_pixels, 78);
        assert_eq!(normalized.len, 1);
        assert_eq!(
            normalized.regions[0],
            ClippedRegion {
                left: 0,
                top: 4,
                right: 13,
                bottom: 10,
            }
        );
        assert_eq!(normalized.copied_pixels(), 78);
    }

    #[test]
    fn damage_normalization_preserves_separated_regions() {
        let normalized = normalize_damage(
            &[
                DamageRegion::new(5, 5, 10, 10),
                DamageRegion::new(30, 40, 5, 7),
                DamageRegion::new(100, 100, -1, 4),
            ],
            640,
            480,
        );
        assert_eq!(normalized.submitted_regions, 3);
        assert_eq!(normalized.submitted_pixels, 135);
        assert_eq!(normalized.len, 2);
        assert_eq!(normalized.copied_regions(), 2);
        assert_eq!(normalized.copied_pixels(), 135);
        assert!(!normalized.regions[0].touches_or_overlaps(normalized.regions[1]));
    }

    #[test]
    fn damage_normalization_is_empty_when_nothing_is_visible() {
        let normalized = normalize_damage(
            &[
                DamageRegion::new(-20, -20, 5, 5),
                DamageRegion::new(10, 10, 0, 20),
            ],
            640,
            480,
        );
        assert_eq!(normalized.submitted_regions, 2);
        assert_eq!(normalized.submitted_pixels, 0);
        assert_eq!(normalized.len, 0);
        assert_eq!(normalized.copied_regions(), 0);
        assert_eq!(normalized.copied_pixels(), 0);
        assert!(!normalized.collapsed);
    }

    #[test]
    fn damage_normalization_collapses_overflow_without_losing_later_damage() {
        let mut damage = [DamageRegion::new(0, 0, 0, 0); MAX_DAMAGE_REGIONS + 2];
        for (index, region) in damage[..MAX_DAMAGE_REGIONS + 1].iter_mut().enumerate() {
            *region = DamageRegion::new((index * 3) as i32, 0, 1, 1);
        }
        damage[MAX_DAMAGE_REGIONS + 1] = DamageRegion::new(200, 20, 2, 2);

        let normalized = normalize_damage(&damage, 640, 480);
        assert_eq!(
            normalized.submitted_regions,
            (MAX_DAMAGE_REGIONS + 2) as u64
        );
        assert_eq!(
            normalized.submitted_pixels,
            (MAX_DAMAGE_REGIONS + 1) as u64 + 4
        );
        assert_eq!(normalized.len, 1);
        assert!(normalized.collapsed);
        assert_eq!(
            normalized.regions[0],
            ClippedRegion {
                left: 0,
                top: 0,
                right: 202,
                bottom: 22,
            }
        );
        assert_eq!(normalized.copied_regions(), 1);
        assert_eq!(normalized.copied_pixels(), 202 * 22);
    }

    #[test]
    fn normalized_regions_leave_no_cost_effective_merge() {
        let normalized = normalize_damage(
            &[
                DamageRegion::new(10, 10, 30, 4),
                DamageRegion::new(20, 0, 4, 30),
                DamageRegion::new(200, 200, 10, 10),
                DamageRegion::new(205, 205, 20, 20),
                DamageRegion::new(400, 300, 8, 8),
            ],
            640,
            480,
        );
        for left in 0..normalized.len {
            for right in left + 1..normalized.len {
                assert!(!normalized.regions[left]
                    .can_merge_without_extra_copy(normalized.regions[right]));
            }
        }
    }

    #[test]
    fn v3_full_damage_promotion_preserves_submission_diagnostics() {
        let mut normalized = normalize_damage(&[DamageRegion::new(10, 20, 30, 40)], 640, 480);
        let submitted_regions = normalized.submitted_regions;
        let submitted_pixels = normalized.submitted_pixels;
        promote_to_full_damage(&mut normalized);
        assert!(is_full_damage(&normalized));
        assert_eq!(normalized.copied_pixels(), 640 * 480);
        assert_eq!(normalized.submitted_regions, submitted_regions);
        assert_eq!(normalized.submitted_pixels, submitted_pixels);
    }

    #[test]
    fn v3_readback_sampling_detects_and_accepts_scanout_content() {
        let mut shadow = [0_u32; 64];
        let mut scanout = [0_u32; 64];
        for (index, pixel) in shadow.iter_mut().enumerate() {
            *pixel = index as u32 * 17;
        }
        scanout.copy_from_slice(&shadow);
        assert!(verify_samples(
            shadow.as_ptr(),
            scanout.as_ptr(),
            shadow.len()
        ));
        scanout[32] ^= 1;
        assert!(!verify_samples(
            shadow.as_ptr(),
            scanout.as_ptr(),
            shadow.len()
        ));
    }

    #[test]
    fn string_fill_writes_exactly_the_requested_dwords() {
        let mut pixels = [0x1111_1111_u32; 12];
        // SAFETY: indices 2..10 are a writable range inside `pixels`.
        unsafe { fill_dwords(pixels.as_mut_ptr().add(2), 0xA5A5_5A5A, 8) };
        assert_eq!(pixels[..2], [0x1111_1111; 2]);
        assert_eq!(pixels[2..10], [0xA5A5_5A5A; 8]);
        assert_eq!(pixels[10..], [0x1111_1111; 2]);
    }

    #[test]
    fn string_copy_writes_exactly_the_requested_dwords() {
        let source = [
            0x10_u32, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xA0,
        ];
        let mut destination = [0xDEAD_BEEF_u32; 12];
        // SAFETY: both five-dword ranges are valid and the arrays do not
        // overlap.
        unsafe { copy_dwords(source.as_ptr().add(2), destination.as_mut_ptr().add(4), 5) };
        assert_eq!(destination[..4], [0xDEAD_BEEF; 4]);
        assert_eq!(destination[4..9], source[2..7]);
        assert_eq!(destination[9..], [0xDEAD_BEEF; 3]);
    }

    #[test]
    fn every_builtin_locale_character_has_a_real_glyph() {
        use crate::locale::{Locale, Text};

        for locale in Locale::ALL {
            let samples = [
                locale.name(),
                locale.text(Text::OpenShell),
                locale.text(Text::DesktopWillClose),
                locale.category(1),
                locale.category(15),
                locale.app(0),
                locale.app(8),
            ];
            for sample in samples {
                for character in sample.chars().filter(|character| !character.is_ascii()) {
                    assert_ne!(unicode_glyph_rows(character), [0; 8], "missing {character}");
                }
            }
        }
    }
}
