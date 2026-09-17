use crate::port;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8, Ordering};

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

// QEMU's standard VGA device exposes a 16 MiB prefetchable BAR0 at this
// address in the i440FX machine used by the Makefile. The bootstrap maps the
// complete 0xC000_0000..=0xFFFF_FFFF PCI/MMIO window, so the 7.91 MiB 1080p
// scanout is both inside the BAR aperture and inside the identity map.
pub const LFB_PHYSICAL_ADDRESS: usize = 0xFD00_0000;
pub const LFB_APERTURE_BYTES: usize = 16 * 1024 * 1024;
const _: () = assert!(SCANOUT_BYTES <= LFB_APERTURE_BYTES);

const LFB: usize = LFB_PHYSICAL_ADDRESS;
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

pub fn width() -> usize {
    current_mode().width()
}

pub fn height() -> usize {
    current_mode().height()
}

pub fn stride_bytes() -> usize {
    current_mode().stride_bytes()
}

pub fn scanout_bytes() -> usize {
    current_mode().scanout_bytes()
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
    }
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
    crate::println!("adapter: id={:#06X} bochs-vbe={}", adapter_id, available());
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
    crate::println!("aperture: {} bytes", LFB_APERTURE_BYTES);
}

fn program_mode(requested: DisplayMode) -> bool {
    ACTIVE_DISPLAY_MODE.store(NO_ACTIVE_MODE, Ordering::Release);
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
    if !available() {
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
    let configured = active.is_some_and(|mode| {
        mode.width == desired.width
            && mode.height == desired.height
            && mode.bits_per_pixel == desired.bits_per_pixel
            && mode.virtual_width == desired.virtual_width
            && mode.virtual_height >= desired.height
            && mode.stride_bytes() == requested.stride_bytes()
            && mode.scanout_bytes() == requested.scanout_bytes()
            && mode.scanout_bytes() <= LFB_APERTURE_BYTES
            && mode.aperture_bytes() <= LFB_APERTURE_BYTES
    });
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
    let page_flip = mode.virtual_height as usize >= requested.height() * 2;
    ACTIVE_DISPLAY_MODE.store(requested.persisted(), Ordering::Release);
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

pub fn exit() {
    ACTIVE_DISPLAY_MODE.store(NO_ACTIVE_MODE, Ordering::Release);
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
    let normalized = normalize_damage(
        damage,
        current_mode().width() as i32,
        current_mode().height() as i32,
    );
    SUBMITTED_DAMAGE_REGIONS.fetch_add(normalized.submitted_regions, Ordering::AcqRel);
    SUBMITTED_DAMAGE_PIXELS.fetch_add(normalized.submitted_pixels, Ordering::AcqRel);
    if normalized.collapsed {
        DAMAGE_COLLAPSES.fetch_add(1, Ordering::AcqRel);
    }
    let page_flip = PAGE_FLIP_AVAILABLE.load(Ordering::Acquire);
    if vsync && page_flip && !wait_for_vertical_retrace() {
        VBLANK_TIMEOUTS.fetch_add(1, Ordering::AcqRel);
    }
    let visible = if page_flip {
        let height = current_mode().height() as u16;
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
    let pointer = LFB as *mut u32;
    let pixels = scanout_bytes() / BYTES_PER_PIXEL;
    let page = draw_page_offset_pixels();
    // SAFETY: the validated scanout geometry keeps this complete draw page
    // inside the mapped LFB aperture. The framebuffer is exclusively owned by
    // this module while graphics mode is active.
    unsafe { fill_dwords(pointer.add(page), value, pixels) };
}

pub fn pixel(x: i32, y: i32, value: u32) {
    let mode = current_mode();
    if x < 0 || y < 0 || x >= mode.width() as i32 || y >= mode.height() as i32 {
        return;
    }
    let stride_pixels = mode.stride_bytes() / BYTES_PER_PIXEL;
    let offset = draw_page_offset_pixels() + y as usize * stride_pixels + x as usize;
    unsafe { core::ptr::write_volatile((LFB as *mut u32).add(offset), value) };
}

pub fn read_pixel(x: i32, y: i32) -> u32 {
    let mode = current_mode();
    if x < 0 || y < 0 || x >= mode.width() as i32 || y >= mode.height() as i32 {
        return 0;
    }
    let stride_pixels = mode.stride_bytes() / BYTES_PER_PIXEL;
    let offset = draw_page_offset_pixels() + y as usize * stride_pixels + x as usize;
    unsafe { core::ptr::read_volatile((LFB as *const u32).add(offset)) }
}

pub fn rect(x: i32, y: i32, width: i32, height: i32, value: u32) {
    let mode = current_mode();
    let mode_width = mode.width() as i32;
    let mode_height = mode.height() as i32;
    let Some(clipped) = clip_region(
        DamageRegion::new(x, y, width, height),
        mode_width,
        mode_height,
    ) else {
        return;
    };
    let stride_pixels = mode.stride_bytes() / BYTES_PER_PIXEL;
    let pointer = LFB as *mut u32;
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

fn copy_clipped_region(source_y: u16, target_y: u16, clipped: ClippedRegion) {
    let stride_pixels = stride_bytes() / BYTES_PER_PIXEL;
    let source = source_y as usize * stride_pixels;
    let target = target_y as usize * stride_pixels;
    let pointer = LFB as *mut u32;

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
    let mode = current_mode();
    let mode_width = mode.width() as i32;
    let mode_height = mode.height() as i32;
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

pub fn text(mut x: i32, mut y: i32, value: &str, color: u32, scale: i32) {
    let origin = x;
    let advance = text_advance(scale);
    let style = font_style();
    for byte in value.bytes() {
        match byte {
            b'\n' => {
                x = origin;
                y += text_line_height(scale);
            }
            b'\t' => x += advance * 4,
            _ => {
                glyph_with_style(x, y, byte, color, scale, style);
                x += advance;
            }
        }
    }
}

pub fn glyph(x: i32, y: i32, byte: u8, color: u32, scale: i32) {
    glyph_with_style(x, y, byte, color, scale, font_style());
}

fn glyph_with_style(x: i32, y: i32, byte: u8, color: u32, scale: i32, style: FontStyle) {
    let rows = styled_glyph_rows(byte, style);
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
            let rows = styled_glyph_rows(character, style);
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

fn glyph_rows(byte: u8) -> [u8; 8] {
    if (b' '..=b'~').contains(&byte) {
        FONT8X8_BASIC[(byte - b' ') as usize]
    } else {
        FONT8X8_BASIC[(b'?' - b' ') as usize]
    }
}

fn styled_glyph_rows(byte: u8, style: FontStyle) -> [u8; 8] {
    let mut rows = glyph_rows(byte);
    apply_font_face(&mut rows, style.face);
    for row in &mut rows {
        *row = apply_font_weight(*row, style.weight);
    }
    rows
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
            styled_glyph_rows(b'A', FontStyle::default()),
            glyph_rows(b'A')
        );
    }

    #[test]
    fn every_font_face_changes_real_glyph_pixels() {
        let regular = FontWeight::Regular;
        let mut drawings = [[0_u8; 8]; FONT_FACES.len()];
        for (index, face) in FONT_FACES.iter().copied().enumerate() {
            drawings[index] = styled_glyph_rows(b'A', FontStyle::new(face, regular));
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
            b'E',
            FontStyle::new(FontFace::System, FontWeight::Light),
        ));
        let regular = lit_pixels(styled_glyph_rows(
            b'E',
            FontStyle::new(FontFace::System, FontWeight::Regular),
        ));
        let bold = lit_pixels(styled_glyph_rows(
            b'E',
            FontStyle::new(FontFace::System, FontWeight::Bold),
        ));
        assert!(light < regular);
        assert!(regular < bold);
    }

    #[test]
    fn font_faces_keep_the_original_bounded_cell_metrics() {
        for face in FONT_FACES {
            for weight in FONT_WEIGHTS {
                let rows = styled_glyph_rows(b'W', FontStyle::new(face, weight));
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
}
