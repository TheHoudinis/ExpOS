use core::fmt;

/// Stable 128-bit Form Identification Number.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Fin([u8; 16]);

impl Fin {
    pub const ZERO: Self = Self([0; 16]);

    pub const fn from_u128(value: u128) -> Self {
        Self(value.to_be_bytes())
    }

    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        let mut index = 0;
        while index < self.0.len() {
            if self.0[index] != 0 {
                return false;
            }
            index += 1;
        }
        true
    }
}

impl fmt::Debug for Fin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for Fin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            write!(formatter, "{byte:02X}")?;
            if matches!(index, 3 | 5 | 7 | 9) {
                formatter.write_str("-")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_a_stable_fin() {
        let fin = Fin::from_u128(0x001A_72F0_1234_5678_9ABC_DEF0_1122_3344);
        assert_eq!(fin.to_string(), "001A72F0-1234-5678-9ABC-DEF011223344");
    }
}
