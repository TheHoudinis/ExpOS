//! Bounded streaming MPEG Layer III decoder for installed Ayo media.
//!
//! ExpAudio owns the hardware queue; this module only turns a small number of
//! compressed frames into the next DMA buffer. Songs are loaded from the
//! verified VM media store and are never compiled into the kernel image.

use super::AudioError;
use alloc::boxed::Box;

const OUTPUT_RATE: u32 = 48_000;
const FRAMES_PER_CHUNK: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Track {
    Imtq,
    Dash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrackInfo {
    pub track: Track,
    pub command_name: &'static str,
    pub display_name: &'static str,
    pub duration_seconds: u16,
    pub compressed_bytes: usize,
    pub sha256: &'static str,
}

impl Track {
    pub const ALL: [Self; 2] = [Self::Imtq, Self::Dash];

    pub const fn info(self) -> TrackInfo {
        match self {
            Self::Imtq => TrackInfo {
                track: self,
                command_name: "imtq",
                display_name: "IMTQ",
                duration_seconds: 243,
                compressed_bytes: 7_585_316,
                sha256: "378ebe91fe508505353894c12a69a0a461cd257a101b3a7ff8e890b925d3f185",
            },
            Self::Dash => TrackInfo {
                track: self,
                command_name: "dash",
                display_name: "-_-.mp3",
                duration_seconds: 73,
                compressed_bytes: 2_256_476,
                sha256: "fc71e866c57fca5126fab0e242538f531b121f6d13354a8858b2b6b0721c72f6",
            },
        }
    }

    pub const fn id(self) -> u8 {
        match self {
            Self::Imtq => 0,
            Self::Dash => 1,
        }
    }

    pub const fn catalog_index(self) -> usize {
        self.id() as usize
    }
}

pub struct Stream {
    decoder: Box<mbop3::Decoder>,
    track: Track,
    bytes: Box<[u8]>,
    offset: usize,
    source_rate: u32,
    source_channels: u8,
    decoded_frames: u64,
    compressed_frames: u64,
    rate_accumulator: u32,
    finished: bool,
}

impl Stream {
    pub fn new(track: Track, bytes: Box<[u8]>) -> Self {
        Self {
            decoder: mbop3::Decoder::new_boxed(),
            track,
            bytes,
            offset: 0,
            source_rate: 0,
            source_channels: 0,
            decoded_frames: 0,
            compressed_frames: 0,
            rate_accumulator: 0,
            finished: false,
        }
    }

    pub const fn track(&self) -> Track {
        self.track
    }

    pub const fn offset(&self) -> usize {
        self.offset
    }

    pub const fn source_rate(&self) -> u32 {
        self.source_rate
    }

    pub const fn source_channels(&self) -> u8 {
        self.source_channels
    }

    pub const fn decoded_frames(&self) -> u64 {
        self.decoded_frames
    }

    pub const fn finished(&self) -> bool {
        self.finished
    }

    pub fn decode_chunk(&mut self, output: &mut [i16]) -> Result<usize, AudioError> {
        if self.finished {
            return Ok(0);
        }
        let bytes = &self.bytes;
        let mut pcm = [0_i16; mbop3::MAX_SAMPLES_PER_FRAME];
        let mut output_frames = 0_usize;
        let mut decoded_this_chunk = 0_usize;

        while self.offset < bytes.len() && decoded_this_chunk < FRAMES_PER_CHUNK {
            let (samples_per_channel, info) = self
                .decoder
                .decode_frame(&bytes[self.offset..], Some(&mut pcm));
            if info.frame_bytes <= 0 {
                self.finished = true;
                break;
            }
            self.offset = self
                .offset
                .checked_add(info.frame_bytes as usize)
                .ok_or(AudioError::MalformedMp3)?;
            if self.offset > bytes.len()
                || !matches!(info.channels, 1 | 2)
                || !(8_000..=48_000).contains(&info.hz)
                || info.layer != 3
            {
                return Err(AudioError::UnsupportedCodec);
            }
            if self.source_rate == 0 {
                self.source_rate = info.hz as u32;
                self.source_channels = info.channels as u8;
            } else if self.source_rate != info.hz as u32
                || self.source_channels != info.channels as u8
            {
                return Err(AudioError::UnsupportedCodec);
            }
            self.compressed_frames = self.compressed_frames.saturating_add(1);
            decoded_this_chunk += 1;
            if samples_per_channel == 0 {
                continue;
            }
            let channels = info.channels as usize;
            for source_frame in 0..samples_per_channel {
                let left = pcm[source_frame * channels];
                let right = if channels == 2 {
                    pcm[source_frame * channels + 1]
                } else {
                    left
                };
                self.rate_accumulator = self.rate_accumulator.saturating_add(OUTPUT_RATE);
                while self.rate_accumulator >= self.source_rate {
                    if output_frames
                        .checked_mul(2)
                        .and_then(|index| index.checked_add(1))
                        .is_none_or(|last| last >= output.len())
                    {
                        return Err(AudioError::MediaTooLarge);
                    }
                    output[output_frames * 2] = left;
                    output[output_frames * 2 + 1] = right;
                    output_frames += 1;
                    self.rate_accumulator -= self.source_rate;
                }
            }
        }
        if self.offset >= bytes.len() {
            self.finished = true;
        }
        self.decoded_frames = self.decoded_frames.saturating_add(output_frames as u64);
        Ok(output_frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_rejects_an_empty_installed_package_without_panicking() {
        let mut stream = Stream::new(Track::Imtq, Box::new([]));
        let mut output = [0_i16; 48_000 * 2];
        assert_eq!(stream.decode_chunk(&mut output), Ok(0));
        assert!(stream.finished());
    }

    #[test]
    fn supplied_track_metadata_is_download_only() {
        assert_eq!(Track::Imtq.info().compressed_bytes, 7_585_316);
        assert_eq!(Track::Dash.info().compressed_bytes, 2_256_476);
        assert_eq!(Track::ALL.len(), 2);
        for track in Track::ALL {
            assert_eq!(track.info().sha256.len(), 64);
            assert!(track
                .info()
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()));
        }
    }
}
