#![cfg_attr(not(test), no_std)]

//! Architecture-level primitives for ExpOS.
//!
//! This crate deliberately exposes Forms, FINs, Dimensions and capability
//! handles instead of paths, file descriptors, users, or POSIX permissions.

mod abi;
mod browser;
mod budget;
mod capability;
mod cfc;
mod diese;
mod display;
mod execution;
mod expfs;
mod fin;
mod form;
mod pimp;
mod protection;
mod recovery;
mod relationship;
mod scope;
mod text;

pub use abi::{AbiCall, AbiRequest, AbiResponse, AbiStatus, FORM_ABI_VERSION, GO_ABI_VERSION};
pub use browser::{
    BorderStyle, BoxEdges, BrowserError, BrowserText, ComputedStyle, CssColor, CssVisibility,
    DisplayMode, Document, DocumentNode, DomEvent, NodeKind, ScriptRejection, ScriptReport,
    SearchResultsDocument, StyledNode, TextAlign, BROWSER_TEXT_CAPACITY,
    MAX_BROWSER_DOCUMENT_BYTES, MAX_BROWSER_NODES, MAX_BROWSER_SCRIPTS, MAX_CLICK_HANDLERS,
    MAX_SCRIPT_BYTES, MAX_SCRIPT_STATEMENTS, MAX_SEARCH_RESULTS, MAX_STYLE_RULES,
};
pub use budget::{BudgetError, ExpBudget, ResourceAccount, ResourceKind, RESOURCE_KIND_COUNT};
pub use capability::{
    Authority, CapabilityBroker, CapabilityError, ExpSeal, FormHandle, Operations,
};
pub use cfc::{Cfc, CfcBinding, CfcCatalog, CfcCatalogError, CfcError, CfcFin};
pub use diese::{Diagnostic, Diese, Resolution};
pub use display::{
    protocol_info as display_protocol_info, BufferFormat, BufferHandle, DisplayError, DisplayEvent,
    DisplayEventKind, DisplayFeatures, DisplayProtocolInfo, DisplayServer, Rect, Surface,
    SurfaceRole, SurfaceState, EXPDISPLAY_PROTOCOL_VERSION,
};
pub use execution::{
    AddressSpace, CpuState, ExecutionContext, ExecutionError, ExecutionEvent, ExecutionIdentity,
    ExecutionRuntime, ExecutionState, Scheduler, MAX_CONTEXT_EVENTS, MAX_CONTEXT_HANDLES,
    MAX_EXECUTION_CONTEXTS,
};
pub use expfs::{
    ExpFs, RecordKey, RecordKind, StorageError, SystemRecord, Transaction, SYSTEM_RECORD_CAPACITY,
};
pub use fin::Fin;
pub use form::{Dimension, Form, FormKind, FormRegistry, Lifecycle, Visibility};
pub use pimp::{NetworkPolicy, PimpScope, PimpSpec, PimpValue, SpecKey};
pub use protection::{
    AeadSuite, Argon2idParameters, BootSupportPolicy, ContentProtection, EncryptedStoragePolicy,
    InstallationMode, PasswordChangeAction, ProtectionError, StorageProtectionPolicy, UnlockMethod,
    ARGON2ID_SALT_BYTES, ARGON2ID_VERSION_13, STORAGE_KEY_BYTES,
};
pub use recovery::{
    CfcRecovery, CheckpointMetadata, InstallationBaseline, RecoveryCatalog, RecoveryError,
    RecoveryMetadata, RestoreSelection, RestoreSource, RestoreTarget, RECOVERY_CHECKPOINT_CAPACITY,
};
pub use relationship::{Relationship, RelationshipError, RelationshipGraph, RelationshipKind};
pub use scope::{ExpScope, ScopeError, MAX_SCOPE_TARGETS};
pub use text::Text;

/// Stable identity of the development image's single CFC. Genesis will mint a
/// fresh CFC FIN for installed systems; this fixed value keeps the pre-Genesis
/// image deterministic and makes CFC ownership explicit today.
pub const DEVELOPMENT_CFC_FIN: CfcFin =
    CfcFin::from_u128(0x4346_4300_0000_0000_0000_0000_0000_0001);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootError {
    RegistryFull,
    DuplicateFin,
    MissingForm,
    MissingDimension,
    PolicyRejected,
    AccessDenied,
    StorageFailure,
    CfcRejected,
    ScopeRejected,
    BudgetRejected,
}

