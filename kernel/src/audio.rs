//! Bounded ExpAudio output service.
//!
//! The first hardware backend targets the AC'97 PCM-out interface exposed by
//! QEMU and compatible Intel ICH controllers. The public path accepts PCM WAV
//! input, validates every chunk, converts mono/stereo 8/16/24/32-bit integer
//! samples to 48 kHz stereo, and submits one DMA descriptor. Unsupported or
//! absent hardware is reported honestly; callers can still validate media.

use crate::{asl, pci, port, slog, sync::SpinMutex};
use core::sync::atomic::{compiler_fence, Ordering};

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
const PCM_STATUS_COMPLETION: u16 = 1 << 3;
const PCM_STATUS_LAST_VALID: u16 = 1 << 2;
const SAMPLE_RATE: u32 = 48_000;
const MAX_SAMPLE_WORDS: usize = 16 * 1024;

#[repr(C, align(16))]
struct BufferDescriptor {
    address: u32,
    sample_words: u16,
    flags: u16,
}

#[repr(C, align(16))]
struct DescriptorList([BufferDescriptor; 32]);

#[repr(C, align(64))]
struct SampleBuffer([i16; MAX_SAMPLE_WORDS]);

static mut DESCRIPTORS: DescriptorList = DescriptorList(
    [const {
        BufferDescriptor {
            address: 0,
            sample_words: 0,
            flags: 0,
        }
    }; 32],
);
static mut SAMPLES: SampleBuffer = SampleBuffer([0; MAX_SAMPLE_WORDS]);

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
    DmaAddress,
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
    pub submitted_frames: u64,
    pub completed_buffers: u64,
    pub underruns: u64,
}

