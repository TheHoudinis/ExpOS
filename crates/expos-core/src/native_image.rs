//! EXF1: a bounded, relocation-free x86-64 executable Form container.
//! Validation grants no authority: admission still requires scoped Handles.

pub const NATIVE_IMAGE_MAGIC: &[u8; 4] = b"EXF1";
pub const NATIVE_IMAGE_HEADER: usize = 24;
pub const NATIVE_IMAGE_LIMIT: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeImageError {
    Header,
    Unsupported,
    Length,
    Entry,
}

#[derive(Clone, Copy, Debug)]
pub struct NativeImage<'a> {
    pub code: &'a [u8],
    pub data: &'a [u8],
    pub zero_bytes: usize,
    pub entry: usize,
}

impl<'a> NativeImage<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, NativeImageError> {
        if bytes.len() < NATIVE_IMAGE_HEADER || !bytes.starts_with(NATIVE_IMAGE_MAGIC) {
            return Err(NativeImageError::Header);
        }
        let word = |at| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
        if word(4) != 1 || word(6) != 1 || word(8) != 1 || word(10) != 0 || bytes[20..24] != [0; 4]
        {
            return Err(NativeImageError::Unsupported);
        }
        let (code, data, zero_bytes, entry) = (word(12), word(14), word(16), word(18));
        // All lengths are widened before addition; no allocation or unchecked indexing.
        if bytes.len() > NATIVE_IMAGE_LIMIT
            || code == 0
            || code > 4096
            || data + zero_bytes > 4096 - 256
            || NATIVE_IMAGE_HEADER + code + data != bytes.len()
        {
            return Err(NativeImageError::Length);
        }
        if entry >= code {
            return Err(NativeImageError::Entry);
        }
        Ok(Self {
            code: &bytes[NATIVE_IMAGE_HEADER..NATIVE_IMAGE_HEADER + code],
            data: &bytes[NATIVE_IMAGE_HEADER + code..],
            zero_bytes,
            entry,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> [u8; 28] {
        let mut bytes = [0; 28];
        bytes[..4].copy_from_slice(NATIVE_IMAGE_MAGIC);
        bytes[4] = 1;
        bytes[6] = 1;
        bytes[8] = 1;
        bytes[12] = 2;
        bytes[14] = 2;
        bytes[16..18].copy_from_slice(&3838_u16.to_le_bytes());
        bytes[18] = 1;
        bytes[24..].copy_from_slice(&[0x90, 0xc3, 42, 43]);
        bytes
    }

    #[test]
    fn validates_segments_and_all_truncations() {
        let bytes = valid();
        let image = NativeImage::parse(&bytes).unwrap();
        assert_eq!(image.code, [0x90, 0xc3]);
        assert_eq!(image.data, [42, 43]);
        assert_eq!((image.entry, image.zero_bytes), (1, 3838));
        for length in 0..bytes.len() {
            assert!(NativeImage::parse(&bytes[..length]).is_err());
        }
    }

    #[test]
    fn rejects_incompatible_headers_and_segment_boundaries() {
        for offset in [0, 4, 6, 8, 10, 20, 21, 22, 23] {
            let mut bytes = valid();
            bytes[offset] ^= 0x80;
            assert!(NativeImage::parse(&bytes).is_err());
        }
        for (at, value) in [(12, 0), (12, 65535), (14, 65535), (16, 3839), (18, 2)] {
            let mut bytes = valid();
            bytes[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes());
            assert!(NativeImage::parse(&bytes).is_err());
        }
        let mut extra = valid().to_vec();
        extra.push(0);
        assert!(NativeImage::parse(&extra).is_err());
        let mut oversized = [0; NATIVE_IMAGE_LIMIT + 1];
        oversized[..28].copy_from_slice(&valid());
        oversized[12..14].copy_from_slice(
            &((NATIVE_IMAGE_LIMIT + 1 - NATIVE_IMAGE_HEADER) as u16).to_le_bytes(),
        );
        assert!(NativeImage::parse(&oversized).is_err());
    }
}
