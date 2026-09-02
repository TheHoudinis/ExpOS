use crate::Fin;

const MAX_RECORDS: usize = 32;
const MAX_STAGED: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistentForm {
    pub form: Fin,
    pub dimension: Fin,
    pub revision: u32,
    pub journal_sequence: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    Full,
    EmptyTransaction,
    InvalidRecord,
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
        if form.is_zero() || dimension.is_zero() || revision == 0 {
            return Err(StorageError::InvalidRecord);
        }
        let slot = self
            .records
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(StorageError::Full)?;
        *slot = Some(PersistentForm {
            form,
            dimension,
            revision,
            journal_sequence: 0,
        });
        Ok(())
    }
}

pub struct HexaFs {
    records: [Option<PersistentForm>; MAX_RECORDS],
    next_sequence: u32,
}

impl HexaFs {
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
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
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

impl Default for HexaFs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commits_share_one_journal_sequence() {
        let mut store = HexaFs::new();
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
