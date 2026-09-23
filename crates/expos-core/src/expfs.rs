use crate::{Cfc, CfcFin, Fin};

const MAX_RECORDS: usize = 32;
const MAX_STAGED: usize = 8;
const MAX_SYSTEM_RECORDS: usize = 64;
const MAX_STAGED_SYSTEM_RECORDS: usize = 16;
pub const FORM_CONTENT_CAPACITY: usize = 512;
pub const SYSTEM_RECORD_CAPACITY: usize = 512;

/// Typed system-database namespaces. They are records in ExpFS rather than
/// side files or subsystem-private persistence formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    Cfc,
    PrimaryDimension,
    Dimension,
    Form,
    Relationship,
    Pimp,
    Capability,
    Account,
    Revision,
    Setting,
    Checkpoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordKey {
    pub kind: RecordKind,
    /// FIN for Form/Dimension-scoped records, or a stable schema-local key.
    pub identity: Fin,
    /// Zero for CFC-global records; otherwise the owning Dimension.
    pub dimension: Fin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemRecord {
    pub cfc: CfcFin,
    pub key: RecordKey,
    pub revision: u32,
    pub journal_sequence: u32,
    bytes: [u8; SYSTEM_RECORD_CAPACITY],
    len: u16,
}

impl SystemRecord {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistentForm {
    pub cfc: CfcFin,
    pub form: Fin,
    pub dimension: Fin,
    pub revision: u32,
    pub journal_sequence: u32,
    content: [u8; FORM_CONTENT_CAPACITY],
    content_len: u16,
}

impl PersistentForm {
    pub fn content(&self) -> &[u8] {
        &self.content[..self.content_len as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    Full,
    EmptyTransaction,
    InvalidRecord,
    ContentTooLarge,
    DuplicateRecord,
    DuplicateKey,
    StaleRevision,
    SequenceExhausted,
    WrongCfc,
    UnownedForm,
    UnownedDimension,
}

pub struct Transaction {
    cfc: Cfc,
    records: [Option<PersistentForm>; MAX_STAGED],
    system_records: [Option<SystemRecord>; MAX_STAGED_SYSTEM_RECORDS],
}

impl Transaction {
    /// Stage any typed piece of CFC state in the same transaction used for
    /// Forms. This is the database path for relationships, policy,
    /// capabilities, accounts, settings, revisions, and checkpoint records.
    pub fn stage_record(
        &mut self,
        key: RecordKey,
        revision: u32,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        if key.identity.is_zero() || revision == 0 {
            return Err(StorageError::InvalidRecord);
        }
        if bytes.len() > SYSTEM_RECORD_CAPACITY {
            return Err(StorageError::ContentTooLarge);
        }
        if !key.dimension.is_zero() && !self.cfc.owns_dimension(key.dimension) {
            return Err(StorageError::UnownedDimension);
        }
        if matches!(
            key.kind,
            RecordKind::Form | RecordKind::Capability | RecordKind::Pimp
        ) && !self.cfc.owns_form(key.identity)
        {
            return Err(StorageError::UnownedForm);
        }
        if self
            .system_records
            .iter()
            .flatten()
            .any(|record| record.key == key)
        {
            return Err(StorageError::DuplicateKey);
        }
        let slot = self
            .system_records
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(StorageError::Full)?;
        let mut payload = [0; SYSTEM_RECORD_CAPACITY];
        payload[..bytes.len()].copy_from_slice(bytes);
        *slot = Some(SystemRecord {
            cfc: self.cfc.fin(),
            key,
            revision,
            journal_sequence: 0,
            bytes: payload,
            len: bytes.len() as u16,
        });
        Ok(())
    }

    pub fn stage_form(
        &mut self,
        form: Fin,
        dimension: Fin,
        revision: u32,
    ) -> Result<(), StorageError> {
        self.stage_content(form, dimension, revision, &[])
    }

    /// Stage identity, revision and its complete content as one atomic record.
    pub fn stage_content(
        &mut self,
        form: Fin,
        dimension: Fin,
        revision: u32,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        if form.is_zero() || dimension.is_zero() || revision == 0 {
            return Err(StorageError::InvalidRecord);
        }
        if !self.cfc.owns_form(form) {
            return Err(StorageError::UnownedForm);
        }
        if !self.cfc.owns_dimension(dimension) {
            return Err(StorageError::UnownedDimension);
        }
        if bytes.len() > FORM_CONTENT_CAPACITY {
            return Err(StorageError::ContentTooLarge);
        }
        if self
            .records
            .iter()
            .flatten()
            .any(|record| record.form == form && record.dimension == dimension)
        {
            return Err(StorageError::DuplicateRecord);
        }
        let slot = self
            .records
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(StorageError::Full)?;
        let mut content = [0; FORM_CONTENT_CAPACITY];
        content[..bytes.len()].copy_from_slice(bytes);
        *slot = Some(PersistentForm {
            cfc: self.cfc.fin(),
            form,
            dimension,
            revision,
            journal_sequence: 0,
            content,
            content_len: bytes.len() as u16,
        });
        Ok(())
    }
}

pub struct ExpFs {
    /// Immutable ownership snapshot for this store. Staging checks the Form
    /// and Dimension against this snapshot before a record can be created.
    cfc: Cfc,
    records: [Option<PersistentForm>; MAX_RECORDS],
    system_records: [Option<SystemRecord>; MAX_SYSTEM_RECORDS],
    next_sequence: u32,
}

impl ExpFs {
    pub fn new(cfc: &Cfc) -> Self {
        cfc.validate().expect("ExpFS requires a valid CFC");
        Self {
            cfc: *cfc,
            records: [None; MAX_RECORDS],
            system_records: [None; MAX_SYSTEM_RECORDS],
            next_sequence: 1,
        }
    }

    pub const fn cfc(&self) -> CfcFin {
        self.cfc.fin()
    }

    pub const fn begin(&self) -> Transaction {
        Transaction {
            cfc: self.cfc,
            records: [None; MAX_STAGED],
            system_records: [None; MAX_STAGED_SYSTEM_RECORDS],
        }
    }

    /// Atomically validates capacity before publishing any staged record.
    pub fn commit(&mut self, transaction: Transaction) -> Result<u32, StorageError> {
        if transaction.cfc != self.cfc {
            return Err(StorageError::WrongCfc);
        }
        let staged_count = transaction.records.iter().flatten().count();
        let staged_system_count = transaction.system_records.iter().flatten().count();
        if staged_count == 0 && staged_system_count == 0 {
            return Err(StorageError::EmptyTransaction);
        }
        let free_count = self.records.iter().filter(|slot| slot.is_none()).count();
        if staged_count > free_count {
            return Err(StorageError::Full);
        }
        let free_system_count = self
            .system_records
            .iter()
            .filter(|slot| slot.is_none())
            .count();
        if staged_system_count > free_system_count {
            return Err(StorageError::Full);
        }
        for record in transaction.records.iter().flatten() {
            if self
                .latest(record.form, record.dimension)
                .is_some_and(|previous| previous.revision >= record.revision)
            {
                return Err(StorageError::StaleRevision);
            }
        }
        for record in transaction.system_records.iter().flatten() {
            if self
                .latest_record(record.key)
                .is_some_and(|previous| previous.revision >= record.revision)
            {
                return Err(StorageError::StaleRevision);
            }
        }
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(StorageError::SequenceExhausted)?;
        for mut record in transaction.records.into_iter().flatten() {
            record.journal_sequence = sequence;
            let slot = self
                .records
                .iter_mut()
                .find(|slot| slot.is_none())
                .expect("capacity preflighted");
            *slot = Some(record);
        }
        for mut record in transaction.system_records.into_iter().flatten() {
            record.journal_sequence = sequence;
            let slot = self
                .system_records
                .iter_mut()
                .find(|slot| slot.is_none())
                .expect("capacity preflighted");
            *slot = Some(record);
        }
        Ok(sequence)
    }

    pub fn latest(&self, form: Fin, dimension: Fin) -> Option<&PersistentForm> {
        self.records
            .iter()
            .flatten()
            .filter(|record| record.form == form && record.dimension == dimension)
            .max_by_key(|record| record.revision)
    }

    pub fn latest_record(&self, key: RecordKey) -> Option<&SystemRecord> {
        self.system_records
            .iter()
            .flatten()
            .filter(|record| record.key == key)
            .max_by_key(|record| record.revision)
    }

    /// Read the newest version of every record no later than a committed
    /// sequence. This is the semantic basis of an ExpFS checkpoint view.
    pub fn records_at(&self, sequence: u32) -> impl Iterator<Item = &SystemRecord> {
        self.system_records.iter().flatten().filter(move |record| {
            record.journal_sequence <= sequence
                && !self.system_records.iter().flatten().any(|candidate| {
                    candidate.key == record.key
                        && candidate.journal_sequence <= sequence
                        && candidate.revision > record.revision
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fin(value: u128) -> Fin {
        Fin::from_u128(value)
    }

    fn cfc(value: u128) -> Cfc {
        let mut cfc = Cfc::new(CfcFin::from_u128(value), "Test CFC", fin(2)).unwrap();
        cfc.own_form(fin(1)).unwrap();
        cfc.own_form(fin(3)).unwrap();
        cfc
    }

    fn store() -> ExpFs {
        ExpFs::new(&cfc(100))
    }

    #[test]
    fn content_and_revision_publish_together_and_failed_batches_publish_nothing() {
        let mut store = store();
        let form = Fin::from_u128(1);
        let dimension = Fin::from_u128(2);
        let other = Fin::from_u128(3);
        let mut tx = store.begin();
        tx.stage_content(form, dimension, 1, b"first state")
            .unwrap();
        store.commit(tx).unwrap();
        let mut tx = store.begin();
        tx.stage_content(other, dimension, 1, b"must not appear")
            .unwrap();
        tx.stage_content(form, dimension, 1, b"stale").unwrap();
        assert_eq!(store.commit(tx), Err(StorageError::StaleRevision));
        assert!(store.latest(other, dimension).is_none());
        assert_eq!(
            store.latest(form, dimension).unwrap().content(),
            b"first state"
        );
        let mut tx = store.begin();
        tx.stage_content(form, dimension, 2, b"second state")
            .unwrap();
        assert_eq!(store.commit(tx), Ok(2));
        assert_eq!(
            store.latest(form, dimension).unwrap().content(),
            b"second state"
        );
    }

    #[test]
    fn oversized_or_duplicate_content_is_rejected_without_truncation() {
        let store = store();
        let mut tx = store.begin();
        let form = Fin::from_u128(1);
        let dimension = Fin::from_u128(2);
        assert_eq!(
            tx.stage_content(form, dimension, 1, &[0; FORM_CONTENT_CAPACITY + 1]),
            Err(StorageError::ContentTooLarge)
        );
        tx.stage_content(form, dimension, 1, b"valid").unwrap();
        assert_eq!(
            tx.stage_content(form, dimension, 2, b"duplicate"),
            Err(StorageError::DuplicateRecord)
        );
    }
    #[test]
    fn commits_share_one_journal_sequence() {
        let mut store = store();
        let mut tx = store.begin();
        tx.stage_form(Fin::from_u128(1), Fin::from_u128(2), 1)
            .unwrap();
        tx.stage_form(Fin::from_u128(3), Fin::from_u128(2), 1)
            .unwrap();
        let sequence = store.commit(tx).unwrap();
        assert_eq!(sequence, 1);
        assert_eq!(
            store
                .latest(Fin::from_u128(3), Fin::from_u128(2))
                .unwrap()
                .journal_sequence,
            sequence
        );
    }

    #[test]
    fn transactions_cannot_cross_cfc_boundaries() {
        let source = ExpFs::new(&cfc(100));
        let mut destination = ExpFs::new(&cfc(200));
        let mut transaction = source.begin();
        transaction
            .stage_form(Fin::from_u128(1), Fin::from_u128(2), 1)
            .unwrap();
        assert_eq!(destination.commit(transaction), Err(StorageError::WrongCfc));
        assert!(destination
            .latest(Fin::from_u128(1), Fin::from_u128(2))
            .is_none());
    }

    #[test]
    fn staging_rejects_forms_and_dimensions_outside_the_cfc_snapshot() {
        let store = store();
        let mut transaction = store.begin();
        assert_eq!(
            transaction.stage_form(fin(99), fin(2), 1),
            Err(StorageError::UnownedForm)
        );
        assert_eq!(
            transaction.stage_form(fin(1), fin(99), 1),
            Err(StorageError::UnownedDimension)
        );
        transaction.stage_form(fin(1), fin(2), 1).unwrap();
    }

    #[test]
    fn all_system_state_uses_typed_transactional_records() {
        let mut store = store();
        let dimension = fin(2);
        let form = fin(1);
        let form_key = RecordKey {
            kind: RecordKind::Form,
            identity: form,
            dimension,
        };
        let setting_key = RecordKey {
            kind: RecordKind::Setting,
            identity: fin(90),
            dimension: Fin::ZERO,
        };
        let mut transaction = store.begin();
        transaction
            .stage_record(form_key, 1, b"name=MyNotes")
            .unwrap();
        transaction
            .stage_record(setting_key, 1, b"theme=graphite")
            .unwrap();
        assert_eq!(store.commit(transaction), Ok(1));
        assert_eq!(
            store.latest_record(form_key).unwrap().bytes(),
            b"name=MyNotes"
        );
        assert_eq!(store.records_at(1).count(), 2);

        let mut transaction = store.begin();
        transaction
            .stage_record(form_key, 2, b"name=Notes")
            .unwrap();
        assert_eq!(store.commit(transaction), Ok(2));
        assert_eq!(
            store.latest_record(form_key).unwrap().bytes(),
            b"name=Notes"
        );
        assert_eq!(store.records_at(1).count(), 2);
        assert_eq!(store.records_at(2).count(), 2);
    }
}
