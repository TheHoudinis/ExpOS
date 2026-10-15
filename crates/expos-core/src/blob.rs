//! Chunked, CFC-owned storage for Form content.
//!
//! The public object is deliberately not a contiguous fixed-size array. Each
//! 4 KiB chunk is allocated only when content reaches it, and transactions
//! publish a complete replacement only after quota and revision preflight.

use alloc::{boxed::Box, vec, vec::Vec};

use crate::{CfcFin, Fin};

pub const BLOB_CHUNK_BYTES: usize = 4096;
pub const DEFAULT_FORM_BLOB_QUOTA: usize = 32 * 1024;
pub const DEFAULT_CFC_BLOB_QUOTA: usize = 48 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlobQuota {
    pub per_form_bytes: usize,
    pub cfc_bytes: usize,
}

impl Default for BlobQuota {
    fn default() -> Self {
        Self {
            per_form_bytes: DEFAULT_FORM_BLOB_QUOTA,
            cfc_bytes: DEFAULT_CFC_BLOB_QUOTA,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkedBlob {
    chunks: Vec<Box<[u8; BLOB_CHUNK_BYTES]>>,
    len: usize,
}

impl ChunkedBlob {
    pub const fn new() -> Self {
        Self {
            chunks: Vec::new(),
            len: 0,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut blob = Self::new();
        blob.replace(bytes);
        blob
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn allocated_bytes(&self) -> usize {
        self.chunks.len() * BLOB_CHUNK_BYTES
    }

    pub fn replace(&mut self, bytes: &[u8]) {
        let required = bytes.len().div_ceil(BLOB_CHUNK_BYTES);
        self.chunks
            .resize_with(required, || Box::new([0; BLOB_CHUNK_BYTES]));
        for (index, chunk) in self.chunks.iter_mut().enumerate() {
            let start = index * BLOB_CHUNK_BYTES;
            let end = (start + BLOB_CHUNK_BYTES).min(bytes.len());
            chunk.fill(0);
            if start < end {
                chunk[..end - start].copy_from_slice(&bytes[start..end]);
            }
        }
        self.len = bytes.len();
    }

    pub fn read(&self, offset: usize, output: &mut [u8]) -> usize {
        if offset >= self.len || output.is_empty() {
            return 0;
        }
        let count = output.len().min(self.len - offset);
        let mut copied = 0;
        while copied < count {
            let position = offset + copied;
            let chunk = position / BLOB_CHUNK_BYTES;
            let within = position % BLOB_CHUNK_BYTES;
            let take = (BLOB_CHUNK_BYTES - within).min(count - copied);
            output[copied..copied + take]
                .copy_from_slice(&self.chunks[chunk][within..within + take]);
            copied += take;
        }
        copied
    }

    pub fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<(), BlobError> {
        let end = offset
            .checked_add(bytes.len())
            .ok_or(BlobError::LengthOverflow)?;
        let required = end.div_ceil(BLOB_CHUNK_BYTES);
        self.chunks
            .resize_with(required, || Box::new([0; BLOB_CHUNK_BYTES]));
        let mut copied = 0;
        while copied < bytes.len() {
            let position = offset + copied;
            let chunk = position / BLOB_CHUNK_BYTES;
            let within = position % BLOB_CHUNK_BYTES;
            let take = (BLOB_CHUNK_BYTES - within).min(bytes.len() - copied);
            self.chunks[chunk][within..within + take]
                .copy_from_slice(&bytes[copied..copied + take]);
            copied += take;
        }
        self.len = self.len.max(end);
        Ok(())
    }

    pub fn to_vec(&self) -> Vec<u8> {
        let mut bytes = vec![0; self.len];
        self.read(0, &mut bytes);
        bytes
    }
}

impl Default for ChunkedBlob {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlobError {
    WrongCfc,
    InvalidForm,
    DuplicateForm,
    StaleRevision,
    FormQuotaExceeded,
    CfcQuotaExceeded,
    EmptyTransaction,
    LengthOverflow,
    NotFound,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BlobObject {
    form: Fin,
    revision: u32,
    content: ChunkedBlob,
}

#[derive(Clone, Debug)]
pub struct BlobTransaction {
    cfc: CfcFin,
    replacements: Vec<BlobObject>,
}

impl BlobTransaction {
    pub fn replace(&mut self, form: Fin, revision: u32, bytes: &[u8]) -> Result<(), BlobError> {
        if form.is_zero() || revision == 0 {
            return Err(BlobError::InvalidForm);
        }
        if self.replacements.iter().any(|object| object.form == form) {
            return Err(BlobError::DuplicateForm);
        }
        self.replacements.push(BlobObject {
            form,
            revision,
            content: ChunkedBlob::from_bytes(bytes),
        });
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CfcBlobStore {
    cfc: CfcFin,
    quota: BlobQuota,
    objects: Vec<BlobObject>,
    generation: u64,
}

impl CfcBlobStore {
    pub fn new(cfc: CfcFin, quota: BlobQuota) -> Result<Self, BlobError> {
        if cfc.is_zero() || quota.per_form_bytes == 0 || quota.cfc_bytes < quota.per_form_bytes {
            return Err(BlobError::WrongCfc);
        }
        Ok(Self {
            cfc,
            quota,
            objects: Vec::new(),
            generation: 0,
        })
    }

    pub fn begin(&self) -> BlobTransaction {
        BlobTransaction {
            cfc: self.cfc,
            replacements: Vec::new(),
        }
    }

    pub fn commit(&mut self, transaction: BlobTransaction) -> Result<u64, BlobError> {
        if transaction.cfc != self.cfc {
            return Err(BlobError::WrongCfc);
        }
        if transaction.replacements.is_empty() {
            return Err(BlobError::EmptyTransaction);
        }
        for replacement in &transaction.replacements {
            if replacement.content.len() > self.quota.per_form_bytes {
                return Err(BlobError::FormQuotaExceeded);
            }
            if self
                .objects
                .iter()
                .find(|object| object.form == replacement.form)
                .is_some_and(|object| object.revision >= replacement.revision)
            {
                return Err(BlobError::StaleRevision);
            }
        }
        let retained: usize = self
            .objects
            .iter()
            .filter(|object| {
                !transaction
                    .replacements
                    .iter()
                    .any(|replacement| replacement.form == object.form)
            })
            .map(|object| object.content.len())
            .sum();
        let staged: usize = transaction
            .replacements
            .iter()
            .map(|object| object.content.len())
            .sum();
        if retained.saturating_add(staged) > self.quota.cfc_bytes {
            return Err(BlobError::CfcQuotaExceeded);
        }
        for replacement in transaction.replacements {
            if let Some(object) = self
                .objects
                .iter_mut()
                .find(|object| object.form == replacement.form)
            {
                *object = replacement;
            } else {
                self.objects.push(replacement);
            }
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        Ok(self.generation)
    }

    pub fn read(&self, form: Fin, offset: usize, output: &mut [u8]) -> Result<usize, BlobError> {
        self.objects
            .iter()
            .find(|object| object.form == form)
            .map(|object| object.content.read(offset, output))
            .ok_or(BlobError::NotFound)
    }

    pub fn len(&self, form: Fin) -> Option<usize> {
        self.objects
            .iter()
            .find(|object| object.form == form)
            .map(|object| object.content.len())
    }

    pub fn used_bytes(&self) -> usize {
        self.objects.iter().map(|object| object.content.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_boundaries_round_trip_without_a_fixed_form_array() {
        let bytes: Vec<u8> = (0..(BLOB_CHUNK_BYTES * 3 + 17))
            .map(|index| (index % 251) as u8)
            .collect();
        let mut blob = ChunkedBlob::from_bytes(&bytes);
        assert_eq!(blob.allocated_bytes(), BLOB_CHUNK_BYTES * 4);
        assert_eq!(blob.to_vec(), bytes);
        blob.write(BLOB_CHUNK_BYTES - 2, b"abcdef").unwrap();
        let mut crossing = [0; 6];
        assert_eq!(blob.read(BLOB_CHUNK_BYTES - 2, &mut crossing), 6);
        assert_eq!(&crossing, b"abcdef");
    }

    #[test]
    fn quota_and_revision_failures_publish_nothing() {
        let cfc = CfcFin::from_u128(1);
        let form = Fin::from_u128(2);
        let mut store = CfcBlobStore::new(
            cfc,
            BlobQuota {
                per_form_bytes: 8192,
                cfc_bytes: 12288,
            },
        )
        .unwrap();
        let mut first = store.begin();
        first.replace(form, 1, &vec![7; 6000]).unwrap();
        assert_eq!(store.commit(first), Ok(1));

        let mut too_large = store.begin();
        too_large.replace(form, 2, &vec![9; 9000]).unwrap();
        assert_eq!(store.commit(too_large), Err(BlobError::FormQuotaExceeded));
        let mut byte = [0];
        store.read(form, 0, &mut byte).unwrap();
        assert_eq!(byte, [7]);

        let mut stale = store.begin();
        stale.replace(form, 1, b"stale").unwrap();
        assert_eq!(store.commit(stale), Err(BlobError::StaleRevision));
        assert_eq!(store.len(form), Some(6000));
    }
}
