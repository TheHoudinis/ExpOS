use crate::port;
use core::sync::atomic::{AtomicU8, Ordering};

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

/// A user-selectable progressive display mode.
///
/// The discriminants are stable because settings persistence may store them.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DisplayMode {
    P480 = 0,
    P720 = 1,
    #[default]
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
            virtual_height: self.height() as u16,
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

static REQUESTED_MODE: AtomicU8 = AtomicU8::new(DisplayMode::P1080 as u8);
static ACTIVE_DISPLAY_MODE: AtomicU8 = AtomicU8::new(NO_ACTIVE_MODE);

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
    program_mode(requested_mode())
}

fn program_mode(requested: DisplayMode) -> bool {
    ACTIVE_DISPLAY_MODE.store(NO_ACTIVE_MODE, Ordering::Release);
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
    });
    if !configured {
        if let Some(mode) = active {
            crate::slog!(
                "HEXA_DISPLAY_MODE_REJECTED requested={}x{}x{} actual={}x{}x{} virtual={}x{}\r\n",
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
                "HEXA_DISPLAY_MODE_REJECTED requested={}x{}x{} actual=disabled\r\n",
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
    ACTIVE_DISPLAY_MODE.store(requested.persisted(), Ordering::Release);
    crate::slog!(
        "HEXA_DISPLAY_MODE width={} height={} bpp={} stride={} bytes={} preset={}\r\n",
        mode.width,
        mode.height,
        mode.bits_per_pixel,
        mode.stride_bytes(),
        mode.scanout_bytes(),
        requested.label()
    );
    clear(color::BACKGROUND);
    true
}

pub fn exit() {
    ACTIVE_DISPLAY_MODE.store(NO_ACTIVE_MODE, Ordering::Release);
    write(INDEX_ENABLE, DISABLED);
    restore_vga_text_mode();
}

pub fn clear(value: u32) {
    let pointer = LFB as *mut u32;
    let pixels = scanout_bytes() / BYTES_PER_PIXEL;
    for offset in 0..pixels {
        // SAFETY: the bootstrap maps the QEMU/Bochs LFB MMIO range and this
        // module is the only graphical writer while display mode is active.
        unsafe { core::ptr::write_volatile(pointer.add(offset), value) };
    }
}

pub fn pixel(x: i32, y: i32, value: u32) {
    let mode = current_mode();
    if x < 0 || y < 0 || x >= mode.width() as i32 || y >= mode.height() as i32 {
        return;
    }
    let stride_pixels = mode.stride_bytes() / BYTES_PER_PIXEL;
    let offset = y as usize * stride_pixels + x as usize;
    unsafe { core::ptr::write_volatile((LFB as *mut u32).add(offset), value) };
}

pub fn read_pixel(x: i32, y: i32) -> u32 {
    let mode = current_mode();
    if x < 0 || y < 0 || x >= mode.width() as i32 || y >= mode.height() as i32 {
        return 0;
    }
    let stride_pixels = mode.stride_bytes() / BYTES_PER_PIXEL;
    let offset = y as usize * stride_pixels + x as usize;
    unsafe { core::ptr::read_volatile((LFB as *const u32).add(offset)) }
}

pub fn rect(x: i32, y: i32, width: i32, height: i32, value: u32) {
    let mode = current_mode();
    let mode_width = mode.width() as i32;
    let mode_height = mode.height() as i32;
    let left = x.max(0).min(mode_width);
    let top = y.max(0).min(mode_height);
    let right = x.saturating_add(width).max(0).min(mode_width);
    let bottom = y.saturating_add(height).max(0).min(mode_height);
    let stride_pixels = mode.stride_bytes() / BYTES_PER_PIXEL;
    let pointer = LFB as *mut u32;
    for row in top..bottom {
        let offset = row as usize * stride_pixels;
        for column in left..right {
            unsafe { core::ptr::write_volatile(pointer.add(offset + column as usize), value) };
        }
    }
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
    for byte in value.bytes() {
        match byte {
            b'\n' => {
                x = origin;
                y += text_line_height(scale);
            }
            b'\t' => x += advance * 4,
            _ => {
                glyph(x, y, byte, color, scale);
                x += advance;
            }
        }
    }
}

pub fn glyph(x: i32, y: i32, byte: u8, color: u32, scale: i32) {
    let rows = glyph_rows(byte);
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
        for character in 0..=u8::MAX {
            let rows = glyph_rows(character);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progressive_modes_have_expected_geometry() {
        assert_eq!(DisplayMode::P480.dimensions(), (640, 480));
        assert_eq!(DisplayMode::P720.dimensions(), (1280, 720));
        assert_eq!(DisplayMode::P1080.dimensions(), (1920, 1080));
        assert_eq!(DisplayMode::ALL.len(), 3);
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
        assert!(request_mode(DisplayMode::P1080));
    }
}
