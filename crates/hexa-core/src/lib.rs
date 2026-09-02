#![cfg_attr(not(test), no_std)]

//! Architecture-level primitives for HexaOS.
//!
//! This crate deliberately exposes Forms, FINs, Dimensions and capability
//! handles instead of paths, file descriptors, users, or POSIX permissions.

mod capability;
mod diese;
mod fin;
mod form;
mod hexafs;
mod pimp;
mod relationship;
mod text;

pub use capability::{Authority, CapabilityBroker, FormHandle, Operations};
pub use diese::{Diagnostic, Diese, Resolution};
pub use fin::Fin;
pub use form::{Dimension, Form, FormKind, FormRegistry, Lifecycle, Visibility};
pub use hexafs::{HexaFs, Transaction};
pub use pimp::{NetworkPolicy, PimpScope, PimpSpec, PimpValue, SpecKey};
pub use relationship::{Relationship, RelationshipError, RelationshipGraph, RelationshipKind};
pub use text::Text;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootError {
    RegistryFull,
    DuplicateFin,
    MissingForm,
    MissingDimension,
    PolicyRejected,
    AccessDenied,
    StorageFailure,
}

/// Runs the smallest useful Form-native system path used by the bare-metal
/// kernel smoke test: create the Root Form and Stable Dimension, bind them,
/// evaluate early policy, grant a scoped handle, and journal the result.
pub fn bootstrap_demo() -> Result<BootReport, BootError> {
    let root_fin = Fin::from_u128(0x4845_5841_0000_0000_0000_0000_0000_0001);
    let stable_fin = Fin::from_u128(0x4449_4d00_0000_0000_0000_0000_0000_0001);

    let mut registry = FormRegistry::new();
    registry
        .register(Form::new(root_fin, "Root", FormKind::Root))
        .map_err(|error| match error {
            form::RegistryError::DuplicateFin => BootError::DuplicateFin,
            _ => BootError::RegistryFull,
        })?;
    registry
        .add_dimension(Dimension::new(stable_fin, "Stable", true))
        .map_err(|_| BootError::RegistryFull)?;
    registry
        .bind(root_fin, stable_fin, 1, Visibility::Visible)
        .map_err(|_| BootError::MissingForm)?;

    let defaults = PimpSpec::parse(
        PimpScope::BuiltIn,
        root_fin,
        None,
        "use=service\nnetwork=restricted\nisolation=enabled",
    )
    .map_err(|_| BootError::PolicyRejected)?;
    let mut diese = Diese::new();
    diese
        .push(defaults)
        .map_err(|_| BootError::PolicyRejected)?;
    let resolution = diese
        .resolve(root_fin, stable_fin)
        .map_err(|_| BootError::PolicyRejected)?;

    let mut broker = CapabilityBroker::new();
    let handle = broker
        .issue(
            Authority::Operator,
            root_fin,
            stable_fin,
            Operations::READ.union(Operations::EXECUTE),
            100,
        )
        .map_err(|_| BootError::AccessDenied)?;

    let mut store = HexaFs::new();
    let mut transaction = store.begin();
    transaction
        .stage_form(root_fin, stable_fin, 1)
        .map_err(|_| BootError::StorageFailure)?;
    let sequence = store
        .commit(transaction)
        .map_err(|_| BootError::StorageFailure)?;

    Ok(BootReport {
        root_fin,
        stable_fin,
        handle_id: handle.id,
        journal_sequence: sequence,
        network_restricted: resolution.network == Some(NetworkPolicy::Restricted),
        isolated: resolution.isolation == Some(true),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootReport {
    pub root_fin: Fin,
    pub stable_fin: Fin,
    pub handle_id: u32,
    pub journal_sequence: u32,
    pub network_restricted: bool,
    pub isolated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_exercises_the_form_native_path() {
        let report = bootstrap_demo().unwrap();
        assert!(report.network_restricted);
        assert!(report.isolated);
        assert_eq!(report.handle_id, 1);
        assert_eq!(report.journal_sequence, 1);
    }
}