/// Runs the smallest useful Form-native system path used by the bare-metal
/// kernel smoke test: create the Root Form and Stable Dimension, bind them,
/// evaluate early policy, grant a scoped handle, and journal the result.
pub fn bootstrap_demo() -> Result<BootReport, BootError> {
    bootstrap_with_identity(
        DEVELOPMENT_CFC_FIN,
        Text::new("ExpOS Development").expect("static CFC name is valid"),
        Fin::from_u128(0x4449_4d00_0000_0000_0000_0000_0000_0001),
        Text::new("Stable").expect("static Dimension name is valid"),
    )
}

/// Bootstrap a Genesis-created installation with identities minted and
/// persisted by the installer. Runtime policy remains identical to the
/// development bootstrap; only installation identity is supplied externally.
pub fn bootstrap_with_identity(
    cfc_fin: CfcFin,
    cfc_name: Text,
    stable_fin: Fin,
    stable_name: Text,
) -> Result<BootReport, BootError> {
    let root_fin = Fin::from_u128(0x4845_5841_0000_0000_0000_0000_0000_0001);

    let mut cfc =
        Cfc::new(cfc_fin, cfc_name.as_str(), stable_fin).map_err(|_| BootError::CfcRejected)?;
    cfc.own_form(root_fin).map_err(|_| BootError::CfcRejected)?;
    cfc.bind(root_fin, stable_fin)
        .map_err(|_| BootError::CfcRejected)?;

    let mut registry = FormRegistry::new();
    registry
        .register(Form::new(root_fin, "Root", FormKind::Root))
        .map_err(|error| match error {
            form::RegistryError::DuplicateFin => BootError::DuplicateFin,
            _ => BootError::RegistryFull,
        })?;
    registry
        .add_dimension(Dimension::new(stable_fin, stable_name.as_str(), true))
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

    let mut scope =
        ExpScope::new(&cfc, stable_fin, root_fin).map_err(|_| BootError::ScopeRejected)?;
    scope
        .allow(root_fin)
        .map_err(|_| BootError::ScopeRejected)?;

    let mut broker = CapabilityBroker::new(cfc_fin);
    let handle = broker
        .issue_for(
            root_fin,
            Authority::Operator,
            root_fin,
            stable_fin,
            Operations::READ.union(Operations::EXECUTE),
            100,
        )
        .map_err(|_| BootError::AccessDenied)?;
    broker
        .seal_context(root_fin, stable_fin)
        .map_err(|_| BootError::AccessDenied)?;

    let mut budget = ExpBudget::uniform((cfc_fin, stable_fin, root_fin), 64);
    budget
        .configure(ResourceKind::FormOperations, 32, 64)
        .map_err(|_| BootError::BudgetRejected)?;
    budget
        .charge(ResourceKind::FormOperations, 1)
        .map_err(|_| BootError::BudgetRejected)?;

    let mut store = ExpFs::new(&cfc);
    let mut transaction = store.begin();
    transaction
        .stage_form(root_fin, stable_fin, 1)
        .map_err(|_| BootError::StorageFailure)?;
    let sequence = store
        .commit(transaction)
        .map_err(|_| BootError::StorageFailure)?;

    Ok(BootReport {
        cfc_fin,
        cfc_name: *cfc.name(),
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
    pub cfc_fin: CfcFin,
    pub cfc_name: Text,
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
        assert_eq!(report.cfc_fin, DEVELOPMENT_CFC_FIN);
        assert_eq!(report.cfc_name.as_str(), "ExpOS Development");
        assert_eq!(report.handle_id, 1);
        assert_eq!(report.journal_sequence, 1);
    }

    #[test]
    fn genesis_identity_replaces_development_identity() {
        let cfc = CfcFin::from_u128(0x4745_4e45_5349_5300_0000_0000_0000_0001);
        let dimension = Fin::from_u128(0x4745_4e45_5349_5300_0000_0000_0000_0002);
        let report = bootstrap_with_identity(
            cfc,
            Text::new("Studio Fabric").unwrap(),
            dimension,
            Text::new("Primary").unwrap(),
        )
        .unwrap();
        assert_eq!(report.cfc_fin, cfc);
        assert_eq!(report.cfc_name.as_str(), "Studio Fabric");
        assert_eq!(report.stable_fin, dimension);
    }
}
