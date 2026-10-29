//! Transactional, VM-state-disk storage for Ayo music packages.
//!
//! ExpFS ends before LBA 2048. Development VMs use an unpartitioned state
//! disk, so the remaining sectors can hold downloaded media without growing
//! the frozen Form ABI. Genesis disks are deliberately rejected: their GPT
//! assigns this range to the EFI System Partition and must never be touched.

use crate::{audio::Track, crypto, storage};
use alloc::{boxed::Box, vec};

const MAGIC: [u8; 8] = *b"EXMEDIA1";
const VERSION: u16 = 1;
const REGION_START: u32 = 2048;
const HEADER_COPIES: u32 = 2;
const DATA_SLOT_BYTES: usize = 8 * 1024 * 1024;
const DATA_SLOT_SECTORS: u32 = (DATA_SLOT_BYTES / storage::SECTOR_SIZE) as u32;
const DATA_SLOT_COUNT: usize = 3;
const ENTRY_OFFSET: usize = 32;
const ENTRY_LEN: usize = 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaStoreError {
    Unavailable,
    PartitionedDisk,
    DiskTooSmall,
    NotInstalled,
    PackageTooLarge,
    StoreFull,
    InvalidOffset,
    DigestMismatch,
    CorruptCatalog,
    Disk(storage::StorageError),
}

impl MediaStoreError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "the VM media store is unavailable",
            Self::PartitionedDisk => "music installs are disabled on Genesis/GPT disks",
            Self::DiskTooSmall => "the VM state disk is too small for downloaded music",
            Self::NotInstalled => "the track is not installed; use 'ayo music install' first",
            Self::PackageTooLarge => "the music package exceeds the 8 MiB VM limit",
            Self::StoreFull => "the music store has no transactional slot available",
            Self::InvalidOffset => "the music download arrived out of order",
            Self::DigestMismatch => "the downloaded music failed SHA-256 verification",
            Self::CorruptCatalog => "the music catalog is damaged",
            Self::Disk(_) => "music-store disk I/O failed",
        }
    }
}

impl From<storage::StorageError> for MediaStoreError {
    fn from(value: storage::StorageError) -> Self {
        Self::Disk(value)
    }
}

#[derive(Clone, Copy, Default)]
struct Entry {
    present: bool,
    track: u8,
    slot: u8,
    length: u32,
    sha256: [u8; 32],
}

#[derive(Clone, Copy)]
struct Catalog {
    generation: u64,
    entries: [Entry; 2],
}

impl Catalog {
    const fn empty() -> Self {
        Self {
            generation: 0,
            entries: [Entry {
                present: false,
                track: 0,
                slot: 0,
                length: 0,
                sha256: [0; 32],
            }; 2],
        }
    }

    fn entry(self, track: Track) -> Option<Entry> {
        self.entries
            .into_iter()
            .find(|entry| entry.present && entry.track == track.id())
    }
}

pub struct Installer {
    device: storage::Device,
    base_generation: u64,
    track: Track,
    slot: u8,
    expected_length: usize,
    expected_sha256: [u8; 32],
    written: usize,
    transfer_digest: crypto::Sha256,
}

pub fn installed(track: Track) -> bool {
    open_catalog()
        .ok()
        .and_then(|(_, catalog)| catalog.entry(track))
        .is_some()
}

pub fn begin_install(track: Track) -> Result<Installer, MediaStoreError> {
    let info = track.info();
    if info.compressed_bytes > DATA_SLOT_BYTES {
        return Err(MediaStoreError::PackageTooLarge);
    }
    let (device, catalog) = open_catalog()?;
    let mut used = [false; DATA_SLOT_COUNT];
    for entry in catalog.entries.into_iter().filter(|entry| entry.present) {
        if (entry.slot as usize) < used.len() {
            used[entry.slot as usize] = true;
        }
    }
    let slot = used
        .iter()
        .position(|value| !*value)
        .ok_or(MediaStoreError::StoreFull)? as u8;
    Ok(Installer {
        device,
        base_generation: catalog.generation,
        track,
        slot,
        expected_length: info.compressed_bytes,
        expected_sha256: parse_digest(info.sha256).ok_or(MediaStoreError::CorruptCatalog)?,
        written: 0,
        transfer_digest: crypto::Sha256::new(),
    })
}

