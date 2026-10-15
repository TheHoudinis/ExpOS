//! Bounded ExpAudio output service.
//!
//! The hardware backend targets the AC'97 PCM-out interface exposed by QEMU
//! and compatible Intel ICH controllers. The media layer strictly validates
//! integer PCM RIFF/WAVE input, converts mono/stereo 8/16/24/32-bit samples to
//! 48 kHz stereo with linear interpolation, and feeds a bounded AC'97 bus-master
//! descriptor ring. Two DMA-safe sample banks provide one active and one queued
//! request without mutating memory still owned by the device.

use crate::{asl, pci, port, slog, sync::SpinMutex};
use core::sync::atomic::{fence, Ordering};

const AC97_VENDOR: u16 = 0x8086;
const AC97_DEVICE: u16 = 0x2415;
const NAM_MASTER_VOLUME: u16 = 0x02;
const NAM_PCM_VOLUME: u16 = 0x18;
const NAM_EXTENDED_STATUS: u16 = 0x2A;
const NAM_PCM_FRONT_RATE: u16 = 0x2C;
const NABM_PCM_OUT_BDBAR: u16 = 0x10;
const NABM_PCM_OUT_CIV: u16 = 0x14;
const NABM_PCM_OUT_LVI: u16 = 0x15;
const NABM_PCM_OUT_STATUS: u16 = 0x16;
const NABM_PCM_OUT_CONTROL: u16 = 0x1B;
const PCM_CONTROL_RUN: u8 = 1;
const PCM_CONTROL_RESET: u8 = 1 << 1;
const PCM_STATUS_HALTED: u16 = 1;
const PCM_STATUS_LAST_VALID: u16 = 1 << 2;
const PCM_STATUS_COMPLETION: u16 = 1 << 3;
const PCM_STATUS_FIFO_ERROR: u16 = 1 << 4;
const PCM_STATUS_CLEAR: u16 = PCM_STATUS_LAST_VALID | PCM_STATUS_COMPLETION | PCM_STATUS_FIFO_ERROR;
const DESCRIPTOR_INTERRUPT_ON_COMPLETION: u16 = 1 << 15;
const SAMPLE_RATE: u32 = 48_000;
const DESCRIPTOR_COUNT: usize = 32;
const WORDS_PER_DESCRIPTOR: usize = 32 * 1024;
const MAX_SAMPLE_WORDS: usize = DESCRIPTOR_COUNT * WORDS_PER_DESCRIPTOR;
const SAMPLE_BANK_COUNT: usize = 2;
const RESET_POLLS: usize = 10_000;

#[repr(C, align(16))]
struct BufferDescriptor {
    address: u32,
    sample_words: u16,
    flags: u16,
}

#[repr(C, align(16))]
struct DescriptorList([BufferDescriptor; DESCRIPTOR_COUNT]);

#[repr(C, align(64))]
struct SampleBanks([[i16; MAX_SAMPLE_WORDS]; SAMPLE_BANK_COUNT]);

static mut DESCRIPTORS: DescriptorList = DescriptorList(
    [const {
        BufferDescriptor {
            address: 0,
            sample_words: 0,
            flags: 0,
        }
    }; DESCRIPTOR_COUNT],
);
static mut SAMPLES: SampleBanks = SampleBanks([[0; MAX_SAMPLE_WORDS]; SAMPLE_BANK_COUNT]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Unavailable,
    Ac97,
}

impl Backend {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "Unavailable",
            Self::Ac97 => "AC'97 PCM",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    NoDevice,
    MalformedWave,
    UnsupportedCodec,
    MediaTooLarge,
    QueueFull,
    DmaAddress,
    ControllerTimeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaveInfo {
    pub channels: u16,
    pub source_rate: u32,
    pub bits_per_sample: u16,
    pub source_frames: usize,
    pub output_frames: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub backend: Backend,
    pub volume: u8,
    pub muted: bool,
    pub playing: bool,
    pub queued: bool,
    pub submitted_frames: u64,
    pub completed_buffers: u64,
    pub queued_buffers: u64,
    pub underruns: u64,
    pub fifo_errors: u64,
    pub recoveries: u64,
}

#[derive(Clone, Copy)]
struct QueuedBuffer {
    bank: u8,
    frames: usize,
}

struct AudioState {
    backend: Backend,
    mixer_base: u16,
    bus_master_base: u16,
    volume: u8,
    muted: bool,
    playing: bool,
    active_bank: u8,
    active_descriptors: u8,
    pending: Option<QueuedBuffer>,
    submitted_frames: u64,
    completed_buffers: u64,
    queued_buffers: u64,
    underruns: u64,
    fifo_errors: u64,
    recoveries: u64,
}

impl AudioState {
    const fn new() -> Self {
        Self {
            backend: Backend::Unavailable,
            mixer_base: 0,
            bus_master_base: 0,
            volume: 80,
            muted: false,
            playing: false,
            active_bank: 0,
            active_descriptors: 0,
            pending: None,
            submitted_frames: 0,
            completed_buffers: 0,
            queued_buffers: 0,
            underruns: 0,
            fifo_errors: 0,
            recoveries: 0,
        }
    }

