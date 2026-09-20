//! Allocation-free CFC recovery metadata.
//!
//! This module records state identity and ExpFS sequencing only. It does not
//! encrypt, authenticate, hash, copy, or restore storage contents. Storage and
//! cryptographic layers must complete their own work before publishing one of
//! these metadata entries.

use crate::cfc::{CfcFin, MAX_CFCS};

/// Number of rotating checkpoints retained in addition to the installation
/// baseline. This is deliberately fixed by the recovery contract.
pub const RECOVERY_CHECKPOINT_CAPACITY: usize = 8;
pub const MAX_RECOVERY_CFCS: usize = MAX_CFCS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryError {
    InvalidCfcFin,
    WrongCfc,
    InvalidMetadata,
    DuplicateCfc,
    UnknownCfc,
    CatalogFull,
    CheckpointNotNewer,
    StaleJournalSequence,
    StaleStateRevision,
    CheckpointNotFound,
}

/// The small, storage-independent description shared by baselines,
/// checkpoints and restore selections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryMetadata {
    state_id: u64,
    expfs_sequence: u32,
    form_graph_revision: u32,
}

impl RecoveryMetadata {
    fn new(
        state_id: u64,
        expfs_sequence: u32,
        form_graph_revision: u32,
    ) -> Result<Self, RecoveryError> {
        if state_id == 0 || expfs_sequence == 0 || form_graph_revision == 0 {
            return Err(RecoveryError::InvalidMetadata);
        }
        Ok(Self {
            state_id,
            expfs_sequence,
            form_graph_revision,
        })
    }

    pub const fn state_id(self) -> u64 {
        self.state_id
    }

    pub const fn expfs_sequence(self) -> u32 {
        self.expfs_sequence
    }

    pub const fn form_graph_revision(self) -> u32 {
        self.form_graph_revision
    }
}

/// Protected installation state. Its distinct type prevents it from entering
/// the rotating-checkpoint publication API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallationBaseline {
    cfc: CfcFin,
    metadata: RecoveryMetadata,
}

impl InstallationBaseline {
    pub fn new(
        cfc: CfcFin,
        state_id: u64,
        expfs_sequence: u32,
        form_graph_revision: u32,
    ) -> Result<Self, RecoveryError> {
        if cfc.is_zero() {
            return Err(RecoveryError::InvalidCfcFin);
        }
        Ok(Self {
            cfc,
            metadata: RecoveryMetadata::new(state_id, expfs_sequence, form_graph_revision)?,
        })
    }

    pub const fn cfc(self) -> CfcFin {
        self.cfc
    }

    pub const fn metadata(self) -> RecoveryMetadata {
        self.metadata
    }
}

/// One immutable checkpoint description. Publication copies this value into a
/// ring slot; retained entries have no update operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointMetadata {
    cfc: CfcFin,
    metadata: RecoveryMetadata,
}

impl CheckpointMetadata {
    pub fn new(
        cfc: CfcFin,
        state_id: u64,
        expfs_sequence: u32,
        form_graph_revision: u32,
    ) -> Result<Self, RecoveryError> {
        if cfc.is_zero() {
            return Err(RecoveryError::InvalidCfcFin);
        }
        Ok(Self {
            cfc,
            metadata: RecoveryMetadata::new(state_id, expfs_sequence, form_graph_revision)?,
        })
    }

    pub const fn cfc(self) -> CfcFin {
        self.cfc
    }