impl Installer {
    pub const fn expected_length(&self) -> usize {
        self.expected_length
    }

    pub const fn written(&self) -> usize {
        self.written
    }

    pub fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<(), MediaStoreError> {
        if offset != self.written
            || bytes.is_empty()
            || offset
                .checked_add(bytes.len())
                .is_none_or(|end| end > self.expected_length)
        {
            return Err(MediaStoreError::InvalidOffset);
        }
        let slot_base = REGION_START + HEADER_COPIES + self.slot as u32 * DATA_SLOT_SECTORS;
        let mut cursor = 0;
        while cursor < bytes.len() {
            let absolute = offset + cursor;
            let sector_index = absolute / storage::SECTOR_SIZE;
            let within = absolute % storage::SECTOR_SIZE;
            let count = (storage::SECTOR_SIZE - within).min(bytes.len() - cursor);
            let mut sector = [0_u8; storage::SECTOR_SIZE];
            if within != 0 || count != storage::SECTOR_SIZE {
                self.device
                    .read_sector(slot_base + sector_index as u32, &mut sector)?;
            }
            sector[within..within + count].copy_from_slice(&bytes[cursor..cursor + count]);
            self.device
                .write_sector_unflushed(slot_base + sector_index as u32, &sector)?;
            cursor += count;
        }
        self.transfer_digest.update(bytes);
        self.written += bytes.len();
        Ok(())
    }

    pub fn commit(self) -> Result<(), MediaStoreError> {
        if self.written != self.expected_length
            || !crypto::constant_time_eq(&self.transfer_digest.finalize(), &self.expected_sha256)
        {
            return Err(MediaStoreError::DigestMismatch);
        }
        self.device.flush()?;
        let (_, mut catalog) = open_catalog()?;
        if catalog.generation != self.base_generation {
            return Err(MediaStoreError::CorruptCatalog);
        }
        verify_slot(
            self.device,
            self.slot,
            self.expected_length,
            &self.expected_sha256,
        )?;
        catalog.entries[self.track.catalog_index()] = Entry {
            present: true,
            track: self.track.id(),
            slot: self.slot,
            length: self.expected_length as u32,
            sha256: self.expected_sha256,
        };
        catalog.generation = catalog.generation.saturating_add(1);
        write_catalog(self.device, catalog)?;
        crate::slog!(
            "EXPOS_AYO_MUSIC_COMMIT track={} bytes={} slot={} generation={} sha256={}\r\n",
            self.track.info().command_name,
            self.expected_length,
            self.slot,
            catalog.generation,
            self.track.info().sha256
        );
        Ok(())
    }
}

pub fn remove(track: Track) -> Result<bool, MediaStoreError> {
    let (device, mut catalog) = open_catalog()?;
    let index = track.catalog_index();
    if !catalog.entries[index].present {
        return Ok(false);
    }
    catalog.entries[index] = Entry::default();
    catalog.generation = catalog.generation.saturating_add(1);
    write_catalog(device, catalog)?;
    crate::slog!(
        "EXPOS_AYO_MUSIC_REMOVE track={} generation={}\r\n",
        track.info().command_name,
        catalog.generation
    );
    Ok(true)
}

pub fn load(track: Track) -> Result<Box<[u8]>, MediaStoreError> {
    let (device, catalog) = open_catalog()?;
    let entry = catalog.entry(track).ok_or(MediaStoreError::NotInstalled)?;
    if entry.length as usize != track.info().compressed_bytes
        || !crypto::constant_time_eq(
            &entry.sha256,
            &parse_digest(track.info().sha256).ok_or(MediaStoreError::CorruptCatalog)?,
        )
    {
        return Err(MediaStoreError::CorruptCatalog);
    }
    let mut bytes = vec![0_u8; entry.length as usize].into_boxed_slice();
    read_slot(device, entry.slot, &mut bytes)?;
    if !crypto::constant_time_eq(&crypto::sha256(&bytes), &entry.sha256) {
        return Err(MediaStoreError::DigestMismatch);
    }
    Ok(bytes)
}

