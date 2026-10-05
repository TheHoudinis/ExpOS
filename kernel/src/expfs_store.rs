//! Disk-backed ExpFS system-database snapshot for the native kernel.
//!
//! The newest valid of two full CFC snapshots is loaded. Commits write the
//! inactive payload sectors first and its header sector last, so interruption
//! cannot replace the previous valid generation with a partial transaction.

use crate::{crypto, println, slog, storage};
use expos_core::{
    CfcFin, Dimension, Fin, Form, FormHandle, FormKind, Lifecycle, NetworkPolicy, Operations,
    Relationship, RelationshipKind, Text,
};

pub const MAX_STORED_FORMS: usize = 12;
pub const MAX_STORED_DIMENSIONS: usize = 6;
pub const MAX_STORED_RELATIONSHIPS: usize = 48;
pub const MAX_STORED_HANDLES: usize = 16;
pub const FORM_CONTENT_CAPACITY: usize = 512;
pub const SETTINGS_RECORD_CAPACITY: usize = 32;
pub const ACCOUNTS_RECORD_CAPACITY: usize = 12 * 80;
pub const CHECKPOINT_CAPACITY: usize = 8;

const MAGIC: [u8; 8] = *b"EXPFSDB1";
const FORMAT_VERSION: u16 = 2;
const HEADER_LEN: usize = 64;
const SLOT_SECTORS: usize = 24;
const SLOT_LEN: usize = SLOT_SECTORS * storage::SECTOR_SIZE;
const PAYLOAD_LEN: usize = SLOT_LEN - HEADER_LEN;
// GPT reserves LBAs 0..33. Genesis stores its immutable identity manifest at
// LBA 40, so the transactional ExpFS region begins at 64 and remains outside
// the EFI System Partition, which begins at LBA 2048.
const SLOT_A_LBA: u32 = 64;
const SLOT_B_LBA: u32 = SLOT_A_LBA + SLOT_SECTORS as u32;
const CHECKPOINT_LBA: u32 = SLOT_B_LBA + SLOT_SECTORS as u32;
const BASELINE_LBA: u32 = CHECKPOINT_LBA + (CHECKPOINT_CAPACITY * SLOT_SECTORS) as u32;
const MINIMUM_DISK_SECTORS: u32 = BASELINE_LBA + SLOT_SECTORS as u32;

#[derive(Clone, Copy)]
pub struct StoredForm {
    pub form: Form,
    pub content: [u8; FORM_CONTENT_CAPACITY],
    pub content_len: u16,
}

#[derive(Clone, Copy)]
pub struct Snapshot {
    pub cfc: CfcFin,
    pub cfc_name: Text,
    pub primary_dimension: Fin,
    pub forms: [Option<StoredForm>; MAX_STORED_FORMS],
    pub dimensions: [Option<Dimension>; MAX_STORED_DIMENSIONS],
    pub relationships: [Option<Relationship>; MAX_STORED_RELATIONSHIPS],
    pub handles: [Option<FormHandle>; MAX_STORED_HANDLES],
    pub next_fin: u32,
    pub next_dimension_fin: u32,
    pub journal_sequence: u32,
    pub network_policy: NetworkPolicy,
    form_graph_present: bool,
    settings: Option<[u8; SETTINGS_RECORD_CAPACITY]>,
    accounts: Option<[u8; ACCOUNTS_RECORD_CAPACITY]>,
}

impl Snapshot {
    pub fn empty(cfc: CfcFin, cfc_name: Text, primary_dimension: Fin) -> Self {
        Self {
            cfc,
            cfc_name,
            primary_dimension,
            forms: [None; MAX_STORED_FORMS],
            dimensions: [None; MAX_STORED_DIMENSIONS],
            relationships: [None; MAX_STORED_RELATIONSHIPS],
            handles: [None; MAX_STORED_HANDLES],
            next_fin: 2,
            next_dimension_fin: 2,
            journal_sequence: 1,
            network_policy: NetworkPolicy::Restricted,
            form_graph_present: false,
            settings: None,
            accounts: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    Unavailable,
    Full,
    WrongCfc,
    InvalidSnapshot,
    Disk(storage::StorageError),
    VerificationFailed,
    NoCurrentState,
    CheckpointNotFound,
    BaselineNotFound,
    BaselineDamaged,
}

impl StoreError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "ExpFS disk is unavailable",
            Self::Full => "ExpFS Form capacity is full",
            Self::WrongCfc => "snapshot belongs to another CFC",
            Self::InvalidSnapshot => "snapshot violates the ExpFS schema",
            Self::Disk(_) => "ExpFS disk I/O failed",
            Self::VerificationFailed => "ExpFS commit verification failed",
            Self::NoCurrentState => "ExpFS has no current state to checkpoint",
            Self::CheckpointNotFound => "ExpFS checkpoint does not exist",
            Self::BaselineNotFound => "ExpFS installation baseline does not exist",
            Self::BaselineDamaged => "ExpFS installation baseline is damaged",
        }
    }
}

struct RuntimeStore {
    device: Option<storage::Device>,
    cfc: CfcFin,
    cfc_name: Text,
    primary_dimension: Fin,
    snapshot: Option<Snapshot>,
    generation: u64,
    active_slot: u8,
    checkpoints: [Option<CheckpointInfo>; CHECKPOINT_CAPACITY],
    baseline: BaselineState,
    storage_key: Option<[u8; crypto::STORAGE_KEY_LEN]>,
}

impl RuntimeStore {
    const fn new() -> Self {
        Self {
            device: None,
            cfc: CfcFin::ZERO,
            cfc_name: Text::empty(),
            primary_dimension: Fin::ZERO,
            snapshot: None,
            generation: 0,
            active_slot: 1,
            checkpoints: [None; CHECKPOINT_CAPACITY],
            baseline: BaselineState::Missing,
            storage_key: None,
        }
    }
}

static STORE: crate::sync::SpinMutex<RuntimeStore> =
    crate::sync::SpinMutex::new(RuntimeStore::new());

