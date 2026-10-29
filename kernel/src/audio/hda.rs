//! Intel High Definition Audio output for the QEMU `intel-hda` controller.
//!
//! Codec topology is discovered through bounded immediate commands. The first
//! audio function group, output converter and output pin are selected; stream
//! DMA uses a fixed BDL and interrupt completion.

use crate::{asl, pci};
use core::sync::atomic::{AtomicU16, AtomicU64, AtomicU8, AtomicUsize, Ordering};

const GCAP: usize = 0x00;
const GCTL: usize = 0x08;
const STATESTS: usize = 0x0E;
const INTCTL: usize = 0x20;
const INTSTS: usize = 0x24;
const ICOI: usize = 0x60;
const ICII: usize = 0x64;
const ICIS: usize = 0x68;
const STREAM_BASE: usize = 0x80;
const STREAM_STRIDE: usize = 0x20;
const STREAM_STATUS: usize = 0x03;
const STREAM_CBL: usize = 0x08;
const STREAM_LVI: usize = 0x0C;
const STREAM_FORMAT: usize = 0x12;
const STREAM_BDL_LOW: usize = 0x18;
const STREAM_BDL_HIGH: usize = 0x1C;
const MAX_BDL: usize = 32;
const MAX_POLLS: usize = 2_000_000;
const FORMAT_48K_STEREO_16: u16 = 0x0011;
const NO_IRQ: u8 = u8::MAX;

#[repr(C)]
#[derive(Clone, Copy)]
struct BdlEntry {
    address: u64,
    length: u32,
    flags: u32,
}

#[repr(C, align(128))]
struct Bdl([BdlEntry; MAX_BDL]);

static mut BDL: Bdl = Bdl([BdlEntry {
    address: 0,
    length: 0,
    flags: 0,
}; MAX_BDL]);
static IRQ_BASE: AtomicUsize = AtomicUsize::new(0);
static IRQ_STREAM: AtomicUsize = AtomicUsize::new(0);
static IRQ_LINE: AtomicU8 = AtomicU8::new(NO_IRQ);
static IRQ_EVENTS: AtomicU8 = AtomicU8::new(0);
static IRQ_LAST_STATUS: AtomicU16 = AtomicU16::new(0);
static IRQ_COUNT: AtomicU64 = AtomicU64::new(0);
static CODEC: AtomicU8 = AtomicU8::new(0);
static DAC: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy)]
pub struct Ready {
    pub irq: u8,
    pub codec: u8,
    pub afg: u8,
    pub dac: u8,
    pub pin: u8,
}

pub fn initialize() -> Option<Ready> {
    let function = pci::find(|function| function.class_code == 0x04 && function.subclass == 0x03)?;
    let bar = function.memory_bar(0)?;
    if bar.address < 0x10_0000 || bar.address.checked_add(0x4000)? > 0x1_0000_0000 {
        return None;
    }
    let irq = function.interrupt_line()?;
    function.enable_memory_bus_master();
    function.write(0x04, function.read(0x04) & !(1 << 10));
    let base = bar.address as usize;
    unsafe { write32(base + GCTL, read32(base + GCTL) & !1) };
    if !wait_until(|| unsafe { read32(base + GCTL) & 1 == 0 }) {
        return None;
    }
    unsafe { write32(base + GCTL, read32(base + GCTL) | 1) };
    if !wait_until(|| unsafe { read32(base + GCTL) & 1 != 0 }) {
        return None;
    }
    let codecs = unsafe { read16(base + STATESTS) };
    let codec = (0_u8..15).find(|index| codecs & (1 << index) != 0)?;
    let (root_start, root_count) = node_range(base, codec, 0)?;
    let mut afg = None;
    for node in root_start..root_start.saturating_add(root_count) {
        if verb(base, codec, node, 0xF00, 0x05)? & 0xFF == 1 {
            afg = Some(node);
            break;
        }
    }
    let afg = afg?;
    let (widget_start, widget_count) = node_range(base, codec, afg)?;
    let mut dac = None;
    let mut pin = None;
    for node in widget_start..widget_start.saturating_add(widget_count) {
        let kind = (verb(base, codec, node, 0xF00, 0x09)? >> 20) & 0xF;
        if kind == 0 && dac.is_none() {
            dac = Some(node);
        } else if kind == 4 && pin.is_none() {
            pin = Some(node);
        }
    }
    let (dac, pin) = (dac?, pin?);
    for node in [afg, dac, pin] {
        let _ = verb(base, codec, node, 0x705, 0);
    }
    let _ = verb(base, codec, pin, 0x701, 0);
    let _ = verb(base, codec, pin, 0x707, 0x40);
    let _ = verb(base, codec, dac, 0x706, 0x10);
    let _ = verb16(base, codec, dac, 0x2, FORMAT_48K_STEREO_16);
    let gcap = unsafe { read16(base + GCAP) };
    let input_streams = ((gcap >> 8) & 0xF) as usize;
    let output_streams = ((gcap >> 12) & 0xF) as usize;
    if output_streams == 0 {
        return None;
    }
    let stream = STREAM_BASE + input_streams * STREAM_STRIDE;
    stop_stream(base + stream);
    unsafe {
        write8(base + stream + STREAM_STATUS, 0x1C);
        write32(base + INTCTL, (1 << 31) | (1 << input_streams));
    }
    if !crate::interrupts::enable_legacy_irq(irq) {
        return None;
    }
    let _ = asl::claim(function, asl::Owner::ExpAudio);
    IRQ_BASE.store(base, Ordering::Release);
    IRQ_STREAM.store(stream, Ordering::Release);
    IRQ_LINE.store(irq, Ordering::Release);
    IRQ_EVENTS.store(0, Ordering::Release);
    IRQ_COUNT.store(0, Ordering::Release);
    CODEC.store(codec, Ordering::Release);
    DAC.store(dac, Ordering::Release);
    crate::slog!(
        "EXPOS_HDA_READY codec={} afg={} dac={} pin={} output_stream={} completion=interrupt irq={}\r\n",
        codec,
        afg,
        dac,
        pin,
        input_streams,
        irq
    );
    Some(Ready {
        irq,
        codec,
        afg,
        dac,
        pin,
    })
}