fn open_catalog() -> Result<(storage::Device, Catalog), MediaStoreError> {
    let device = storage::initialize().map_err(|_| MediaStoreError::Unavailable)?;
    let required = REGION_START + HEADER_COPIES + DATA_SLOT_COUNT as u32 * DATA_SLOT_SECTORS;
    if device.sectors() < required {
        return Err(MediaStoreError::DiskTooSmall);
    }
    let mut mbr = [0_u8; storage::SECTOR_SIZE];
    device.read_sector(0, &mut mbr)?;
    if mbr[510..512] == [0x55, 0xAA]
        && (0..4).any(|index| {
            let at = 446 + index * 16;
            mbr[at + 4] != 0 || mbr[at + 8..at + 12] != [0; 4] || mbr[at + 12..at + 16] != [0; 4]
        })
    {
        return Err(MediaStoreError::PartitionedDisk);
    }
    let mut gpt = [0_u8; storage::SECTOR_SIZE];
    device.read_sector(1, &mut gpt)?;
    if gpt[..8] == *b"EFI PART" {
        return Err(MediaStoreError::PartitionedDisk);
    }
    let first = read_catalog_copy(device, 0).ok();
    let second = read_catalog_copy(device, 1).ok();
    let catalog = match (first, second) {
        (Some(a), Some(b)) => {
            if a.generation >= b.generation {
                a
            } else {
                b
            }
        }
        (Some(value), None) | (None, Some(value)) => value,
        (None, None) => Catalog::empty(),
    };
    Ok((device, catalog))
}

fn read_catalog_copy(device: storage::Device, copy: u32) -> Result<Catalog, MediaStoreError> {
    let mut bytes = [0_u8; storage::SECTOR_SIZE];
    device.read_sector(REGION_START + copy, &mut bytes)?;
    decode_catalog(&bytes)
}

fn write_catalog(device: storage::Device, catalog: Catalog) -> Result<(), MediaStoreError> {
    let bytes = encode_catalog(catalog);
    let copy = (catalog.generation & 1) as u32;
    device.write_sector(REGION_START + copy, &bytes)?;
    let verified = read_catalog_copy(device, copy)?;
    if verified.generation != catalog.generation {
        return Err(MediaStoreError::CorruptCatalog);
    }
    Ok(())
}

fn read_slot(device: storage::Device, slot: u8, output: &mut [u8]) -> Result<(), MediaStoreError> {
    if slot as usize >= DATA_SLOT_COUNT || output.len() > DATA_SLOT_BYTES {
        return Err(MediaStoreError::CorruptCatalog);
    }
    let base = REGION_START + HEADER_COPIES + slot as u32 * DATA_SLOT_SECTORS;
    let mut sector = [0_u8; storage::SECTOR_SIZE];
    for (index, chunk) in output.chunks_mut(storage::SECTOR_SIZE).enumerate() {
        device.read_sector(base + index as u32, &mut sector)?;
        chunk.copy_from_slice(&sector[..chunk.len()]);
    }
    Ok(())
}

fn verify_slot(
    device: storage::Device,
    slot: u8,
    length: usize,
    expected: &[u8; 32],
) -> Result<(), MediaStoreError> {
    let base = REGION_START + HEADER_COPIES + slot as u32 * DATA_SLOT_SECTORS;
    let mut digest = crypto::Sha256::new();
    let mut sector = [0_u8; storage::SECTOR_SIZE];
    let mut remaining = length;
    let mut index = 0_u32;
    while remaining != 0 {
        device.read_sector(base + index, &mut sector)?;
        let count = remaining.min(sector.len());
        digest.update(&sector[..count]);
        remaining -= count;
        index += 1;
    }
    if !crypto::constant_time_eq(&digest.finalize(), expected) {
        return Err(MediaStoreError::DigestMismatch);
    }
    Ok(())
}

