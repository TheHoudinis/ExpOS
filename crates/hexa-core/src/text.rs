use core::fmt;

pub const TEXT_CAPACITY: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Text {
    bytes: [u8; TEXT_CAPACITY],
    len: u8,
}

impl Text {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; TEXT_CAPACITY],
            len: 0,
        }
    }

    pub fn new(value: &str) -> Result<Self, TextError> {
        if value.is_empty() {
            return Err(TextError::Empty);
        }
        if value.len() > TEXT_CAPACITY || !value.is_ascii() {
            return Err(TextError::Invalid);
        }
        let mut text = Self::empty();
        text.bytes[..value.len()].copy_from_slice(value.as_bytes());
        text.len = value.len() as u8;
        Ok(text)
    }

    pub fn as_str(&self) -> &str {
        // Text::new only accepts ASCII, so the occupied prefix is UTF-8.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len as usize]) }
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Text").field(&self.as_str()).finish()
    }
}

impl fmt::Display for Text {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextError {
    Empty,
    Invalid,
}