    pub const fn metadata(self) -> RecoveryMetadata {
        self.metadata
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreTarget {
    InstallationBaseline,
    Checkpoint(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreSource {
    InstallationBaseline,
    Checkpoint,
}

/// A non-destructive choice returned to the layer that will perform a restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestoreSelection {
    pub cfc: CfcFin,
    pub source: RestoreSource,
    pub metadata: RecoveryMetadata,
}

/// Recovery history for exactly one CFC.
///
/// The baseline lives outside the checkpoint ring and is never an eviction
/// candidate. Checkpoint mutation validates fully before touching ring state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CfcRecovery {
    cfc: CfcFin,
    baseline: InstallationBaseline,
    checkpoints: [Option<CheckpointMetadata>; RECOVERY_CHECKPOINT_CAPACITY],
    oldest: u8,
    len: u8,
}

impl CfcRecovery {
    pub fn new(cfc: CfcFin, baseline: InstallationBaseline) -> Result<Self, RecoveryError> {
        if cfc.is_zero() {
            return Err(RecoveryError::InvalidCfcFin);
        }
        if baseline.cfc() != cfc {
            return Err(RecoveryError::WrongCfc);
        }
        Ok(Self {
            cfc,
            baseline,
            checkpoints: [None; RECOVERY_CHECKPOINT_CAPACITY],
            oldest: 0,
            len: 0,
        })
    }

    pub const fn cfc(&self) -> CfcFin {
        self.cfc
    }

    pub const fn baseline(&self) -> InstallationBaseline {
        self.baseline
    }

    pub const fn checkpoint_count(&self) -> usize {
        self.len as usize
    }

    /// Return a checkpoint by age: index zero is the oldest retained entry.
    pub fn checkpoint(&self, index: usize) -> Option<CheckpointMetadata> {
        if index >= self.len as usize {
            return None;
        }
        let slot = (self.oldest as usize + index) % RECOVERY_CHECKPOINT_CAPACITY;
        self.checkpoints[slot]
    }

    pub fn latest_checkpoint(&self) -> Option<CheckpointMetadata> {
        self.checkpoint(self.len.saturating_sub(1) as usize)
    }

    /// Atomically publish one validated checkpoint.
    ///
    /// Once eight entries exist, the oldest checkpoint is returned and its
    /// slot is reused. The separately stored installation baseline is not
    /// consulted as an eviction candidate and cannot be overwritten here.
    pub fn record_checkpoint(
        &mut self,
        checkpoint: CheckpointMetadata,
    ) -> Result<Option<CheckpointMetadata>, RecoveryError> {
        if checkpoint.cfc() != self.cfc {
            return Err(RecoveryError::WrongCfc);
        }
        self.validate_successor(checkpoint)?;

        if (self.len as usize) < RECOVERY_CHECKPOINT_CAPACITY {
            let slot = (self.oldest as usize + self.len as usize) % RECOVERY_CHECKPOINT_CAPACITY;
            self.checkpoints[slot] = Some(checkpoint);
            self.len += 1;
            return Ok(None);
        }

        let slot = self.oldest as usize;
        let evicted = self.checkpoints[slot].replace(checkpoint);
        self.oldest = ((slot + 1) % RECOVERY_CHECKPOINT_CAPACITY) as u8;
        Ok(evicted)
    }

    /// Publish a sequence as one transaction. A failure at any point leaves
    /// the original ring, ordering and baseline unchanged.
    pub fn record_batch(
        &mut self,
        checkpoints: &[CheckpointMetadata],
    ) -> Result<(), RecoveryError> {
        let mut candidate = *self;
        for checkpoint in checkpoints {
            candidate.record_checkpoint(*checkpoint)?;
        }
        *self = candidate;
        Ok(())
    }

    /// Choose a retained state without changing or consuming recovery data.
    pub fn select_restore(&self, target: RestoreTarget) -> Result<RestoreSelection, RecoveryError> {
        match target {
            RestoreTarget::InstallationBaseline => Ok(RestoreSelection {
                cfc: self.cfc,
                source: RestoreSource::InstallationBaseline,
                metadata: self.baseline.metadata(),
            }),
            RestoreTarget::Checkpoint(state_id) => self
                .checkpoints
                .iter()
                .flatten()
                .copied()
                .find(|checkpoint| checkpoint.metadata().state_id() == state_id)
                .map(|checkpoint| RestoreSelection {
                    cfc: self.cfc,
                    source: RestoreSource::Checkpoint,
                    metadata: checkpoint.metadata(),
                })
                .ok_or(RecoveryError::CheckpointNotFound),
        }
    }

    fn latest_metadata(&self) -> RecoveryMetadata {
        self.latest_checkpoint()
            .map(CheckpointMetadata::metadata)
            .unwrap_or_else(|| self.baseline.metadata())
    }

    fn validate_successor(&self, checkpoint: CheckpointMetadata) -> Result<(), RecoveryError> {
        let latest = self.latest_metadata();
        let candidate = checkpoint.metadata();
        if candidate.state_id() <= latest.state_id() {
            return Err(RecoveryError::CheckpointNotNewer);
        }
        if candidate.expfs_sequence() < latest.expfs_sequence() {
            return Err(RecoveryError::StaleJournalSequence);
        }
        if candidate.form_graph_revision() < latest.form_graph_revision() {
            return Err(RecoveryError::StaleStateRevision);
        }
        Ok(())
    }
}

/// Fixed-capacity collection of independent per-CFC recovery histories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryCatalog {
    histories: [Option<CfcRecovery>; MAX_RECOVERY_CFCS],
}

impl RecoveryCatalog {
    pub const fn new() -> Self {
        Self {
            histories: [None; MAX_RECOVERY_CFCS],
        }
    }