pub fn submit(sample_address: usize, bytes: usize) -> bool {
    let base = IRQ_BASE.load(Ordering::Acquire);
    let stream = IRQ_STREAM.load(Ordering::Acquire);
    if base == 0 || bytes == 0 || bytes > u32::MAX as usize {
        return false;
    }
    let entries = bytes.div_ceil(128 * 1024);
    if entries == 0 || entries > MAX_BDL {
        return false;
    }
    let bdl_address = unsafe { core::ptr::addr_of!(BDL.0) as u64 };
    if bdl_address >= 0x1_0000_0000 || sample_address >= 0x1_0000_0000 {
        return false;
    }
    stop_stream(base + stream);
    unsafe {
        for index in 0..MAX_BDL {
            let offset = index * 128 * 1024;
            BDL.0[index] = if index < entries {
                BdlEntry {
                    address: (sample_address + offset) as u64,
                    length: (128 * 1024).min(bytes - offset) as u32,
                    flags: if index + 1 == entries { 1 } else { 0 },
                }
            } else {
                BdlEntry {
                    address: 0,
                    length: 0,
                    flags: 0,
                }
            };
        }
        core::sync::atomic::fence(Ordering::SeqCst);
        write8(base + stream + STREAM_STATUS, 0x1C);
        write32(base + stream + STREAM_CBL, bytes as u32);
        write16(base + stream + STREAM_LVI, (entries - 1) as u16);
        write16(base + stream + STREAM_FORMAT, FORMAT_48K_STEREO_16);
        write32(base + stream + STREAM_BDL_LOW, bdl_address as u32);
        write32(base + stream + STREAM_BDL_HIGH, 0);
        // stream tag 1 in CTL[23:20], completion/fifo/descriptor IRQs, run.
        write8(base + stream + 2, 0x10);
        write8(base + stream, 0x1E);
    }
    true
}

pub fn stop() {
    let base = IRQ_BASE.load(Ordering::Acquire);
    if base != 0 {
        stop_stream(base + IRQ_STREAM.load(Ordering::Acquire));
    }
    IRQ_EVENTS.store(0, Ordering::Release);
}

pub fn set_volume(volume: u8, muted: bool) {
    let base = IRQ_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    let gain = (volume.min(100) as u16 * 0x7F / 100) & 0x7F;
    let mute = if muted || volume == 0 { 1 << 7 } else { 0 };
    let codec = CODEC.load(Ordering::Acquire);
    let dac = DAC.load(Ordering::Acquire);
    // Output amp, both channels. Controllers/codecs that lack this amp simply
    // ignore the verb; stream safety does not depend on mixer support.
    let _ = verb16(base, codec, dac, 0x3, 0xB000 | mute | gain);
}