fn encode_catalog(catalog: Catalog) -> [u8; storage::SECTOR_SIZE] {
    let mut output = [0_u8; storage::SECTOR_SIZE];
    output[..8].copy_from_slice(&MAGIC);
    output[8..10].copy_from_slice(&VERSION.to_le_bytes());
    output[16..24].copy_from_slice(&catalog.generation.to_le_bytes());
    for (index, entry) in catalog.entries.into_iter().enumerate() {
        let at = ENTRY_OFFSET + index * ENTRY_LEN;
        output[at] = entry.present as u8;
        output[at + 1] = entry.track;
        output[at + 2] = entry.slot;
        output[at + 4..at + 8].copy_from_slice(&entry.length.to_le_bytes());
        output[at + 8..at + 40].copy_from_slice(&entry.sha256);
    }
    let checksum = crc32(&output[..508]);
    output[508..].copy_from_slice(&checksum.to_le_bytes());
    output
}

fn decode_catalog(input: &[u8; storage::SECTOR_SIZE]) -> Result<Catalog, MediaStoreError> {
    if input[..8] != MAGIC
        || u16::from_le_bytes([input[8], input[9]]) != VERSION
        || crc32(&input[..508]) != u32::from_le_bytes(input[508..512].try_into().unwrap())
    {
        return Err(MediaStoreError::CorruptCatalog);
    }
    let mut catalog = Catalog {
        generation: u64::from_le_bytes(input[16..24].try_into().unwrap()),
        entries: [Entry::default(); 2],
    };
    let mut slots = [false; DATA_SLOT_COUNT];
    for (index, entry) in catalog.entries.iter_mut().enumerate() {
        let at = ENTRY_OFFSET + index * ENTRY_LEN;
        entry.present = input[at] == 1;
        entry.track = input[at + 1];
        entry.slot = input[at + 2];
        entry.length = u32::from_le_bytes(input[at + 4..at + 8].try_into().unwrap());
        entry.sha256.copy_from_slice(&input[at + 8..at + 40]);
        if entry.present {
            let slot = entry.slot as usize;
            if entry.track as usize != index
                || slot >= DATA_SLOT_COUNT
                || slots[slot]
                || entry.length == 0
                || entry.length as usize > DATA_SLOT_BYTES
            {
                return Err(MediaStoreError::CorruptCatalog);
            }
            slots[slot] = true;
        }
    }
    Ok(catalog)
}

fn parse_digest(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let nibble = |byte| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    };
    let mut output = [0_u8; 32];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        output[index] = nibble(pair[0])? << 4 | nibble(pair[1])?;
    }
    Some(output)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_roundtrip_rejects_damage_and_duplicate_slots() {
        let digest = [7_u8; 32];
        let catalog = Catalog {
            generation: 9,
            entries: [
                Entry {
                    present: true,
                    track: 0,
                    slot: 1,
                    length: 1234,
                    sha256: digest,
                },
                Entry::default(),
            ],
        };
        let encoded = encode_catalog(catalog);
        let decoded = decode_catalog(&encoded).unwrap();
        assert_eq!(decoded.generation, 9);
        assert_eq!(decoded.entries[0].length, 1234);
        let mut damaged = encoded;
        damaged[70] ^= 1;
        assert!(matches!(
            decode_catalog(&damaged),
            Err(MediaStoreError::CorruptCatalog)
        ));

        let mut duplicate = catalog;
        duplicate.entries[1] = Entry {
            present: true,
            track: 1,
            slot: 1,
            length: 99,
            sha256: digest,
        };
        assert!(matches!(
            decode_catalog(&encode_catalog(duplicate)),
            Err(MediaStoreError::CorruptCatalog)
        ));
    }

    #[test]
    fn expected_track_digests_are_well_formed() {
        for track in Track::ALL {
            assert!(parse_digest(track.info().sha256).is_some());
            assert!(track.info().compressed_bytes <= DATA_SLOT_BYTES);
        }
    }
}