    pub fn install(
        &mut self,
        cfc: CfcFin,
        baseline: InstallationBaseline,
    ) -> Result<(), RecoveryError> {
        if self.recovery(cfc).is_some() {
            return Err(RecoveryError::DuplicateCfc);
        }
        let history = CfcRecovery::new(cfc, baseline)?;
        let slot = self
            .histories
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RecoveryError::CatalogFull)?;
        *slot = Some(history);
        Ok(())
    }

    pub fn recovery(&self, cfc: CfcFin) -> Option<&CfcRecovery> {
        self.histories
            .iter()
            .flatten()
            .find(|history| history.cfc == cfc)
    }

    pub fn record_checkpoint(
        &mut self,
        cfc: CfcFin,
        checkpoint: CheckpointMetadata,
    ) -> Result<Option<CheckpointMetadata>, RecoveryError> {
        self.histories
            .iter_mut()
            .flatten()
            .find(|history| history.cfc == cfc)
            .ok_or(RecoveryError::UnknownCfc)?
            .record_checkpoint(checkpoint)
    }

    pub fn select_restore(
        &self,
        cfc: CfcFin,
        target: RestoreTarget,
    ) -> Result<RestoreSelection, RecoveryError> {
        self.recovery(cfc)
            .ok_or(RecoveryError::UnknownCfc)?
            .select_restore(target)
    }