    fn set_volume(&mut self, volume: u8, muted: bool) {
        self.volume = volume.min(100);
        self.muted = muted;
        if self.backend != Backend::Ac97 {
            return;
        }
        // Avoid applying the same attenuation twice. Master provides the hard
        // mute while PCM provides the requested level.
        let master = if muted || self.volume == 0 { 0x8000 } else { 0 };
        let step = 31_u16.saturating_sub(self.volume as u16 * 31 / 100);
        let pcm = step | (step << 8);
        unsafe {
            port::outw(self.mixer_base + NAM_MASTER_VOLUME, master);
            port::outw(self.mixer_base + NAM_PCM_VOLUME, pcm);
        }
    }

    fn stop(&mut self) {
        if self.backend == Backend::Ac97 {
            unsafe {
                port::outb(self.bus_master_base + NABM_PCM_OUT_CONTROL, 0);
                port::outw(self.bus_master_base + NABM_PCM_OUT_STATUS, PCM_STATUS_CLEAR);
            }
        }
        self.playing = false;
        self.active_descriptors = 0;
        self.pending = None;
    }

    fn poll(&mut self) {
        if self.backend != Backend::Ac97 || !self.playing {
            return;
        }
        let status = unsafe { port::inw(self.bus_master_base + NABM_PCM_OUT_STATUS) };
        let clear = status & PCM_STATUS_CLEAR;
        if clear != 0 {
            unsafe { port::outw(self.bus_master_base + NABM_PCM_OUT_STATUS, clear) };
        }
        let fifo_error = status & PCM_STATUS_FIFO_ERROR != 0;
        if fifo_error {
            self.fifo_errors = self.fifo_errors.saturating_add(1);
            self.underruns = self.underruns.saturating_add(1);
            slog!("EXPOS_AUDIO_FAULT kind=fifo-error action=recover\r\n");
        }
        if status & PCM_STATUS_HALTED == 0 {
            return;
        }
        self.playing = false;
        self.active_descriptors = 0;
        let completed = status & (PCM_STATUS_COMPLETION | PCM_STATUS_LAST_VALID) != 0;
        let faulted = fifo_error || !completed;
        if completed {
            self.completed_buffers = self.completed_buffers.saturating_add(1);
        } else if !fifo_error {
            self.underruns = self.underruns.saturating_add(1);
            slog!("EXPOS_AUDIO_FAULT kind=unexpected-halt action=recover\r\n");
        }
        if let Some(queued) = self.pending.take() {
            match submit_samples(self, queued.bank as usize, queued.frames) {
                Ok(()) => {
                    if faulted {
                        self.recoveries = self.recoveries.saturating_add(1);
                    }
                    slog!(
                        "EXPOS_AUDIO_DEQUEUE bank={} frames={}\r\n",
                        queued.bank,
                        queued.frames
                    );
                }
                Err(error) => slog!("EXPOS_AUDIO_RECOVERY_FAILED error={:?}\r\n", error),
            }
        }
    }
}

static AUDIO: SpinMutex<AudioState> = SpinMutex::new(AudioState::new());

pub fn initialize() -> Status {
    let mut state = AUDIO.lock();
    if state.backend != Backend::Unavailable {
        return status_locked(&mut state);
    }
    let Some(function) = pci::find(|function| {
        (function.vendor_id == AC97_VENDOR && function.device_id == AC97_DEVICE)
            || (function.class_code == 0x04 && function.subclass == 0x01)
    }) else {
        slog!("EXPOS_AUDIO_UNAVAILABLE reason=no-supported-pci-device\r\n");
        return status_locked(&mut state);
    };
    let Some(mixer) = function.io_bar(0) else {
        slog!("EXPOS_AUDIO_UNAVAILABLE reason=no-mixer-io-bar\r\n");
        return status_locked(&mut state);
    };
    let Some(bus_master) = function.io_bar(1) else {
        slog!("EXPOS_AUDIO_UNAVAILABLE reason=no-bus-master-io-bar\r\n");
        return status_locked(&mut state);
    };
    if !asl::claim(function, asl::Owner::ExpAudio) {
        slog!("EXPOS_AUDIO_UNAVAILABLE reason=asl-claim-denied\r\n");
        return status_locked(&mut state);
    }
    let original_command = function.read(0x04);
    function.enable_io_bus_master();
    if !reset_dma(bus_master.address) {
        function.write(0x04, original_command);
        let _ = asl::release(function, asl::Owner::ExpAudio);
        slog!("EXPOS_AUDIO_UNAVAILABLE reason=controller-reset-timeout\r\n");
        return status_locked(&mut state);
    }
    state.mixer_base = mixer.address;
    state.bus_master_base = bus_master.address;
    state.backend = Backend::Ac97;
    let volume = state.volume;
    let muted = state.muted;
    unsafe {
        let extended = port::inw(state.mixer_base + NAM_EXTENDED_STATUS);
        port::outw(state.mixer_base + NAM_EXTENDED_STATUS, extended | 1);
        port::outw(state.mixer_base + NAM_PCM_FRONT_RATE, SAMPLE_RATE as u16);
    }
    state.set_volume(volume, muted);
    slog!(
        "EXPOS_AUDIO_READY backend=ac97 rate={} channels=2 sample=s16le descriptors={} queue=1\r\n",
        SAMPLE_RATE,
        DESCRIPTOR_COUNT
    );
    status_locked(&mut state)
}

pub fn available() -> bool {
    AUDIO.lock().backend != Backend::Unavailable
}

pub fn set_volume(volume: u8) {
    let mut state = AUDIO.lock();
    let muted = state.muted;
    state.set_volume(volume, muted);
}

pub fn set_muted(muted: bool) {
    let mut state = AUDIO.lock();
    let volume = state.volume;
    state.set_volume(volume, muted);
}

pub fn stop() {
    AUDIO.lock().stop();
    slog!("EXPOS_AUDIO_STOP queue=cleared\r\n");
}

pub fn poll() -> Status {
    let mut state = AUDIO.lock();
    status_locked(&mut state)
}

pub fn status() -> Status {
    poll()
}

fn status_locked(state: &mut AudioState) -> Status {
    state.poll();
    Status {
        backend: state.backend,
        volume: state.volume,
        muted: state.muted,
        playing: state.playing,
        queued: state.pending.is_some(),
        submitted_frames: state.submitted_frames,
        completed_buffers: state.completed_buffers,
        queued_buffers: state.queued_buffers,
        underruns: state.underruns,
        fifo_errors: state.fifo_errors,
        recoveries: state.recoveries,
    }
}

pub fn play_wave(bytes: &[u8]) -> Result<WaveInfo, AudioError> {
    let mut state = AUDIO.lock();
    if state.backend == Backend::Unavailable {
        return Err(AudioError::NoDevice);
    }
    state.poll();
    let bank = available_bank(&state)?;
    let output = unsafe { sample_bank_mut(bank) };
    let info = decode_wave_to_stereo_48k(bytes, output)?;
    schedule_samples(&mut state, bank, info.output_frames)?;
    slog!(
        "EXPOS_AUDIO_PLAY codec=pcm-wave source_rate={} source_channels={} source_bits={} frames={} output_rate={} bank={}\r\n",
        info.source_rate,
        info.channels,
        info.bits_per_sample,
        info.output_frames,
        SAMPLE_RATE,
        bank
    );
    Ok(info)
}

pub fn play_tone(frequency_hz: u16, milliseconds: u16) -> Result<(), AudioError> {
    let mut state = AUDIO.lock();
    if state.backend == Backend::Unavailable {
        return Err(AudioError::NoDevice);
    }
    state.poll();
    let frames = (SAMPLE_RATE as usize)
        .checked_mul(milliseconds as usize)
        .and_then(|samples| samples.checked_div(1_000))
        .ok_or(AudioError::MediaTooLarge)?;
    if frequency_hz == 0
        || frames == 0
        || frames
            .checked_mul(2)
            .is_none_or(|words| words > MAX_SAMPLE_WORDS)
    {
        return Err(AudioError::MediaTooLarge);
    }
    let bank = available_bank(&state)?;
    let output = unsafe { sample_bank_mut(bank) };
    for frame in 0..frames {
        let phase = frame.saturating_mul(frequency_hz as usize) % SAMPLE_RATE as usize;
        let sample = if phase < SAMPLE_RATE as usize / 2 {
            7_000
        } else {
            -7_000
        };
        output[frame * 2] = sample;
        output[frame * 2 + 1] = sample;
    }
    schedule_samples(&mut state, bank, frames)?;
    slog!(
        "EXPOS_AUDIO_PLAY codec=generated-tone frequency={} frames={} output_rate={} bank={}\r\n",
        frequency_hz,
        frames,
        SAMPLE_RATE,
        bank
    );
    Ok(())
}

fn available_bank(state: &AudioState) -> Result<usize, AudioError> {
    if state.playing && state.pending.is_some() {
        return Err(AudioError::QueueFull);
    }
    Ok((state.active_bank as usize + 1) % SAMPLE_BANK_COUNT)
}

fn schedule_samples(state: &mut AudioState, bank: usize, frames: usize) -> Result<(), AudioError> {
    if state.playing {
        state.pending = Some(QueuedBuffer {
            bank: bank as u8,
            frames,
        });
        state.queued_buffers = state.queued_buffers.saturating_add(1);
        slog!(
            "EXPOS_AUDIO_QUEUE bank={} frames={} depth=1\r\n",
            bank,
            frames
        );
        Ok(())
    } else {
        submit_samples(state, bank, frames)
    }
}

fn submit_samples(state: &mut AudioState, bank: usize, frames: usize) -> Result<(), AudioError> {
    let sample_words = frames.checked_mul(2).ok_or(AudioError::MediaTooLarge)?;
    let descriptor_count = descriptor_count(sample_words)?;
    let sample_address = unsafe { sample_bank_address(bank) };
    let descriptor_address = unsafe { core::ptr::addr_of!(DESCRIPTORS.0) as usize };
    let sample_bytes = sample_words
        .checked_mul(core::mem::size_of::<i16>())
        .ok_or(AudioError::DmaAddress)?;
    if bank >= SAMPLE_BANK_COUNT
        || sample_address > u32::MAX as usize
        || sample_address
            .checked_add(sample_bytes.saturating_sub(1))
            .is_none_or(|end| end > u32::MAX as usize)
        || descriptor_address > u32::MAX as usize
    {
        return Err(AudioError::DmaAddress);
    }
    unsafe {
        for index in 0..DESCRIPTOR_COUNT {
            let offset_words = index * WORDS_PER_DESCRIPTOR;
            if index < descriptor_count {
                let words = WORDS_PER_DESCRIPTOR.min(sample_words - offset_words);
                DESCRIPTORS.0[index] = BufferDescriptor {
                    address: (sample_address + offset_words * core::mem::size_of::<i16>()) as u32,
                    sample_words: words as u16,
                    flags: if index + 1 == descriptor_count {
                        DESCRIPTOR_INTERRUPT_ON_COMPLETION
                    } else {
                        0
                    },
                };
            } else {
                DESCRIPTORS.0[index] = BufferDescriptor {
                    address: 0,
                    sample_words: 0,
                    flags: 0,
                };
            }
        }
    }
    fence(Ordering::SeqCst);
    if !reset_dma(state.bus_master_base) {
        return Err(AudioError::ControllerTimeout);
    }
    unsafe {
        port::outw(
            state.bus_master_base + NABM_PCM_OUT_STATUS,
            PCM_STATUS_CLEAR,
        );
        port::outl(
            state.bus_master_base + NABM_PCM_OUT_BDBAR,
            descriptor_address as u32,
        );
        port::outb(
            state.bus_master_base + NABM_PCM_OUT_LVI,
            (descriptor_count - 1) as u8,
        );
        let _ = port::inb(state.bus_master_base + NABM_PCM_OUT_CIV);
        port::outb(
            state.bus_master_base + NABM_PCM_OUT_CONTROL,
            PCM_CONTROL_RUN,
        );
    }
    state.active_bank = bank as u8;
    state.active_descriptors = descriptor_count as u8;
    state.playing = true;
    state.submitted_frames = state.submitted_frames.saturating_add(frames as u64);
    Ok(())
}

fn descriptor_count(sample_words: usize) -> Result<usize, AudioError> {
    if sample_words == 0 || sample_words > MAX_SAMPLE_WORDS {
        return Err(AudioError::MediaTooLarge);
    }
    let count = sample_words.div_ceil(WORDS_PER_DESCRIPTOR);
    if count == 0 || count > DESCRIPTOR_COUNT {
        return Err(AudioError::MediaTooLarge);
    }
    Ok(count)
}

fn reset_dma(base: u16) -> bool {
    unsafe { port::outb(base + NABM_PCM_OUT_CONTROL, PCM_CONTROL_RESET) };
    for _ in 0..RESET_POLLS {
        if unsafe { port::inb(base + NABM_PCM_OUT_CONTROL) } & PCM_CONTROL_RESET == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    unsafe { port::outb(base + NABM_PCM_OUT_CONTROL, 0) };
    false
}

unsafe fn sample_bank_mut(bank: usize) -> &'static mut [i16] {
    let pointer = unsafe {
        core::ptr::addr_of_mut!(SAMPLES.0)
            .cast::<i16>()
            .add(bank * MAX_SAMPLE_WORDS)
    };
    unsafe { core::slice::from_raw_parts_mut(pointer, MAX_SAMPLE_WORDS) }
}

unsafe fn sample_bank_address(bank: usize) -> usize {
    unsafe {
        core::ptr::addr_of!(SAMPLES.0)
            .cast::<i16>()
            .add(bank * MAX_SAMPLE_WORDS) as usize
    }
}

fn decode_wave_to_stereo_48k(bytes: &[u8], output: &mut [i16]) -> Result<WaveInfo, AudioError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(AudioError::MalformedWave);
    }
    let declared_end = 8_usize
        .checked_add(read_u32(bytes, 4)? as usize)
        .filter(|end| (12..=bytes.len()).contains(end))
        .ok_or(AudioError::MalformedWave)?;
    let bytes = &bytes[..declared_end];
    let mut cursor = 12_usize;
    let mut format = None;
    let mut data = None;
    while cursor < bytes.len() {
        let header_end = cursor.checked_add(8).ok_or(AudioError::MalformedWave)?;
        if header_end > bytes.len() {
            return Err(AudioError::MalformedWave);
        }
        let id = &bytes[cursor..cursor + 4];
        let length = read_u32(bytes, cursor + 4)? as usize;
        let start = header_end;
        let end = start.checked_add(length).ok_or(AudioError::MalformedWave)?;
        let padded_end = end
            .checked_add(length & 1)
            .filter(|end| *end <= bytes.len())
            .ok_or(AudioError::MalformedWave)?;
        if id == b"fmt " {
            if format.is_some() || length < 16 {
                return Err(AudioError::MalformedWave);
            }
            format = Some((
                read_u16(bytes, start)?,
                read_u16(bytes, start + 2)?,
                read_u32(bytes, start + 4)?,
                read_u32(bytes, start + 8)?,
                read_u16(bytes, start + 12)?,
                read_u16(bytes, start + 14)?,
            ));
        } else if id == b"data" {
            if data.is_some() {
                return Err(AudioError::MalformedWave);
            }
            data = Some(&bytes[start..end]);
        }
        cursor = padded_end;
    }
    let (codec, channels, source_rate, byte_rate, block_align, bits) =
        format.ok_or(AudioError::MalformedWave)?;
    let data = data.ok_or(AudioError::MalformedWave)?;
    if codec != 1 || !matches!(channels, 1 | 2) || !matches!(bits, 8 | 16 | 24 | 32) {
        return Err(AudioError::UnsupportedCodec);
    }
    if !(8_000..=192_000).contains(&source_rate) {
        return Err(AudioError::UnsupportedCodec);
    }
    let bytes_per_sample = bits as usize / 8;
    let frame_bytes = bytes_per_sample
        .checked_mul(channels as usize)
        .ok_or(AudioError::MalformedWave)?;
    let expected_byte_rate = source_rate
        .checked_mul(frame_bytes as u32)
        .ok_or(AudioError::MalformedWave)?;
    if frame_bytes == 0
        || block_align as usize != frame_bytes
        || byte_rate != expected_byte_rate
        || data.is_empty()
        || !data.len().is_multiple_of(frame_bytes)
    {
        return Err(AudioError::MalformedWave);
    }
    let source_frames = data.len() / frame_bytes;
    let output_frames = source_frames
        .checked_mul(SAMPLE_RATE as usize)
        .and_then(|frames| frames.checked_add(source_rate as usize - 1))
        .and_then(|frames| frames.checked_div(source_rate as usize))
        .ok_or(AudioError::MediaTooLarge)?;
    if output_frames == 0
        || output_frames
            .checked_mul(2)
            .is_none_or(|words| words > output.len())
    {
        return Err(AudioError::MediaTooLarge);
    }
    for output_frame in 0..output_frames {
        let position = output_frame
            .checked_mul(source_rate as usize)
            .ok_or(AudioError::MediaTooLarge)?;
        let source_frame = (position / SAMPLE_RATE as usize).min(source_frames - 1);
        let next_frame = (source_frame + 1).min(source_frames - 1);
        let fraction = position % SAMPLE_RATE as usize;
        let (left, right) = decode_frame(data, source_frame, channels, bits, frame_bytes);
        let (next_left, next_right) = decode_frame(data, next_frame, channels, bits, frame_bytes);
        output[output_frame * 2] = interpolate(left, next_left, fraction);
        output[output_frame * 2 + 1] = interpolate(right, next_right, fraction);
    }
    Ok(WaveInfo {
        channels,
        source_rate,
        bits_per_sample: bits,
        source_frames,
        output_frames,
    })
}

