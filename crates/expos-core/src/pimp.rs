use crate::{Fin, Text};

const MAX_ENTRIES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PimpScope {
    BuiltIn,
    System,
    Subsystem,
    Dimension,
    Form,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecKey {
    Use,
    Backend,
    Network,
    Debug,
    Isolation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkPolicy {
    Open,
    Restricted,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PimpValue {
    Enabled(bool),
    Network(NetworkPolicy),
    Name(Text),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PimpEntry {
    pub key: SpecKey,
    pub value: PimpValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PimpSpec {
    pub scope: PimpScope,
    pub target: Fin,
    pub dimension: Option<Fin>,
    entries: [Option<PimpEntry>; MAX_ENTRIES],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    MalformedLine,
    UnknownKey,
    InvalidValue,
    DuplicateKey,
    TooManyEntries,
}

impl PimpSpec {
    pub fn parse(
        scope: PimpScope,
        target: Fin,
        dimension: Option<Fin>,
        source: &str,
    ) -> Result<Self, ParseError> {
        let mut spec = Self {
            scope,
            target,
            dimension,
            entries: [None; MAX_ENTRIES],
        };
        let mut count = 0;
        for raw_line in source.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (raw_key, raw_value) = line.split_once('=').ok_or(ParseError::MalformedLine)?;
            let key = match raw_key.trim() {
                "use" => SpecKey::Use,
                "backend" => SpecKey::Backend,
                "network" => SpecKey::Network,
                "debug" => SpecKey::Debug,
                "isolation" => SpecKey::Isolation,
                _ => return Err(ParseError::UnknownKey),
            };
            if spec.entries.iter().flatten().any(|entry| entry.key == key) {
                return Err(ParseError::DuplicateKey);
            }
            let value_text = raw_value.trim();
            let value = match key {
                SpecKey::Debug | SpecKey::Isolation => PimpValue::Enabled(match value_text {
                    "enabled" | "true" => true,
                    "disabled" | "false" => false,
                    _ => return Err(ParseError::InvalidValue),
                }),
                SpecKey::Network => PimpValue::Network(match value_text {
                    "open" => NetworkPolicy::Open,
                    "restricted" => NetworkPolicy::Restricted,
                    "disabled" => NetworkPolicy::Disabled,
                    _ => return Err(ParseError::InvalidValue),
                }),
                SpecKey::Use | SpecKey::Backend => {
                    PimpValue::Name(Text::new(value_text).map_err(|_| ParseError::InvalidValue)?)
                }
            };
            if count >= MAX_ENTRIES {
                return Err(ParseError::TooManyEntries);
            }
            spec.entries[count] = Some(PimpEntry { key, value });
            count += 1;
        }
        if count == 0 {
            return Err(ParseError::Empty);
        }
        Ok(spec)
    }

    pub fn entries(&self) -> impl Iterator<Item = &PimpEntry> {
        self.entries.iter().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_is_typed_and_rejects_unknown_switches() {
        let fin = Fin::from_u128(1);
        let spec = PimpSpec::parse(
            PimpScope::Form,
            fin,
            None,
            "network=restricted\ndebug=enabled",
        )
        .unwrap();
        assert_eq!(spec.entries().count(), 2);
        assert_eq!(
            PimpSpec::parse(PimpScope::Form, fin, None, "whatever=yes"),
            Err(ParseError::UnknownKey)
        );
    }
}