pub fn take_events() -> u8 {
    IRQ_EVENTS.swap(0, Ordering::AcqRel)
}

pub fn interrupt_count() -> u64 {
    IRQ_COUNT.load(Ordering::Acquire)
}

pub fn last_status() -> u16 {
    IRQ_LAST_STATUS.load(Ordering::Acquire)
}

pub fn hardware_status() -> u16 {
    let base = IRQ_BASE.load(Ordering::Acquire);
    if base == 0 {
        0
    } else {
        unsafe { read8(base + IRQ_STREAM.load(Ordering::Acquire) + STREAM_STATUS) as u16 }
    }
}

pub fn handle_legacy_interrupt(irq: u8) {
    if IRQ_LINE.load(Ordering::Acquire) != irq {
        return;
    }
    let base = IRQ_BASE.load(Ordering::Acquire);
    let stream = IRQ_STREAM.load(Ordering::Acquire);
    if base == 0 || unsafe { read32(base + INTSTS) } == 0 {
        return;
    }
    let status = unsafe { read8(base + stream + STREAM_STATUS) };
    if status != 0 {
        unsafe { write8(base + stream + STREAM_STATUS, status) };
        IRQ_LAST_STATUS.store(status as u16, Ordering::Release);
        IRQ_EVENTS.fetch_or(status, Ordering::AcqRel);
        IRQ_COUNT.fetch_add(1, Ordering::AcqRel);
    }
}

pub fn handle_message_interrupt() {
    let base = IRQ_BASE.load(Ordering::Acquire);
    let stream = IRQ_STREAM.load(Ordering::Acquire);
    if base == 0 || unsafe { read32(base + INTSTS) } == 0 {
        return;
    }
    let status = unsafe { read8(base + stream + STREAM_STATUS) };
    if status != 0 {
        unsafe { write8(base + stream + STREAM_STATUS, status) };
        IRQ_LAST_STATUS.store(status as u16, Ordering::Release);
        IRQ_EVENTS.fetch_or(status, Ordering::AcqRel);
        IRQ_COUNT.fetch_add(1, Ordering::AcqRel);
    }
}

fn node_range(base: usize, codec: u8, node: u8) -> Option<(u8, u8)> {
    let value = verb(base, codec, node, 0xF00, 0x04)?;
    Some(((value >> 16) as u8, value as u8))
}

fn verb(base: usize, codec: u8, node: u8, command: u16, payload: u8) -> Option<u32> {
    immediate(
        base,
        ((codec as u32) << 28) | ((node as u32) << 20) | ((command as u32) << 8) | payload as u32,
    )
}

fn verb16(base: usize, codec: u8, node: u8, command: u8, payload: u16) -> Option<u32> {
    immediate(
        base,
        ((codec as u32) << 28) | ((node as u32) << 20) | ((command as u32) << 16) | payload as u32,
    )
}

fn immediate(base: usize, command: u32) -> Option<u32> {
    if !wait_until(|| unsafe { read16(base + ICIS) & 1 == 0 }) {
        return None;
    }
    unsafe {
        write16(base + ICIS, 2);
        write32(base + ICOI, command);
        write16(base + ICIS, 1);
    }
    if !wait_until(|| unsafe { read16(base + ICIS) & 1 == 0 })
        || unsafe { read16(base + ICIS) } & 2 == 0
    {
        return None;
    }
    Some(unsafe { read32(base + ICII) })
}

fn stop_stream(stream: usize) {
    unsafe { write8(stream, read8(stream) & !2) };
    let _ = wait_until(|| unsafe { read8(stream) & 2 == 0 });
}

fn wait_until(mut ready: impl FnMut() -> bool) -> bool {
    for _ in 0..MAX_POLLS {
        if ready() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

unsafe fn read8(address: usize) -> u8 {
    unsafe { core::ptr::read_volatile(address as *const u8) }
}
unsafe fn read16(address: usize) -> u16 {
    unsafe { core::ptr::read_volatile(address as *const u16) }
}
unsafe fn read32(address: usize) -> u32 {
    unsafe { core::ptr::read_volatile(address as *const u32) }
}
unsafe fn write8(address: usize, value: u8) {
    unsafe { core::ptr::write_volatile(address as *mut u8, value) }
}
unsafe fn write16(address: usize, value: u16) {
    unsafe { core::ptr::write_volatile(address as *mut u16, value) }
}
unsafe fn write32(address: usize, value: u32) {
    unsafe { core::ptr::write_volatile(address as *mut u32, value) }
}