fn decode_frame(
    data: &[u8],
    frame: usize,
    channels: u16,
    bits: u16,
    frame_bytes: usize,
) -> (i16, i16) {
    let bytes_per_sample = bits as usize / 8;
    let base = frame * frame_bytes;
    let left = decode_sample(&data[base..base + bytes_per_sample], bits);
    let right = if channels == 2 {
        decode_sample(
            &data[base + bytes_per_sample..base + 2 * bytes_per_sample],
            bits,
        )
    } else {
        left
    };
    (left, right)
}

fn interpolate(first: i16, second: i16, fraction: usize) -> i16 {
    let first = first as i64;
    let delta = second as i64 - first;
    (first + delta * fraction as i64 / SAMPLE_RATE as i64) as i16
}

fn decode_sample(bytes: &[u8], bits: u16) -> i16 {
    match bits {
        8 => ((bytes[0] as i16) - 128) << 8,
        16 => i16::from_le_bytes([bytes[0], bytes[1]]),
        24 => {
            let raw = (bytes[0] as i32) | ((bytes[1] as i32) << 8) | ((bytes[2] as i32) << 16);
            let signed = if raw & 0x80_0000 != 0 {
                raw | !0xFF_FFFF
            } else {
                raw
            };
            (signed >> 8) as i16
        }
        32 => (i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) >> 16) as i16,
        _ => 0,
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, AudioError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(AudioError::MalformedWave)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, AudioError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(AudioError::MalformedWave)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn wave(bits: u16, channels: u16, rate: u32, samples: &[u8]) -> Vec<u8> {
        let padded = samples.len() + (samples.len() & 1);
        let mut bytes = alloc::vec![0_u8; 44 + padded];
        let riff_size = (bytes.len() - 8) as u32;
        bytes[..4].copy_from_slice(b"RIFF");
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        bytes[8..12].copy_from_slice(b"WAVE");
        bytes[12..16].copy_from_slice(b"fmt ");
        bytes[16..20].copy_from_slice(&16_u32.to_le_bytes());
        bytes[20..22].copy_from_slice(&1_u16.to_le_bytes());
        bytes[22..24].copy_from_slice(&channels.to_le_bytes());
        bytes[24..28].copy_from_slice(&rate.to_le_bytes());
        let block = channels * (bits / 8);
        bytes[28..32].copy_from_slice(&(rate * block as u32).to_le_bytes());
        bytes[32..34].copy_from_slice(&block.to_le_bytes());
        bytes[34..36].copy_from_slice(&bits.to_le_bytes());
        bytes[36..40].copy_from_slice(b"data");
        bytes[40..44].copy_from_slice(&(samples.len() as u32).to_le_bytes());
        bytes[44..44 + samples.len()].copy_from_slice(samples);
        bytes
    }

    #[test]
    #[ignore = "deterministic mutation campaign; run make parser-fuzz"]
    fn parser_fuzz_wave() {
        let mono = wave(8, 1, 24_000, &[0, 128, 255, 128]);
        let stereo = wave(16, 2, 48_000, &[0; 8]);
        let mut output = [0_i16; 512];
        crate::parser_fuzz::campaign("wave", &[&mono, &stereo], |data| {
            if let Ok(info) = decode_wave_to_stereo_48k(data, &mut output) {
                assert!(info.output_frames > 0 && info.output_frames * 2 <= output.len());
                assert!(matches!(info.channels, 1 | 2));
            }
        });
    }

    #[test]
    fn pcm_wave_decoder_interpolates_mono_and_resamples() {
        let bytes = wave(8, 1, 24_000, &[0, 128, 255, 128]);
        let mut output = [0_i16; 32];
        let info = decode_wave_to_stereo_48k(&bytes, &mut output).unwrap();
        assert_eq!(info.output_frames, 8);
        assert_eq!(output[0], i16::MIN);
        assert_eq!(output[0], output[1]);
        assert_eq!(output[2], -16_384);
        assert_eq!(output[4], 0);
    }

    #[test]
    fn pcm_wave_decoder_preserves_stereo_channels() {
        let bytes = wave(16, 2, 48_000, &[0x34, 0x12, 0xCC, 0xED]);
        let mut output = [0_i16; 8];
        let info = decode_wave_to_stereo_48k(&bytes, &mut output).unwrap();
        assert_eq!(info.output_frames, 1);
        assert_eq!(output[..2], [0x1234, -0x1234]);
    }

    #[test]
    fn rejects_compressed_inconsistent_or_partial_wave_data() {
        let mut compressed = wave(16, 2, 48_000, &[0; 8]);
        compressed[20..22].copy_from_slice(&3_u16.to_le_bytes());
        let mut output = [0_i16; 64];
        assert_eq!(
            decode_wave_to_stereo_48k(&compressed, &mut output),
            Err(AudioError::UnsupportedCodec)
        );
        let mut wrong_alignment = wave(16, 2, 48_000, &[0; 8]);
        wrong_alignment[32..34].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            decode_wave_to_stereo_48k(&wrong_alignment, &mut output),
            Err(AudioError::MalformedWave)
        );
        let partial = wave(16, 2, 48_000, &[0; 5]);
        assert_eq!(
            decode_wave_to_stereo_48k(&partial, &mut output),
            Err(AudioError::MalformedWave)
        );
    }

    #[test]
    fn rejects_truncated_riff_and_duplicate_data_chunks() {
        let bytes = wave(8, 1, 8_000, &[0, 1, 2]);
        let mut output = [0_i16; 64];
        assert_eq!(
            decode_wave_to_stereo_48k(&bytes[..bytes.len() - 1], &mut output),
            Err(AudioError::MalformedWave)
        );
        let mut duplicate = wave(8, 1, 48_000, &[128]);
        duplicate.extend_from_slice(b"data\x01\0\0\0\x80\0");
        let declared = (duplicate.len() - 8) as u32;
        duplicate[4..8].copy_from_slice(&declared.to_le_bytes());
        assert_eq!(
            decode_wave_to_stereo_48k(&duplicate, &mut output),
            Err(AudioError::MalformedWave)
        );
    }

    #[test]
    fn descriptor_ring_splits_large_streams_without_u16_truncation() {
        assert_eq!(descriptor_count(1), Ok(1));
        assert_eq!(descriptor_count(WORDS_PER_DESCRIPTOR), Ok(1));
        assert_eq!(descriptor_count(WORDS_PER_DESCRIPTOR + 2), Ok(2));
        assert_eq!(descriptor_count(MAX_SAMPLE_WORDS), Ok(DESCRIPTOR_COUNT));
        assert_eq!(
            descriptor_count(MAX_SAMPLE_WORDS + 1),
            Err(AudioError::MediaTooLarge)
        );
    }
}
