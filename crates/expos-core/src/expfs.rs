use crate::Fin;

const MAX_RECORDS: usize = 32;
const MAX_STAGED: usize = 8;
pub const FORM_CONTENT_CAPACITY: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistentForm {
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
    StaleRevision,
    SequenceExhausted,
}

pub struct Transaction {
    records: [Option<PersistentForm>; MAX_STAGED],
}

impl Transaction {
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
    records: [Option<PersistentForm>; MAX_RECORDS],
    next_sequence: u32,
}

impl ExpFs {
    pub const fn new() -> Self {
        Self {
            records: [None; MAX_RECORDS],
            next_sequence: 1,
        }
    }
    pub const fn begin(&self) -> Transaction {
        Transaction {
            records: [None; MAX_STAGED],
        }
    }

    /// Atomically validates capacity before publishing any staged record.
    pub fn commit(&mut self, transaction: Transaction) -> Result<u32, StorageError> {
        let staged_count = transaction.records.iter().flatten().count();
        if staged_count == 0 {
            return Err(StorageError::EmptyTransaction);
        }
        let free_count = self.records.iter().filter(|slot| slot.is_none()).count();
        if staged_count > free_count {
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
        Ok(sequence)
    }

    pub fn latest(&self, form: Fin, dimension: Fin) -> Option<&PersistentForm> {
        self.records
            .iter()
            .flatten()
            .filter(|record| record.form == form && record.dimension == dimension)
            .max_by_key(|record| record.revision)
    }
}

impl Default for ExpFs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_and_revision_publish_together_and_failed_batches_publish_nothing() {
        let mut store = ExpFs::new();
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
        let store = ExpFs::new();
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
        let mut store = ExpFs::new();
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
}