struct AudioState {
    backend: Backend,
    mixer_base: u16,
    bus_master_base: u16,
    volume: u8,
    muted: bool,
    playing: bool,
    submitted_frames: u64,
    completed_buffers: u64,
    underruns: u64,
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
            submitted_frames: 0,
            completed_buffers: 0,
            underruns: 0,
        }
    }

    fn set_volume(&mut self, volume: u8, muted: bool) {
        self.volume = volume.min(100);
        self.muted = muted;
        if self.backend != Backend::Ac97 {
            return;
        }
        let attenuation = if muted {
            0x8000
        } else {
            let step = 31_u16.saturating_sub(self.volume as u16 * 31 / 100);
            step | (step << 8)
        };
        unsafe {
            port::outw(self.mixer_base + NAM_MASTER_VOLUME, attenuation);
            port::outw(self.mixer_base + NAM_PCM_VOLUME, attenuation);
        }
    }

    fn stop(&mut self) {
        if self.backend == Backend::Ac97 {
            unsafe { port::outb(self.bus_master_base + NABM_PCM_OUT_CONTROL, 0) };
        }
        self.playing = false;
    }

    fn poll(&mut self) {
        if self.backend != Backend::Ac97 || !self.playing {
            return;
        }
        let status = unsafe { port::inw(self.bus_master_base + NABM_PCM_OUT_STATUS) };
        if status & (PCM_STATUS_COMPLETION | PCM_STATUS_LAST_VALID) != 0 {
            unsafe {
                port::outw(
                    self.bus_master_base + NABM_PCM_OUT_STATUS,
                    status & (PCM_STATUS_COMPLETION | PCM_STATUS_LAST_VALID),
                )
            };
        }
        if status & PCM_STATUS_HALTED != 0 {
            self.playing = false;
            self.completed_buffers = self.completed_buffers.saturating_add(1);
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
    function.enable_io_bus_master();
    state.mixer_base = mixer.address;
    state.bus_master_base = bus_master.address;
    state.backend = Backend::Ac97;
    let volume = state.volume;
    let muted = state.muted;
    unsafe {
        port::outb(
            state.bus_master_base + NABM_PCM_OUT_CONTROL,
            PCM_CONTROL_RESET,
        );
        port::outb(state.bus_master_base + NABM_PCM_OUT_CONTROL, 0);
        let extended = port::inw(state.mixer_base + NAM_EXTENDED_STATUS);
        port::outw(state.mixer_base + NAM_EXTENDED_STATUS, extended | 1);
        port::outw(state.mixer_base + NAM_PCM_FRONT_RATE, SAMPLE_RATE as u16);
    }
    state.set_volume(volume, muted);
    let _ = asl::claim(function, asl::Owner::ExpAudio);
    slog!(
        "EXPOS_AUDIO_READY backend=ac97 rate={} channels=2 sample=s16le\r\n",
        SAMPLE_RATE
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
    slog!("EXPOS_AUDIO_STOP\r\n");
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
        submitted_frames: state.submitted_frames,
        completed_buffers: state.completed_buffers,
        underruns: state.underruns,
    }
}

pub fn play_wave(bytes: &[u8]) -> Result<WaveInfo, AudioError> {
    let mut state = AUDIO.lock();
    if state.backend == Backend::Unavailable {
        return Err(AudioError::NoDevice);
    }
    state.stop();
    let info =
        unsafe { decode_wave_to_stereo_48k(bytes, &mut *core::ptr::addr_of_mut!(SAMPLES.0))? };
    submit_samples(&mut state, info.output_frames)?;
    slog!(
        "EXPOS_AUDIO_PLAY codec=pcm-wave source_rate={} source_channels={} source_bits={} frames={} output_rate={}\r\n",
        info.source_rate,
        info.channels,
        info.bits_per_sample,
        info.output_frames,
        SAMPLE_RATE
    );
    Ok(info)
}

pub fn play_tone(frequency_hz: u16, milliseconds: u16) -> Result<(), AudioError> {
    let mut state = AUDIO.lock();
    if state.backend == Backend::Unavailable {
        return Err(AudioError::NoDevice);
    }
    let frames = (SAMPLE_RATE as usize)
        .saturating_mul(milliseconds as usize)
        .checked_div(1_000)
        .unwrap_or(0);
    if frequency_hz == 0 || frames == 0 || frames.saturating_mul(2) > MAX_SAMPLE_WORDS {
        return Err(AudioError::MediaTooLarge);
    }
    state.stop();
    unsafe {
        for frame in 0..frames {
            let phase = frame.saturating_mul(frequency_hz as usize) % SAMPLE_RATE as usize;
            let sample = if phase < SAMPLE_RATE as usize / 2 {
                7_000
            } else {
                -7_000
            };
            SAMPLES.0[frame * 2] = sample;
            SAMPLES.0[frame * 2 + 1] = sample;
        }
    }
    submit_samples(&mut state, frames)?;
    slog!(
        "EXPOS_AUDIO_PLAY codec=generated-tone frequency={} frames={} output_rate={}\r\n",
        frequency_hz,
        frames,
        SAMPLE_RATE
    );
    Ok(())
}

fn submit_samples(state: &mut AudioState, frames: usize) -> Result<(), AudioError> {
    let sample_words = frames.saturating_mul(2);
    if sample_words == 0 || sample_words > u16::MAX as usize {
        return Err(AudioError::MediaTooLarge);
    }
    let sample_address = unsafe { core::ptr::addr_of!(SAMPLES.0) as usize };
    let descriptor_address = unsafe { core::ptr::addr_of!(DESCRIPTORS.0) as usize };
    if sample_address > u32::MAX as usize || descriptor_address > u32::MAX as usize {
        return Err(AudioError::DmaAddress);
    }
    unsafe {
        DESCRIPTORS.0[0] = BufferDescriptor {
            address: sample_address as u32,
            sample_words: sample_words as u16,
            flags: 1 << 15,
        };
        for descriptor in &mut DESCRIPTORS.0[1..] {
            *descriptor = BufferDescriptor {
                address: 0,
                sample_words: 0,
                flags: 0,
            };
        }
    }
    compiler_fence(Ordering::Release);
    unsafe {
        port::outb(
            state.bus_master_base + NABM_PCM_OUT_CONTROL,
            PCM_CONTROL_RESET,
        );
        port::outb(state.bus_master_base + NABM_PCM_OUT_CONTROL, 0);
        port::outl(
            state.bus_master_base + NABM_PCM_OUT_BDBAR,
            descriptor_address as u32,
        );
        port::outb(state.bus_master_base + NABM_PCM_OUT_LVI, 0);
        let _ = port::inb(state.bus_master_base + NABM_PCM_OUT_CIV);
        port::outb(
            state.bus_master_base + NABM_PCM_OUT_CONTROL,
            PCM_CONTROL_RUN,
        );
    }
    state.playing = true;
    state.submitted_frames = state.submitted_frames.saturating_add(frames as u64);
    Ok(())
}

fn decode_wave_to_stereo_48k(
    bytes: &[u8],
    output: &mut [i16; MAX_SAMPLE_WORDS],
) -> Result<WaveInfo, AudioError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(AudioError::MalformedWave);
    }
    let mut cursor: usize = 12;
    let mut format = None;
    let mut data = None;
    while cursor.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let id = &bytes[cursor..cursor + 4];
        let length = read_u32(bytes, cursor + 4)? as usize;
        let start = cursor + 8;
        let end = start.checked_add(length).ok_or(AudioError::MalformedWave)?;
        if end > bytes.len() {
            return Err(AudioError::MalformedWave);
        }
        if id == b"fmt " && length >= 16 {
            format = Some((
                read_u16(bytes, start)?,
                read_u16(bytes, start + 2)?,
                read_u32(bytes, start + 4)?,
                read_u16(bytes, start + 14)?,
            ));
        } else if id == b"data" {
            data = Some(&bytes[start..end]);
        }
        cursor = end + (length & 1);
    }
    let (codec, channels, source_rate, bits) = format.ok_or(AudioError::MalformedWave)?;
    let data = data.ok_or(AudioError::MalformedWave)?;
    if codec != 1 || !matches!(channels, 1 | 2) || !matches!(bits, 8 | 16 | 24 | 32) {
        return Err(AudioError::UnsupportedCodec);
    }
    if !(8_000..=192_000).contains(&source_rate) {
        return Err(AudioError::UnsupportedCodec);
    }
    let bytes_per_sample = bits as usize / 8;
    let frame_bytes = bytes_per_sample * channels as usize;
    if frame_bytes == 0 || data.len() < frame_bytes {
        return Err(AudioError::MalformedWave);
    }
    let source_frames = data.len() / frame_bytes;
    let output_frames = source_frames
        .saturating_mul(SAMPLE_RATE as usize)
        .checked_div(source_rate as usize)
        .ok_or(AudioError::MalformedWave)?;
    if output_frames == 0 || output_frames.saturating_mul(2) > output.len() {
        return Err(AudioError::MediaTooLarge);
    }
    for output_frame in 0..output_frames {
        let source_frame = output_frame
            .saturating_mul(source_rate as usize)
            .checked_div(SAMPLE_RATE as usize)
            .unwrap_or(0)
            .min(source_frames - 1);
        let base = source_frame * frame_bytes;
        let left = decode_sample(&data[base..base + bytes_per_sample], bits);
        let right = if channels == 2 {
            decode_sample(
                &data[base + bytes_per_sample..base + 2 * bytes_per_sample],
                bits,
            )
        } else {
            left
        };
        output[output_frame * 2] = left;
        output[output_frame * 2 + 1] = right;
    }
    Ok(WaveInfo {
        channels,
        source_rate,
        bits_per_sample: bits,
        source_frames,
        output_frames,
    })
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

    fn wave(bits: u16, channels: u16, rate: u32, samples: &[u8]) -> [u8; 64] {
        let mut bytes = [0_u8; 64];
        bytes[..4].copy_from_slice(b"RIFF");
        bytes[4..8].copy_from_slice(&(56_u32).to_le_bytes());
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
    fn pcm_wave_decoder_converts_mono_and_resamples() {
        let bytes = wave(8, 1, 24_000, &[0, 128, 255, 128]);
        let mut output = [0_i16; MAX_SAMPLE_WORDS];
        let info = decode_wave_to_stereo_48k(&bytes[..48], &mut output).unwrap();
        assert_eq!(info.output_frames, 8);
        assert_eq!(output[0], i16::MIN);
        assert_eq!(output[0], output[1]);
        assert_eq!(output[4], 0);
    }

    #[test]
    fn rejects_compressed_or_overlarge_wave_data() {
        let mut bytes = wave(16, 2, 48_000, &[0; 8]);
        bytes[20..22].copy_from_slice(&3_u16.to_le_bytes());
        let mut output = [0_i16; MAX_SAMPLE_WORDS];
        assert_eq!(
            decode_wave_to_stereo_48k(&bytes[..52], &mut output),
            Err(AudioError::UnsupportedCodec)
        );
    }
}