    pub fn len(&self) -> usize {
        self.histories.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for RecoveryCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfc(value: u128) -> CfcFin {
        CfcFin::from_u128(value)
    }

    fn baseline(cfc_id: u128, state_id: u64) -> InstallationBaseline {
        InstallationBaseline::new(cfc(cfc_id), state_id, state_id as u32, state_id as u32).unwrap()
    }

    fn checkpoint(cfc_id: u128, state_id: u64) -> CheckpointMetadata {
        CheckpointMetadata::new(cfc(cfc_id), state_id, state_id as u32, state_id as u32).unwrap()
    }

    #[test]
    fn metadata_and_per_cfc_identity_are_validated() {
        assert_eq!(
            InstallationBaseline::new(cfc(1), 0, 1, 1),
            Err(RecoveryError::InvalidMetadata)
        );
        assert_eq!(
            CheckpointMetadata::new(cfc(1), 1, 0, 1),
            Err(RecoveryError::InvalidMetadata)
        );
        assert_eq!(
            CfcRecovery::new(CfcFin::ZERO, baseline(1, 1)),
            Err(RecoveryError::InvalidCfcFin)
        );
        assert_eq!(
            CfcRecovery::new(cfc(2), baseline(1, 1)),
            Err(RecoveryError::WrongCfc)
        );
    }

    #[test]
    fn eight_checkpoints_rotate_oldest_first_without_touching_baseline() {
        assert_eq!(RECOVERY_CHECKPOINT_CAPACITY, 8);
        let protected = baseline(1, 1);
        let mut recovery = CfcRecovery::new(cfc(1), protected).unwrap();
        for state_id in 2..=9 {
            assert_eq!(
                recovery.record_checkpoint(checkpoint(1, state_id)),
                Ok(None)
            );
        }
        assert_eq!(recovery.checkpoint_count(), 8);
        assert_eq!(recovery.checkpoint(0), Some(checkpoint(1, 2)));
        assert_eq!(recovery.checkpoint(7), Some(checkpoint(1, 9)));

        assert_eq!(
            recovery.record_checkpoint(checkpoint(1, 10)),
            Ok(Some(checkpoint(1, 2)))
        );
        assert_eq!(recovery.checkpoint_count(), 8);
        assert_eq!(recovery.checkpoint(0), Some(checkpoint(1, 3)));
        assert_eq!(recovery.checkpoint(7), Some(checkpoint(1, 10)));
        assert_eq!(recovery.baseline(), protected);
    }

    #[test]
    fn failed_single_and_batch_publications_are_atomic() {
        let mut recovery = CfcRecovery::new(cfc(1), baseline(1, 1)).unwrap();
        recovery.record_checkpoint(checkpoint(1, 2)).unwrap();

        let before_single = recovery;
        assert_eq!(
            recovery.record_checkpoint(checkpoint(1, 2)),
            Err(RecoveryError::CheckpointNotNewer)
        );
        assert_eq!(recovery, before_single);

        let before_batch = recovery;
        assert_eq!(
            recovery.record_batch(&[checkpoint(1, 3), checkpoint(1, 2)]),
            Err(RecoveryError::CheckpointNotNewer)
        );
        assert_eq!(recovery, before_batch);

        recovery
            .record_batch(&[checkpoint(1, 3), checkpoint(1, 4)])
            .unwrap();
        assert_eq!(recovery.checkpoint_count(), 3);
        assert_eq!(recovery.latest_checkpoint(), Some(checkpoint(1, 4)));
    }

    #[test]
    fn sequencing_failures_do_not_partially_change_the_ring() {
        let mut recovery = CfcRecovery::new(cfc(1), baseline(1, 10)).unwrap();
        recovery
            .record_checkpoint(CheckpointMetadata::new(cfc(1), 11, 11, 11).unwrap())
            .unwrap();

        let before = recovery;
        assert_eq!(
            recovery.record_checkpoint(CheckpointMetadata::new(cfc(1), 12, 10, 12).unwrap()),
            Err(RecoveryError::StaleJournalSequence)
        );
        assert_eq!(recovery, before);
        assert_eq!(
            recovery.record_checkpoint(CheckpointMetadata::new(cfc(1), 12, 12, 10).unwrap()),
            Err(RecoveryError::StaleStateRevision)
        );
        assert_eq!(recovery, before);
    }

    #[test]
    fn restore_selection_is_non_destructive_and_rotated_states_are_unavailable() {
        let mut recovery = CfcRecovery::new(cfc(7), baseline(7, 1)).unwrap();
        for state_id in 2..=10 {
            recovery.record_checkpoint(checkpoint(7, state_id)).unwrap();
        }
        let before = recovery;

        let selected = recovery
            .select_restore(RestoreTarget::InstallationBaseline)
            .unwrap();
        assert_eq!(selected.cfc, cfc(7));
        assert_eq!(selected.source, RestoreSource::InstallationBaseline);
        assert_eq!(selected.metadata, baseline(7, 1).metadata());
        assert_eq!(recovery, before);

        let selected = recovery
            .select_restore(RestoreTarget::Checkpoint(10))
            .unwrap();
        assert_eq!(selected.source, RestoreSource::Checkpoint);
        assert_eq!(selected.metadata, checkpoint(7, 10).metadata());
        assert_eq!(recovery, before);

        assert_eq!(
            recovery.select_restore(RestoreTarget::Checkpoint(2)),
            Err(RecoveryError::CheckpointNotFound)
        );
        assert_eq!(recovery, before);
    }

    #[test]
    fn recovery_catalog_keeps_histories_separate_per_cfc() {
        let mut catalog = RecoveryCatalog::new();
        catalog.install(cfc(1), baseline(1, 1)).unwrap();
        catalog.install(cfc(2), baseline(2, 20)).unwrap();
        assert_eq!(
            catalog.install(cfc(1), baseline(1, 100)),
            Err(RecoveryError::DuplicateCfc)
        );

        catalog.record_checkpoint(cfc(1), checkpoint(1, 2)).unwrap();
        catalog
            .record_checkpoint(cfc(2), CheckpointMetadata::new(cfc(2), 21, 21, 21).unwrap())
            .unwrap();
        assert_eq!(catalog.recovery(cfc(1)).unwrap().checkpoint_count(), 1);
        assert_eq!(catalog.recovery(cfc(2)).unwrap().checkpoint_count(), 1);
        assert_eq!(
            catalog.record_checkpoint(cfc(3), checkpoint(3, 2)),
            Err(RecoveryError::UnknownCfc)
        );
        assert_eq!(catalog.len(), 2);
    }

    #[test]
    fn recovery_artifacts_cannot_cross_cfc_boundaries() {
        let mut recovery = CfcRecovery::new(cfc(1), baseline(1, 1)).unwrap();
        let before = recovery;
        assert_eq!(
            recovery.record_checkpoint(checkpoint(2, 2)),
            Err(RecoveryError::WrongCfc)
        );
        assert_eq!(recovery, before);
        assert_eq!(
            RecoveryCatalog::new().install(cfc(2), baseline(1, 1)),
            Err(RecoveryError::WrongCfc)
        );
    }
}
