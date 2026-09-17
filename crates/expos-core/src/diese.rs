use crate::{Fin, NetworkPolicy, PimpScope, PimpSpec, PimpValue, SpecKey, Text};

const MAX_SPECS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Diagnostic {
    TooManySpecifications,
    Conflict { key: SpecKey, scope: PimpScope },
    MissingPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub use_mode: Option<Text>,
    pub backend: Option<Text>,
    pub network: Option<NetworkPolicy>,
    pub debug: Option<bool>,
    pub isolation: Option<bool>,
}

#[derive(Clone, Copy)]
struct Selected {
    scope: PimpScope,
    value: PimpValue,
}

pub struct Diese {
    specs: [Option<PimpSpec>; MAX_SPECS],
}

impl Diese {
    pub const fn new() -> Self {
        Self {
            specs: [None; MAX_SPECS],
        }
    }

    pub fn push(&mut self, spec: PimpSpec) -> Result<(), Diagnostic> {
        let slot = self
            .specs
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(Diagnostic::TooManySpecifications)?;
        *slot = Some(spec);
        Ok(())
    }

    pub fn resolve(&self, target: Fin, dimension: Fin) -> Result<Resolution, Diagnostic> {
        let mut selected: [Option<Selected>; 5] = [None; 5];
        let mut matched = false;
        for spec in self.specs.iter().flatten() {
            if spec.target != target {
                continue;
            }
            if let Some(required_dimension) = spec.dimension {
                if required_dimension != dimension {
                    continue;
                }
            }
            matched = true;
            for entry in spec.entries() {
                let index = key_index(entry.key);
                match selected[index] {
                    Some(current) if current.scope > spec.scope => {}
                    Some(current)
                        if current.scope == spec.scope && current.value != entry.value =>
                    {
                        return Err(Diagnostic::Conflict {
                            key: entry.key,
                            scope: spec.scope,
                        });
                    }
                    _ => {
                        selected[index] = Some(Selected {
                            scope: spec.scope,
                            value: entry.value,
                        })
                    }
                }
            }
        }
        if !matched {
            return Err(Diagnostic::MissingPolicy);
        }
        Ok(Resolution {
            use_mode: name_value(selected[0]),
            backend: name_value(selected[1]),
            network: network_value(selected[2]),
            debug: bool_value(selected[3]),
            isolation: bool_value(selected[4]),
        })
    }
}

impl Default for Diese {
    fn default() -> Self {
        Self::new()
    }
}

const fn key_index(key: SpecKey) -> usize {
    match key {
        SpecKey::Use => 0,
        SpecKey::Backend => 1,
        SpecKey::Network => 2,
        SpecKey::Debug => 3,
        SpecKey::Isolation => 4,
    }
}

fn name_value(selected: Option<Selected>) -> Option<Text> {
    match selected {
        Some(Selected {
            value: PimpValue::Name(value),
            ..
        }) => Some(value),
        _ => None,
    }
}
fn network_value(selected: Option<Selected>) -> Option<NetworkPolicy> {
    match selected {
        Some(Selected {
            value: PimpValue::Network(value),
            ..
        }) => Some(value),
        _ => None,
    }
}
fn bool_value(selected: Option<Selected>) -> Option<bool> {
    match selected {
        Some(Selected {
            value: PimpValue::Enabled(value),
            ..
        }) => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn form_policy_overrides_builtin_policy() {
        let target = Fin::from_u128(1);
        let dimension = Fin::from_u128(2);
        let mut diese = Diese::new();
        diese
            .push(
                PimpSpec::parse(
                    PimpScope::BuiltIn,
                    target,
                    None,
                    "network=open\ndebug=disabled",
                )
                .unwrap(),
            )
            .unwrap();
        diese
            .push(
                PimpSpec::parse(
                    PimpScope::Form,
                    target,
                    Some(dimension),
                    "network=restricted",
                )
                .unwrap(),
            )
            .unwrap();
        let result = diese.resolve(target, dimension).unwrap();
        assert_eq!(result.network, Some(NetworkPolicy::Restricted));
        assert_eq!(result.debug, Some(false));
    }

    #[test]
    fn same_scope_conflicts_are_explainable() {
        let target = Fin::from_u128(1);
        let dimension = Fin::from_u128(2);
        let mut diese = Diese::new();
        diese
            .push(PimpSpec::parse(PimpScope::Form, target, None, "network=open").unwrap())
            .unwrap();
        diese
            .push(PimpSpec::parse(PimpScope::Form, target, None, "network=disabled").unwrap())
            .unwrap();
        assert_eq!(
            diese.resolve(target, dimension),
            Err(Diagnostic::Conflict {
                key: SpecKey::Network,
                scope: PimpScope::Form
            })
        );
    }
}