#[derive(Clone, Copy)]
struct DecodedSlot {
    snapshot: Snapshot,
    generation: u64,
    slot: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointInfo {
    pub state_id: u64,
    pub journal_sequence: u32,
    pub slot: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaselineInfo {
    pub generation: u64,
    pub journal_sequence: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaselineState {
    Missing,
    Ready(BaselineInfo),
    Damaged,
}

pub fn initialize(
    cfc: CfcFin,
    cfc_name: Text,
    primary_dimension: Fin,
    storage_key: Option<[u8; crypto::STORAGE_KEY_LEN]>,
) {
    let device = match storage::initialize() {
        Ok(device) if device.sectors() >= MINIMUM_DISK_SECTORS => device,
        Ok(device) => {
            println!(
                "[warn] ExpFS disk too small: {} sectors (need {})",
                device.sectors(),
                MINIMUM_DISK_SECTORS
            );
            slog!("EXPOS_EXPFS_VOLATILE reason=disk-too-small\r\n");
            return;
        }
        Err(error) => {
            println!("[warn] ExpFS persistence unavailable: {:?}", error);
            slog!("EXPOS_EXPFS_VOLATILE error={:?}\r\n", error);
            return;
        }
    };

    let first = read_slot(device, 0, storage_key.as_ref())
        .ok()
        .filter(|slot| {
            slot.snapshot.cfc == cfc && slot.snapshot.primary_dimension == primary_dimension
        });
    let second = read_slot(device, 1, storage_key.as_ref())
        .ok()
        .filter(|slot| {
            slot.snapshot.cfc == cfc && slot.snapshot.primary_dimension == primary_dimension
        });
    let selected = newest(first, second);
    let mut store = STORE.lock();
    store.device = Some(device);
    store.cfc = cfc;
    store.cfc_name = cfc_name;
    store.primary_dimension = primary_dimension;
    store.storage_key = storage_key;
    store.checkpoints = [None; CHECKPOINT_CAPACITY];
    for index in 0..CHECKPOINT_CAPACITY {
        if let Ok(checkpoint) = read_at(
            device,
            checkpoint_lba(index),
            index as u8,
            storage_key.as_ref(),
        ) {
            if checkpoint.snapshot.cfc == cfc
                && checkpoint.snapshot.primary_dimension == primary_dimension
            {
                store.checkpoints[index] = Some(CheckpointInfo {
                    state_id: checkpoint.generation,
                    journal_sequence: checkpoint.snapshot.journal_sequence,
                    slot: index as u8,
                });
            }
        }
    }
    store.baseline = match read_at(device, BASELINE_LBA, u8::MAX, storage_key.as_ref()) {
        Ok(baseline)
            if baseline.snapshot.cfc == cfc
                && baseline.snapshot.primary_dimension == primary_dimension =>
        {
            let info = BaselineInfo {
                generation: baseline.generation,
                journal_sequence: baseline.snapshot.journal_sequence,
            };
            println!(
                "[ok] protected ExpFS installation baseline generation {} loaded",
                info.generation
            );
            slog!(
                "EXPOS_EXPFS_BASELINE_READY generation={} sequence={} source=disk\r\n",
                info.generation,
                info.journal_sequence
            );
            BaselineState::Ready(info)
        }
        _ if region_is_blank(device, BASELINE_LBA) => BaselineState::Missing,
        _ => {
            println!("[warn] protected ExpFS installation baseline is damaged");
            slog!("EXPOS_EXPFS_BASELINE_DAMAGED\r\n");
            BaselineState::Damaged
        }
    };
    if let Some(slot) = selected {
        store.snapshot = Some(slot.snapshot);
        store.generation = slot.generation;
        store.active_slot = slot.slot;
        println!(
            "[ok] ExpFS CFC database generation {} loaded from slot {}",
            slot.generation,
            if slot.slot == 0 { "A" } else { "B" }
        );
        slog!(
            "EXPOS_EXPFS_READY generation={} slot={} forms={}\r\n",
            slot.generation,
            if slot.slot == 0 { "A" } else { "B" },
            slot.snapshot.forms.iter().flatten().count()
        );
    } else {
        println!("[ok] blank ExpFS CFC database; first mutation will create it");
        slog!("EXPOS_EXPFS_READY generation=0 slot=none forms=0\r\n");
    }
}

pub fn loaded_snapshot(cfc: CfcFin) -> Option<Snapshot> {
    let store = STORE.lock();
    (store.cfc == cfc)
        .then_some(store.snapshot)
        .flatten()
        .filter(|snapshot| snapshot.form_graph_present)
}

/// Return the durable CFC state even before the interactive shell has
/// committed its complete Form graph. Genesis uses this phase to seed only
/// installation policy, dimensions and package selections.
pub fn current_snapshot(cfc: CfcFin) -> Option<Snapshot> {
    let store = STORE.lock();
    (store.cfc == cfc).then_some(store.snapshot).flatten()
}

/// Seed a newly installed CFC from its authenticated Genesis plan.
///
/// This is deliberately one-shot: an existing ExpFS snapshot is authoritative
/// so a later boot cannot reinstall packages or recreate dimensions the user
/// removed after installation.
pub fn seed_genesis_plan(
    primary_name: Text,
    plan: crate::genesis::GenesisPlan,
) -> Result<u64, StoreError> {
    const PLAN_FIN: Fin = Fin::from_u128(0x4745_4E45_5349_5350_4C41_4E00_0000_0001);
    const APPS_FIN: Fin = Fin::from_u128(0x4159_4F41_5050_5353_5441_5445_0000_0001);

    let mut store = STORE.lock();
    if store.device.is_none() || store.cfc.is_zero() || store.primary_dimension.is_zero() {
        return Err(StoreError::Unavailable);
    }
    if store.snapshot.is_some() {
        return Ok(store.generation);
    }

    let mut snapshot = Snapshot::empty(store.cfc, store.cfc_name, store.primary_dimension);
    snapshot.dimensions[0] = Some(Dimension::new(
        store.primary_dimension,
        primary_name.as_str(),
        true,
    ));
    for (target, dimension) in snapshot
        .dimensions
        .iter_mut()
        .skip(1)
        .zip(plan.secondary_dimensions.into_iter().flatten())
    {
        *target = Some(Dimension::new(dimension.fin, dimension.name.as_str(), true));
    }
    snapshot.network_policy = if plan.network_allowed() {
        NetworkPolicy::Restricted
    } else {
        NetworkPolicy::Disabled
    };

    let mut plan_bytes = [0_u8; FORM_CONTENT_CAPACITY];
    plan_bytes[..4].copy_from_slice(b"GNP2");
    plan_bytes[4] = plan.components;
    plan_bytes[5] = plan.sdks;
    plan_bytes[6] = plan.drivers;
    plan_bytes[7] = plan.encryption.persisted();
    plan_bytes[8..12].copy_from_slice(&plan.initial_apps.to_le_bytes());
    let mut plan_form = Form::new(PLAN_FIN, "GenesisPlan.state", FormKind::Policy);
    plan_form.revision = 1;
    snapshot.forms[0] = Some(StoredForm {
        form: plan_form,
        content: plan_bytes,
        content_len: 12,
    });

    let mut app_bytes = [0_u8; FORM_CONTENT_CAPACITY];
    app_bytes[..4].copy_from_slice(b"AYO1");
    app_bytes[4..8].copy_from_slice(&plan.initial_apps.to_le_bytes());
    app_bytes[8] = plan.initial_apps.trailing_zeros().min(19) as u8;
    let mut apps_form = Form::new(APPS_FIN, "AyoApps.state", FormKind::Data);
    apps_form.revision = 1;
    snapshot.forms[1] = Some(StoredForm {
        form: apps_form,
        content: app_bytes,
        content_len: 22,
    });
    snapshot.journal_sequence = 2;
    validate_snapshot(&snapshot)?;
    let generation = commit_locked(&mut store, snapshot)?;
    slog!(
        "EXPOS_GENESIS_PLAN_SEEDED generation={} dimensions={} apps={} components={:#04x}\r\n",
        generation,
        snapshot.dimensions.iter().flatten().count(),
        plan.initial_apps.count_ones(),
        plan.components
    );
    Ok(generation)
}

/// Load the first active Data Form whose name ends in `.txt`.
///
/// Notes uses the same generic Form records as the shell rather than a private
/// side file, so saved text remains visible to every ExpFS-aware tool.
pub fn load_text_form(cfc: CfcFin) -> Option<(Text, [u8; FORM_CONTENT_CAPACITY], usize)> {
    let snapshot = current_snapshot(cfc)?;
    snapshot.forms.iter().flatten().find_map(|stored| {
        (stored.form.kind == FormKind::Data
            && stored.form.lifecycle == Lifecycle::Active
            && stored.form.name.as_str().ends_with(".txt"))
        .then_some((
            stored.form.name,
            stored.content,
            stored.content_len as usize,
        ))
    })
}

pub fn load_data_form(cfc: CfcFin, name: &str) -> Option<([u8; FORM_CONTENT_CAPACITY], usize)> {
    let snapshot = current_snapshot(cfc)?;
    snapshot.forms.iter().flatten().find_map(|stored| {
        (stored.form.kind == FormKind::Data
            && stored.form.lifecycle == Lifecycle::Active
            && stored.form.name.as_str() == name)
            .then_some((stored.content, stored.content_len as usize))
    })
}

/// Atomically create or revise a named text Data Form in the active CFC.
pub fn save_text_form(cfc: CfcFin, name: &str, bytes: &[u8]) -> Result<(Fin, u32), StoreError> {
    if !name.ends_with(".txt") {
        return Err(StoreError::InvalidSnapshot);
    }
    save_data_form(cfc, name, bytes)
}

/// Commit application state through the same generic Data Form transaction
/// used by text documents and shell-created data.
pub fn save_data_form(cfc: CfcFin, name: &str, bytes: &[u8]) -> Result<(Fin, u32), StoreError> {
    if bytes.len() > FORM_CONTENT_CAPACITY {
        return Err(StoreError::InvalidSnapshot);
    }
    let name = Text::new(name).map_err(|_| StoreError::InvalidSnapshot)?;
    let mut store = STORE.lock();
    if store.device.is_none() || store.cfc != cfc || store.primary_dimension.is_zero() {
        return Err(StoreError::Unavailable);
    }
    let mut snapshot = store
        .snapshot
        .unwrap_or_else(|| Snapshot::empty(store.cfc, store.cfc_name, store.primary_dimension));
    let existing = snapshot.forms.iter().position(|entry| {
        entry.is_some_and(|stored| stored.form.kind == FormKind::Data && stored.form.name == name)
    });
    let slot = existing.or_else(|| snapshot.forms.iter().position(Option::is_none));
    let index = slot.ok_or(StoreError::Full)?;
    let (fin, revision) = if let Some(stored) = snapshot.forms[index] {
        (stored.form.fin, stored.form.revision.wrapping_add(1).max(1))
    } else {
        let fin =
            Fin::from_u128(0x464F_524D_0000_0000_0000_0000_0000_0000 | snapshot.next_fin as u128);
        snapshot.next_fin = snapshot.next_fin.wrapping_add(1).max(2);
        (fin, 1)
    };
    let mut content = [0_u8; FORM_CONTENT_CAPACITY];
    content[..bytes.len()].copy_from_slice(bytes);
    let mut form = Form::new(fin, name.as_str(), FormKind::Data);
    form.revision = revision;
    snapshot.forms[index] = Some(StoredForm {
        form,
        content,
        content_len: bytes.len() as u16,
    });
    if existing.is_none() {
        if let Some(root) = snapshot
            .forms
            .iter()
            .flatten()
            .find(|stored| stored.form.kind == FormKind::Root)
            .map(|stored| stored.form.fin)
        {
            if let Some(target) = snapshot
                .relationships
                .iter_mut()
                .find(|entry| entry.is_none())
            {
                *target = Some(Relationship {
                    source: root,
                    target: fin,
                    kind: RelationshipKind::Contains,
                    dimension: Some(snapshot.primary_dimension),
                });
            }
        }
    }
    snapshot.journal_sequence = snapshot.journal_sequence.wrapping_add(1).max(1);
    validate_snapshot(&snapshot)?;
    commit_locked(&mut store, snapshot)?;
    Ok((fin, revision))
}

pub fn system_records() -> (
    Option<[u8; SETTINGS_RECORD_CAPACITY]>,
    Option<[u8; ACCOUNTS_RECORD_CAPACITY]>,
) {
    let store = STORE.lock();
    store
        .snapshot
        .map(|snapshot| (snapshot.settings, snapshot.accounts))
        .unwrap_or((None, None))
}

pub fn generation() -> Option<u64> {
    let store = STORE.lock();
    store.snapshot.map(|_| store.generation)
}

pub fn available() -> bool {
    STORE.lock().device.is_some()
}

pub fn checkpoints() -> [Option<CheckpointInfo>; CHECKPOINT_CAPACITY] {
    STORE.lock().checkpoints
}

pub fn baseline() -> BaselineState {
    STORE.lock().baseline
}

/// Create the one protected installation baseline if this CFC does not yet
/// have one. A present or damaged baseline is never overwritten.
pub fn establish_baseline() -> Result<BaselineInfo, StoreError> {
    let mut store = STORE.lock();
    match store.baseline {
        BaselineState::Ready(info) => return Ok(info),
        BaselineState::Damaged => return Err(StoreError::BaselineDamaged),
        BaselineState::Missing => {}
    }
    let snapshot = store.snapshot.ok_or(StoreError::NoCurrentState)?;
    let device = store.device.ok_or(StoreError::Unavailable)?;
    let generation = store.generation.max(1);
    let mut encoded = [0_u8; SLOT_LEN];
    encode_slot(
        &snapshot,
        generation,
        store.storage_key.as_ref(),
        BASELINE_LBA,
        &mut encoded,
    )?;
    write_at(device, BASELINE_LBA, &encoded)?;
    let verified = read_at(device, BASELINE_LBA, u8::MAX, store.storage_key.as_ref())?;
    if verified.generation != generation
        || verified.snapshot.cfc != snapshot.cfc
        || verified.snapshot.primary_dimension != snapshot.primary_dimension
        || verified.snapshot.journal_sequence != snapshot.journal_sequence
    {
        store.baseline = BaselineState::Damaged;
        return Err(StoreError::VerificationFailed);
    }
    let info = BaselineInfo {
        generation,
        journal_sequence: snapshot.journal_sequence,
    };
    store.baseline = BaselineState::Ready(info);
    slog!(
        "EXPOS_EXPFS_BASELINE_READY generation={} sequence={} source=created\r\n",
        info.generation,
        info.journal_sequence
    );
    Ok(info)
}

pub fn restore_baseline() -> Result<u64, StoreError> {
    let mut store = STORE.lock();
    match store.baseline {
        BaselineState::Missing => return Err(StoreError::BaselineNotFound),
        BaselineState::Damaged => return Err(StoreError::BaselineDamaged),
        BaselineState::Ready(_) => {}
    }
    let device = store.device.ok_or(StoreError::Unavailable)?;
    let mut snapshot = read_at(device, BASELINE_LBA, u8::MAX, store.storage_key.as_ref())?.snapshot;
    if let Some(current) = store.snapshot {
        snapshot.journal_sequence = current.journal_sequence.wrapping_add(1).max(1);
    }
    let generation = commit_locked(&mut store, snapshot)?;
    slog!(
        "EXPOS_EXPFS_BASELINE_RESTORE generation={} sequence={}\r\n",
        generation,
        snapshot.journal_sequence
    );
    Ok(generation)
}

pub fn record_checkpoint() -> Result<CheckpointInfo, StoreError> {
    let mut store = STORE.lock();
    let snapshot = store.snapshot.ok_or(StoreError::NoCurrentState)?;
    let device = store.device.ok_or(StoreError::Unavailable)?;
    let state_id = store
        .checkpoints
        .iter()
        .flatten()
        .map(|checkpoint| checkpoint.state_id)
        .max()
        .unwrap_or(0)
        .wrapping_add(1)
        .max(1);
    let slot = store
        .checkpoints
        .iter()
        .position(Option::is_none)
        .or_else(|| {
            store
                .checkpoints
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| entry.map(|entry| (index, entry.state_id)))
                .min_by_key(|(_, state_id)| *state_id)
                .map(|(index, _)| index)
        })
        .ok_or(StoreError::VerificationFailed)?;
    let mut encoded = [0_u8; SLOT_LEN];
    encode_slot(
        &snapshot,
        state_id,
        store.storage_key.as_ref(),
        checkpoint_lba(slot),
        &mut encoded,
    )?;
    write_at(device, checkpoint_lba(slot), &encoded)?;
    let verified = read_at(
        device,
        checkpoint_lba(slot),
        slot as u8,
        store.storage_key.as_ref(),
    )?;
    if verified.generation != state_id
        || verified.snapshot.cfc != snapshot.cfc
        || verified.snapshot.journal_sequence != snapshot.journal_sequence
    {
        return Err(StoreError::VerificationFailed);
    }
    let checkpoint = CheckpointInfo {
        state_id,
        journal_sequence: snapshot.journal_sequence,
        slot: slot as u8,
    };
    store.checkpoints[slot] = Some(checkpoint);
    slog!(
        "EXPOS_EXPFS_CHECKPOINT state={} sequence={} slot={}\r\n",
        state_id,
        snapshot.journal_sequence,
        slot
    );
    Ok(checkpoint)
}

pub fn restore_checkpoint(state_id: u64) -> Result<u64, StoreError> {
    let mut store = STORE.lock();
    let checkpoint = store
        .checkpoints
        .iter()
        .flatten()
        .find(|checkpoint| checkpoint.state_id == state_id)
        .copied()
        .ok_or(StoreError::CheckpointNotFound)?;
    let device = store.device.ok_or(StoreError::Unavailable)?;
    let mut snapshot = read_at(
        device,
        checkpoint_lba(checkpoint.slot as usize),
        checkpoint.slot,
        store.storage_key.as_ref(),
    )?
    .snapshot;
    if let Some(current) = store.snapshot {
        snapshot.journal_sequence = current.journal_sequence.wrapping_add(1).max(1);
    }
    let generation = commit_locked(&mut store, snapshot)?;
    slog!(
        "EXPOS_EXPFS_RESTORE state={} generation={} sequence={}\r\n",
        state_id,
        generation,
        snapshot.journal_sequence
    );
    Ok(generation)
}

pub fn commit(snapshot: Snapshot) -> Result<u32, StoreError> {
    let mut store = STORE.lock();
    let mut snapshot = snapshot;
    snapshot.form_graph_present = true;
    if let Some(current) = store.snapshot {
        snapshot.settings = current.settings;
        snapshot.accounts = current.accounts;
        if snapshot.journal_sequence <= current.journal_sequence {
            snapshot.journal_sequence = current.journal_sequence.wrapping_add(1).max(1);
        }
    }
    validate_snapshot(&snapshot)?;
    if snapshot.cfc != store.cfc {
        return Err(StoreError::WrongCfc);
    }
    commit_locked(&mut store, snapshot)?;
    Ok(snapshot.journal_sequence)
}

pub fn commit_system_records(
    settings: [u8; SETTINGS_RECORD_CAPACITY],
    accounts: Option<[u8; ACCOUNTS_RECORD_CAPACITY]>,
) -> Result<u64, StoreError> {
    let mut store = STORE.lock();
    if store.cfc.is_zero() || store.primary_dimension.is_zero() {
        return Err(StoreError::Unavailable);
    }
    let mut snapshot = store
        .snapshot
        .unwrap_or_else(|| Snapshot::empty(store.cfc, store.cfc_name, store.primary_dimension));
    snapshot.settings = Some(settings);
    snapshot.accounts = accounts;
    snapshot.journal_sequence = snapshot.journal_sequence.wrapping_add(1).max(1);
    validate_snapshot(&snapshot)?;
    commit_locked(&mut store, snapshot)
}

fn commit_locked(store: &mut RuntimeStore, snapshot: Snapshot) -> Result<u64, StoreError> {
    let device = store.device.ok_or(StoreError::Unavailable)?;
    let generation = store.generation.wrapping_add(1).max(1);
    let target_slot = store.active_slot ^ 1;
    let base = slot_lba(target_slot);
    let mut encoded = [0_u8; SLOT_LEN];
    encode_slot(
        &snapshot,
        generation,
        store.storage_key.as_ref(),
        base,
        &mut encoded,
    )?;
    write_at(device, base, &encoded)?;
    let verified = read_at(device, base, target_slot, store.storage_key.as_ref()).map_err(
        |error| match error {
            StoreError::Disk(error) => StoreError::Disk(error),
            _ => StoreError::VerificationFailed,
        },
    )?;
    if verified.generation != generation
        || verified.snapshot.cfc != snapshot.cfc
        || verified.snapshot.journal_sequence != snapshot.journal_sequence
    {
        return Err(StoreError::VerificationFailed);
    }
    store.snapshot = Some(snapshot);
    store.generation = generation;
    store.active_slot = target_slot;
    slog!(
        "EXPOS_EXPFS_COMMIT generation={} sequence={} slot={}\r\n",
        generation,
        snapshot.journal_sequence,
        if target_slot == 0 { "A" } else { "B" }
    );
    Ok(generation)
}

fn write_at(
    device: storage::Device,
    base: u32,
    encoded: &[u8; SLOT_LEN],
) -> Result<(), StoreError> {
    for write_index in 0..SLOT_SECTORS {
        let sector_index = (write_index + 1) % SLOT_SECTORS;
        let mut sector = [0_u8; storage::SECTOR_SIZE];
        let start = sector_index * storage::SECTOR_SIZE;
        sector.copy_from_slice(&encoded[start..start + storage::SECTOR_SIZE]);
        device
            .write_sector(base + sector_index as u32, &sector)
            .map_err(StoreError::Disk)?;
    }
    Ok(())
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<(), StoreError> {
    if snapshot.cfc.is_zero()
        || snapshot.cfc_name.as_str().is_empty()
        || snapshot.primary_dimension.is_zero()
        || snapshot.journal_sequence == 0
        || snapshot.next_fin == 0
        || snapshot.next_dimension_fin == 0
    {
        return Err(StoreError::InvalidSnapshot);
    }
    for stored in snapshot.forms.iter().flatten() {
        if stored.form.fin.is_zero()
            || stored.form.name.as_str().is_empty()
            || stored.form.revision == 0
            || stored.content_len as usize > FORM_CONTENT_CAPACITY
        {
            return Err(StoreError::InvalidSnapshot);
        }
    }
    for dimension in snapshot.dimensions.iter().flatten() {
        if dimension.fin.is_zero() || dimension.name.as_str().is_empty() {
            return Err(StoreError::InvalidSnapshot);
        }
    }
    for (index, handle) in snapshot.handles.iter().flatten().enumerate() {
        if handle.id == 0
            || handle.cfc != snapshot.cfc
            || handle.requester.is_zero()
            || handle.target.is_zero()
            || handle.dimension.is_zero()
            || handle.operations == Operations::NONE
            || snapshot
                .handles
                .iter()
                .flatten()
                .skip(index + 1)
                .any(|candidate| candidate.id == handle.id)
        {
            return Err(StoreError::InvalidSnapshot);
        }
    }
    Ok(())
}

fn encode_slot(
    snapshot: &Snapshot,
    generation: u64,
    storage_key: Option<&[u8; crypto::STORAGE_KEY_LEN]>,
    nonce_domain: u32,
    output: &mut [u8; SLOT_LEN],
) -> Result<(), StoreError> {
    output.fill(0);
    let mut offset = HEADER_LEN;
    put(output, &mut offset, &snapshot.primary_dimension.bytes())?;
    put_text(output, &mut offset, snapshot.cfc_name)?;
    put_u32_cursor(output, &mut offset, snapshot.next_fin)?;
    put_u32_cursor(output, &mut offset, snapshot.next_dimension_fin)?;
    put_u32_cursor(output, &mut offset, snapshot.journal_sequence)?;
    put_byte(output, &mut offset, encode_network(snapshot.network_policy))?;
    put_byte(output, &mut offset, u8::from(snapshot.form_graph_present))?;
    put_byte(output, &mut offset, u8::from(snapshot.settings.is_some()))?;
    put(
        output,
        &mut offset,
        &snapshot.settings.unwrap_or([0; SETTINGS_RECORD_CAPACITY]),
    )?;
    put_byte(output, &mut offset, u8::from(snapshot.accounts.is_some()))?;
    put(
        output,
        &mut offset,
        &snapshot.accounts.unwrap_or([0; ACCOUNTS_RECORD_CAPACITY]),
    )?;

    for slot in snapshot.dimensions {
        put_byte(output, &mut offset, u8::from(slot.is_some()))?;
        if let Some(dimension) = slot {
            put(output, &mut offset, &dimension.fin.bytes())?;
            put_text(output, &mut offset, dimension.name)?;
            put_byte(output, &mut offset, u8::from(dimension.security_boundary))?;
        } else {
            offset += 16 + 1 + 32 + 1;
        }
    }
    for slot in snapshot.forms {
        put_byte(output, &mut offset, u8::from(slot.is_some()))?;
        if let Some(stored) = slot {
            put(output, &mut offset, &stored.form.fin.bytes())?;
            put_text(output, &mut offset, stored.form.name)?;
            put_byte(output, &mut offset, encode_kind(stored.form.kind))?;
            put_u32_cursor(output, &mut offset, stored.form.revision)?;
            put_byte(output, &mut offset, encode_lifecycle(stored.form.lifecycle))?;
            put_u16_cursor(output, &mut offset, stored.content_len)?;
            put(output, &mut offset, &stored.content)?;
        } else {
            offset += 16 + 1 + 32 + 1 + 4 + 1 + 2 + FORM_CONTENT_CAPACITY;
        }
    }
    for slot in snapshot.relationships {
        put_byte(output, &mut offset, u8::from(slot.is_some()))?;
        if let Some(relationship) = slot {
            put(output, &mut offset, &relationship.source.bytes())?;
            put(output, &mut offset, &relationship.target.bytes())?;
            put_byte(output, &mut offset, encode_relationship(relationship.kind))?;
            put_byte(
                output,
                &mut offset,
                u8::from(relationship.dimension.is_some()),
            )?;
            put(
                output,
                &mut offset,
                &relationship.dimension.unwrap_or(Fin::ZERO).bytes(),
            )?;
        } else {
            offset += 16 + 16 + 1 + 1 + 16;
        }
    }
    for slot in snapshot.handles {
        put_byte(output, &mut offset, u8::from(slot.is_some()))?;
        if let Some(handle) = slot {
            put_u32_cursor(output, &mut offset, handle.id)?;
            put_u32_cursor(output, &mut offset, handle.parent_id)?;
            put(output, &mut offset, &handle.requester.bytes())?;
            put(output, &mut offset, &handle.target.bytes())?;
            put(output, &mut offset, &handle.dimension.bytes())?;
            put_u16_cursor(output, &mut offset, handle.operations.bits())?;
            put_u64_cursor(output, &mut offset, handle.valid_until_tick)?;
            put_byte(output, &mut offset, u8::from(handle.revoked))?;
        } else {
            offset += 4 + 4 + 16 + 16 + 16 + 2 + 8 + 1;
        }
    }
    if offset > SLOT_LEN {
        return Err(StoreError::InvalidSnapshot);
    }

    output[..8].copy_from_slice(&MAGIC);
    put_u16(output, 8, FORMAT_VERSION);
    put_u16(output, 10, HEADER_LEN as u16);
    put_u32(output, 12, PAYLOAD_LEN as u32);
    put_u64(output, 16, generation);
    output[24..40].copy_from_slice(&snapshot.cfc.bytes());
    put_u32(output, 40, snapshot.journal_sequence);
    if let Some(key) = storage_key {
        output[44] = 1;
        output[45] = 1; // XChaCha20-Poly1305
        let nonce = crypto::storage_nonce(key, &snapshot.cfc.bytes(), generation, nonce_domain);
        let (header, payload) = output.split_at_mut(HEADER_LEN);
        let tag = crypto::seal_storage(key, &nonce, &header[..48], payload)
            .map_err(|_| StoreError::VerificationFailed)?;
        header[48..64].copy_from_slice(&tag);
    } else {
        let payload_crc = crc32(&output[HEADER_LEN..]);
        put_u32(output, 48, payload_crc);
        let header_crc = crc32(&output[..52]);
        put_u32(output, 52, header_crc);
    }
    Ok(())
}

fn read_slot(
    device: storage::Device,
    slot: u8,
    storage_key: Option<&[u8; crypto::STORAGE_KEY_LEN]>,
) -> Result<DecodedSlot, StoreError> {
    read_at(device, slot_lba(slot), slot, storage_key)
}

fn read_at(
    device: storage::Device,
    base: u32,
    slot: u8,
    storage_key: Option<&[u8; crypto::STORAGE_KEY_LEN]>,
) -> Result<DecodedSlot, StoreError> {
    let mut encoded = [0_u8; SLOT_LEN];
    for sector_index in 0..SLOT_SECTORS {
        let mut sector = [0_u8; storage::SECTOR_SIZE];
        device
            .read_sector(base + sector_index as u32, &mut sector)
            .map_err(StoreError::Disk)?;
        let start = sector_index * storage::SECTOR_SIZE;
        encoded[start..start + storage::SECTOR_SIZE].copy_from_slice(&sector);
    }
    decode_slot(&encoded, slot, storage_key, base).ok_or(StoreError::VerificationFailed)
}

fn region_is_blank(device: storage::Device, base: u32) -> bool {
    for sector_index in 0..SLOT_SECTORS {
        let mut sector = [0_u8; storage::SECTOR_SIZE];
        if device
            .read_sector(base + sector_index as u32, &mut sector)
            .is_err()
            || sector.iter().any(|byte| *byte != 0)
        {
            return false;
        }
    }
    true
}

fn decode_slot(
    encoded: &[u8; SLOT_LEN],
    slot: u8,
    storage_key: Option<&[u8; crypto::STORAGE_KEY_LEN]>,
    nonce_domain: u32,
) -> Option<DecodedSlot> {
    let version = get_u16(encoded, 8);
    if encoded[..8] != MAGIC
        || !(1..=FORMAT_VERSION).contains(&version)
        || get_u16(encoded, 10) as usize != HEADER_LEN
        || get_u32(encoded, 12) as usize != PAYLOAD_LEN
    {
        return None;
    }
    let encrypted = version >= 2 && encoded[44] == 1;
    let mut decoded = *encoded;
    if encrypted {
        if encoded[45] != 1 {
            return None;
        }
        let key = storage_key?;
        let generation = get_u64(encoded, 16);
        let cfc_bytes: [u8; 16] = encoded[24..40].try_into().ok()?;
        let nonce = crypto::storage_nonce(key, &cfc_bytes, generation, nonce_domain);
        let tag: [u8; crypto::STORAGE_TAG_LEN] = encoded[48..64].try_into().ok()?;
        let (header, payload) = decoded.split_at_mut(HEADER_LEN);
        crypto::open_storage(key, &nonce, &header[..48], payload, &tag).ok()?;
    } else if crc32(&encoded[..52]) != get_u32(encoded, 52)
        || crc32(&encoded[HEADER_LEN..]) != get_u32(encoded, 48)
    {
        return None;
    }
    let input = &decoded;
    let generation = get_u64(input, 16);
    if generation == 0 {
        return None;
    }
    let cfc = CfcFin::from_u128(u128::from_be_bytes(input[24..40].try_into().ok()?));
    let mut offset = HEADER_LEN;
    let primary_dimension = read_fin(input, &mut offset)?;
    let cfc_name = read_text(input, &mut offset)?;
    let next_fin = read_u32_cursor(input, &mut offset)?;
    let next_dimension_fin = read_u32_cursor(input, &mut offset)?;
    let journal_sequence = read_u32_cursor(input, &mut offset)?;
    if journal_sequence != get_u32(input, 40) {
        return None;
    }
    let network_policy = decode_network(read_byte(input, &mut offset)?)?;
    let form_graph_present = match read_byte(input, &mut offset)? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let settings_present = read_byte(input, &mut offset)?;
    let mut settings = [0_u8; SETTINGS_RECORD_CAPACITY];
    settings.copy_from_slice(read(input, &mut offset, SETTINGS_RECORD_CAPACITY)?);
    let accounts_present = read_byte(input, &mut offset)?;
    let mut accounts = [0_u8; ACCOUNTS_RECORD_CAPACITY];
    accounts.copy_from_slice(read(input, &mut offset, ACCOUNTS_RECORD_CAPACITY)?);
    if settings_present > 1 || accounts_present > 1 {
        return None;
    }
    let mut snapshot = Snapshot::empty(cfc, cfc_name, primary_dimension);
    snapshot.next_fin = next_fin;
    snapshot.next_dimension_fin = next_dimension_fin;
    snapshot.journal_sequence = journal_sequence;
    snapshot.network_policy = network_policy;
    snapshot.form_graph_present = form_graph_present;
    snapshot.settings = (settings_present == 1).then_some(settings);
    snapshot.accounts = (accounts_present == 1).then_some(accounts);

    for target in &mut snapshot.dimensions {
        let present = read_byte(input, &mut offset)?;
        if present == 0 {
            offset += 16 + 1 + 32 + 1;
            continue;
        }
        if present != 1 {
            return None;
        }
        let fin = read_fin(input, &mut offset)?;
        let name = read_text(input, &mut offset)?;
        let security_boundary = match read_byte(input, &mut offset)? {
            0 => false,
            1 => true,
            _ => return None,
        };
        *target = Some(Dimension::new(fin, name.as_str(), security_boundary));
    }
    for target in &mut snapshot.forms {
        let present = read_byte(input, &mut offset)?;
        if present == 0 {
            offset += 16 + 1 + 32 + 1 + 4 + 1 + 2 + FORM_CONTENT_CAPACITY;
            continue;
        }
        if present != 1 {
            return None;
        }
        let fin = read_fin(input, &mut offset)?;
        let name = read_text(input, &mut offset)?;
        let kind = decode_kind(read_byte(input, &mut offset)?)?;
        let revision = read_u32_cursor(input, &mut offset)?;
        let lifecycle = decode_lifecycle(read_byte(input, &mut offset)?)?;
        let content_len = read_u16_cursor(input, &mut offset)?;
        if content_len as usize > FORM_CONTENT_CAPACITY {
            return None;
        }
        let mut content = [0_u8; FORM_CONTENT_CAPACITY];
        content.copy_from_slice(read(input, &mut offset, FORM_CONTENT_CAPACITY)?);
        let mut form = Form::new(fin, name.as_str(), kind);
        form.revision = revision;
        form.lifecycle = lifecycle;
        *target = Some(StoredForm {
            form,
            content,
            content_len,
        });
    }
    for target in &mut snapshot.relationships {
        let present = read_byte(input, &mut offset)?;
        if present == 0 {
            offset += 16 + 16 + 1 + 1 + 16;
            continue;
        }
        if present != 1 {
            return None;
        }
        let source = read_fin(input, &mut offset)?;
        let destination = read_fin(input, &mut offset)?;
        let kind = decode_relationship(read_byte(input, &mut offset)?)?;
        let dimension_present = read_byte(input, &mut offset)?;
        let dimension = read_fin(input, &mut offset)?;
        *target = Some(Relationship {
            source,
            target: destination,
            kind,
            dimension: match dimension_present {
                0 if dimension.is_zero() => None,
                1 if !dimension.is_zero() => Some(dimension),
                _ => return None,
            },
        });
    }
    for target in &mut snapshot.handles {
        let present = read_byte(input, &mut offset)?;
        if present == 0 {
            offset += 4 + 4 + 16 + 16 + 16 + 2 + 8 + 1;
            continue;
        }
        if present != 1 {
            return None;
        }
        let id = read_u32_cursor(input, &mut offset)?;
        let parent_id = read_u32_cursor(input, &mut offset)?;
        let requester = read_fin(input, &mut offset)?;
        let target_fin = read_fin(input, &mut offset)?;
        let dimension = read_fin(input, &mut offset)?;
        let operations = Operations::from_bits(read_u16_cursor(input, &mut offset)?)?;
        let valid_until_tick = read_u64_cursor(input, &mut offset)?;
        let revoked = match read_byte(input, &mut offset)? {
            0 => false,
            1 => true,
            _ => return None,
        };
        *target = Some(FormHandle {
            id,
            parent_id,
            cfc,
            requester,
            target: target_fin,
            dimension,
            operations,
            valid_until_tick,
            revoked,
        });
    }
    validate_snapshot(&snapshot).ok()?;
    Some(DecodedSlot {
        snapshot,
        generation,
        slot,
    })
}

fn newest(first: Option<DecodedSlot>, second: Option<DecodedSlot>) -> Option<DecodedSlot> {
    match (first, second) {
        (Some(first), Some(second)) => {
            if second.generation != first.generation
                && second.generation.wrapping_sub(first.generation) < (1_u64 << 63)
            {
                Some(second)
            } else {
                Some(first)
            }
        }
        (Some(slot), None) | (None, Some(slot)) => Some(slot),
        (None, None) => None,
    }
}

const fn slot_lba(slot: u8) -> u32 {
    if slot == 0 {
        SLOT_A_LBA
    } else {
        SLOT_B_LBA
    }
}

const fn checkpoint_lba(slot: usize) -> u32 {
    CHECKPOINT_LBA + (slot * SLOT_SECTORS) as u32
}

fn put(output: &mut [u8], offset: &mut usize, bytes: &[u8]) -> Result<(), StoreError> {
    let end = offset
        .checked_add(bytes.len())
        .ok_or(StoreError::InvalidSnapshot)?;
    let target = output
        .get_mut(*offset..end)
        .ok_or(StoreError::InvalidSnapshot)?;
    target.copy_from_slice(bytes);
    *offset = end;
    Ok(())
}

fn put_byte(output: &mut [u8], offset: &mut usize, value: u8) -> Result<(), StoreError> {
    put(output, offset, &[value])
}

fn put_text(output: &mut [u8], offset: &mut usize, value: Text) -> Result<(), StoreError> {
    put_byte(output, offset, value.as_str().len() as u8)?;
    let mut bytes = [0_u8; 32];
    bytes[..value.as_str().len()].copy_from_slice(value.as_str().as_bytes());
    put(output, offset, &bytes)
}

fn put_u16_cursor(output: &mut [u8], offset: &mut usize, value: u16) -> Result<(), StoreError> {
    put(output, offset, &value.to_le_bytes())
}

fn put_u32_cursor(output: &mut [u8], offset: &mut usize, value: u32) -> Result<(), StoreError> {
    put(output, offset, &value.to_le_bytes())
}

fn put_u64_cursor(output: &mut [u8], offset: &mut usize, value: u64) -> Result<(), StoreError> {
    put(output, offset, &value.to_le_bytes())
}

fn read<'a>(input: &'a [u8], offset: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = offset.checked_add(len)?;
    let value = input.get(*offset..end)?;
    *offset = end;
    Some(value)
}

fn read_byte(input: &[u8], offset: &mut usize) -> Option<u8> {
    Some(read(input, offset, 1)?[0])
}

fn read_u16_cursor(input: &[u8], offset: &mut usize) -> Option<u16> {
    Some(u16::from_le_bytes(read(input, offset, 2)?.try_into().ok()?))
}

fn read_u32_cursor(input: &[u8], offset: &mut usize) -> Option<u32> {
    Some(u32::from_le_bytes(read(input, offset, 4)?.try_into().ok()?))
}

fn read_u64_cursor(input: &[u8], offset: &mut usize) -> Option<u64> {
    Some(u64::from_le_bytes(read(input, offset, 8)?.try_into().ok()?))
}

fn read_fin(input: &[u8], offset: &mut usize) -> Option<Fin> {
    Some(Fin::from_u128(u128::from_be_bytes(
        read(input, offset, 16)?.try_into().ok()?,
    )))
}

fn read_text(input: &[u8], offset: &mut usize) -> Option<Text> {
    let len = read_byte(input, offset)? as usize;
    let bytes = read(input, offset, 32)?;
    if len == 0 || len > 32 || !bytes[..len].is_ascii() {
        return None;
    }
    Text::new(core::str::from_utf8(&bytes[..len]).ok()?).ok()
}

fn encode_kind(value: FormKind) -> u8 {
    match value {
        FormKind::Root => 0,
        FormKind::Service => 1,
        FormKind::Interface => 2,
        FormKind::Package => 3,
        FormKind::Driver => 4,
        FormKind::Data => 5,
        FormKind::Policy => 6,
        FormKind::Executable => 7,
    }
}

fn decode_kind(value: u8) -> Option<FormKind> {
    Some(match value {
        0 => FormKind::Root,
        1 => FormKind::Service,
        2 => FormKind::Interface,
        3 => FormKind::Package,
        4 => FormKind::Driver,
        5 => FormKind::Data,
        6 => FormKind::Policy,
        7 => FormKind::Executable,
        _ => return None,
    })
}

fn encode_lifecycle(value: Lifecycle) -> u8 {
    match value {
        Lifecycle::Active => 0,
        Lifecycle::Retired => 1,
        Lifecycle::Recoverable => 2,
        Lifecycle::Removed => 3,
    }
}

fn decode_lifecycle(value: u8) -> Option<Lifecycle> {
    Some(match value {
        0 => Lifecycle::Active,
        1 => Lifecycle::Retired,
        2 => Lifecycle::Recoverable,
        3 => Lifecycle::Removed,
        _ => return None,
    })
}

fn encode_relationship(value: RelationshipKind) -> u8 {
    match value {
        RelationshipKind::DependsOn => 0,
        RelationshipKind::Provides => 1,
        RelationshipKind::Contains => 2,
        RelationshipKind::ConfiguredBy => 3,
        RelationshipKind::Revises => 4,
    }
}

fn decode_relationship(value: u8) -> Option<RelationshipKind> {
    Some(match value {
        0 => RelationshipKind::DependsOn,
        1 => RelationshipKind::Provides,
        2 => RelationshipKind::Contains,
        3 => RelationshipKind::ConfiguredBy,
        4 => RelationshipKind::Revises,
        _ => return None,
    })
}

fn encode_network(value: NetworkPolicy) -> u8 {
    match value {
        NetworkPolicy::Open => 0,
        NetworkPolicy::Restricted => 1,
        NetworkPolicy::Disabled => 2,
    }
}

fn decode_network(value: u8) -> Option<NetworkPolicy> {
    Some(match value {
        0 => NetworkPolicy::Open,
        1 => NetworkPolicy::Restricted,
        2 => NetworkPolicy::Disabled,
        _ => return None,
    })
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(input[offset..offset + 2].try_into().unwrap())
}

fn get_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(input[offset..offset + 4].try_into().unwrap())
}

fn get_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(input[offset..offset + 8].try_into().unwrap())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        let cfc = CfcFin::from_u128(100);
        let dimension = Fin::from_u128(2);
        let form_fin = Fin::from_u128(3);
        let mut snapshot = Snapshot::empty(cfc, Text::new("Test CFC").unwrap(), dimension);
        snapshot.journal_sequence = 7;
        snapshot.next_fin = 9;
        snapshot.dimensions[0] = Some(Dimension::new(dimension, "Primary", true));
        let mut content = [0_u8; FORM_CONTENT_CAPACITY];
        content[..5].copy_from_slice(b"hello");
        snapshot.forms[0] = Some(StoredForm {
            form: Form::new(form_fin, "MyNotes", FormKind::Data),
            content,
            content_len: 5,
        });
        snapshot.relationships[0] = Some(Relationship {
            source: Fin::from_u128(4),
            target: form_fin,
            kind: RelationshipKind::Contains,
            dimension: Some(dimension),
        });
        snapshot.handles[0] = Some(FormHandle {
            id: 3,
            parent_id: 0,
            cfc,
            requester: Fin::from_u128(4),
            target: form_fin,
            dimension,
            operations: Operations::EXECUTE,
            valid_until_tick: u64::MAX,
            revoked: false,
        });
        snapshot
    }

    #[test]
    fn disk_snapshot_round_trips_form_graph_and_content() {
        let source = snapshot();
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&source, 11, None, 0, &mut encoded).unwrap();
        let decoded = decode_slot(&encoded, 0, None, 0).unwrap();
        assert_eq!(decoded.generation, 11);
        assert_eq!(decoded.snapshot.cfc, source.cfc);
        assert_eq!(decoded.snapshot.journal_sequence, 7);
        let stored = decoded.snapshot.forms[0].unwrap();
        assert_eq!(stored.form.name.as_str(), "MyNotes");
        assert_eq!(&stored.content[..stored.content_len as usize], b"hello");
        assert_eq!(decoded.snapshot.relationships[0], source.relationships[0]);
        assert_eq!(decoded.snapshot.handles[0], source.handles[0]);
    }

    #[test]
    fn corruption_invalidates_the_snapshot() {
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&snapshot(), 1, None, 0, &mut encoded).unwrap();
        encoded[HEADER_LEN + 700] ^= 0x80;
        assert!(decode_slot(&encoded, 0, None, 0).is_none());
    }

    #[test]
    fn encrypted_snapshot_requires_key_and_authenticates_header_payload_and_domain() {
        let source = snapshot();
        let key = [0x42_u8; crypto::STORAGE_KEY_LEN];
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&source, 12, Some(&key), 64, &mut encoded).unwrap();
        assert_eq!(encoded[44], 1);
        assert_ne!(
            &encoded[HEADER_LEN..HEADER_LEN + 16],
            &source.primary_dimension.bytes()
        );
        assert!(decode_slot(&encoded, 0, None, 64).is_none());
        assert!(decode_slot(&encoded, 0, Some(&[1; crypto::STORAGE_KEY_LEN]), 64).is_none());
        assert!(decode_slot(&encoded, 0, Some(&key), 65).is_none());
        let decoded = decode_slot(&encoded, 0, Some(&key), 64).unwrap();
        assert_eq!(decoded.snapshot.cfc, source.cfc);
        encoded[HEADER_LEN + 9] ^= 1;
        assert!(decode_slot(&encoded, 0, Some(&key), 64).is_none());
    }

    #[test]
    fn protected_baseline_is_outside_checkpoint_rotation_and_has_its_own_aead_domain() {
        assert_eq!(BASELINE_LBA, checkpoint_lba(CHECKPOINT_CAPACITY));
        let key = [0x37_u8; crypto::STORAGE_KEY_LEN];
        let mut encoded = [0_u8; SLOT_LEN];
        encode_slot(&snapshot(), 3, Some(&key), BASELINE_LBA, &mut encoded).unwrap();
        assert!(decode_slot(&encoded, u8::MAX, Some(&key), BASELINE_LBA).is_some());
        assert!(decode_slot(&encoded, u8::MAX, Some(&key), checkpoint_lba(0)).is_none());
    }
}
