//! Small allocation-free cryptographic building blocks used by the session
//! manager.  Passwords are stored as versioned PBKDF2-HMAC-SHA256 verifiers;
//! they are never serialized as plaintext or with reversible encryption.

use alloc::vec;
use argon2::{Algorithm, Argon2, Block, Params, Version};
use chacha20poly1305::{
    aead::{AeadInPlace, KeyInit},
    Tag, XChaCha20Poly1305, XNonce,
};
use core::sync::atomic::{AtomicU64, Ordering};

pub const PASSWORD_HASH_LEN: usize = 32;
pub const PASSWORD_SALT_LEN: usize = 16;
pub const PASSWORD_KDF_ROUNDS: u32 = 100_000;
pub const STORAGE_KEY_LEN: usize = 32;
pub const STORAGE_SALT_LEN: usize = 16;
pub const STORAGE_NONCE_LEN: usize = 24;
pub const STORAGE_TAG_LEN: usize = 16;
pub const STORAGE_ARGON2_MEMORY_KIB: u32 = 65_536;
pub const STORAGE_ARGON2_PASSES: u32 = 3;
pub const STORAGE_ARGON2_LANES: u32 = 1;
pub const STORAGE_PARANOID_ARGON2_PASSES: u32 = 6;

const SHA256_INITIAL: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Clone, Copy)]
struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    total_len: u64,
}

impl Sha256 {
    const fn new() -> Self {
        Self {
            state: SHA256_INITIAL,
            buffer: [0; 64],
            buffered: 0,
            total_len: 0,
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.total_len = self.total_len.wrapping_add(input.len() as u64);
        if self.buffered != 0 {
            let take = (64 - self.buffered).min(input.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&input[..take]);
            self.buffered += take;
            input = &input[take..];
            if self.buffered == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }
        while input.len() >= 64 {
            let mut block = [0_u8; 64];
            block.copy_from_slice(&input[..64]);
            self.compress(&block);
            input = &input[64..];
        }
        if !input.is_empty() {
            self.buffer[..input.len()].copy_from_slice(input);
            self.buffered = input.len();
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.buffer[self.buffered] = 0x80;
        self.buffered += 1;
        if self.buffered > 56 {
            self.buffer[self.buffered..].fill(0);
            let block = self.buffer;
            self.compress(&block);
            self.buffer = [0; 64];
            self.buffered = 0;
        }
        self.buffer[self.buffered..56].fill(0);
        self.buffer[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.buffer;
        self.compress(&block);

        let mut output = [0_u8; 32];
        for (index, word) in self.state.into_iter().enumerate() {
            output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        output
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut schedule = [0_u32; 64];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                block[offset],
                block[offset + 1],
                block[offset + 2],
                block[offset + 3],
            ]);
        }
        for index in 16..64 {
            let x = schedule[index - 15];
            let y = schedule[index - 2];
            let s0 = x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3);
            let s1 = y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(SHA256_K[index])
                .wrapping_add(schedule[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (state, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *state = state.wrapping_add(value);
        }
    }
}

fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut key_block = [0_u8; 64];
    if key.len() > key_block.len() {
        let mut digest = Sha256::new();
        digest.update(key);
        key_block[..32].copy_from_slice(&digest.finalize());
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for index in 0..64 {
        inner_pad[index] ^= key_block[index];
        outer_pad[index] ^= key_block[index];
    }

    let mut inner = Sha256::new();
    inner.update(&inner_pad);
    for part in parts {
        inner.update(part);
    }
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&outer_pad);
    outer.update(&inner_hash);
    let result = outer.finalize();

    // These stack buffers only contain key-derived intermediate material.
    // Volatile clearing prevents the compiler from eliding the cleanup.
    wipe(&mut key_block);
    wipe(&mut inner_pad);
    wipe(&mut outer_pad);
    result
}

/// Derive the fixed-size verifier stored in the persistent account record.
pub fn password_hash(password: &[u8], salt: &[u8; PASSWORD_SALT_LEN], rounds: u32) -> [u8; 32] {
    pbkdf2_hmac_sha256(password, salt, rounds)
}

fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let rounds = rounds.max(1);
    let block_number = 1_u32.to_be_bytes();
    let mut u = hmac_sha256(password, &[salt, &block_number]);
    let mut result = u;
    for _ in 1..rounds {
        u = hmac_sha256(password, &[&u]);
        for index in 0..PASSWORD_HASH_LEN {
            result[index] ^= u[index];
        }
    }
    wipe(&mut u);
    result
}

/// Compare secret-derived values without data-dependent early returns.
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut different = left.len() ^ right.len();
    let shared = left.len().min(right.len());
    for index in 0..shared {
        different |= (left[index] ^ right[index]) as usize;
    }
    different == 0
}

static SALT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static STORAGE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaltEntropy {
    Hardware,
    Degraded,
}

/// Produce a unique per-account salt. RDRAND is mixed in when the CPU offers
/// it; TSC and a monotonic sequence retain uniqueness on smaller emulators.
/// Salts need uniqueness, while password strength comes from PBKDF2.
pub fn password_salt(context: &[u8]) -> ([u8; PASSWORD_SALT_LEN], SaltEntropy) {
    let sequence = SALT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let ticks = unsafe { core::arch::x86_64::_rdtsc() };
    let hardware = hardware_random();
    let random = hardware.unwrap_or(0);
    let mut digest = Sha256::new();
    digest.update(b"ExpOS account salt v1");
    digest.update(context);
    digest.update(&sequence.to_le_bytes());
    digest.update(&ticks.to_le_bytes());
    digest.update(&random.to_le_bytes());
    let hash = digest.finalize();
    let mut salt = [0_u8; PASSWORD_SALT_LEN];
    salt.copy_from_slice(&hash[..PASSWORD_SALT_LEN]);
    (
        salt,
        if hardware.is_some() {
            SaltEntropy::Hardware
        } else {
            SaltEntropy::Degraded
        },
    )
}

/// Derive the key-encryption key used only to wrap a CFC's random storage key.
/// Manifest-controlled cost parameters are accepted only for audited profiles.
pub fn storage_kek_with_params(
    password: &[u8],
    salt: &[u8; STORAGE_SALT_LEN],
    memory_kib: u32,
    passes: u32,
    lanes: u32,
) -> Result<[u8; STORAGE_KEY_LEN], ()> {
    // Manifest-controlled values must never drive an unbounded allocation.
    // Genesis currently recognizes only the audited Easy and Paranoid profiles.
    if memory_kib != STORAGE_ARGON2_MEMORY_KIB
        || !matches!(
            passes,
            STORAGE_ARGON2_PASSES | STORAGE_PARANOID_ARGON2_PASSES
        )
        || lanes != STORAGE_ARGON2_LANES
    {
        return Err(());
    }
    storage_kek_with_params_unchecked(password, salt, memory_kib, passes, lanes)
}

fn storage_kek_with_params_unchecked(
    password: &[u8],
    salt: &[u8; STORAGE_SALT_LEN],
    memory_kib: u32,
    passes: u32,
    lanes: u32,
) -> Result<[u8; STORAGE_KEY_LEN], ()> {
    let params = Params::new(memory_kib, passes, lanes, Some(STORAGE_KEY_LEN)).map_err(|_| ())?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut memory = vec![Block::default(); memory_kib as usize];
    let mut output = [0_u8; STORAGE_KEY_LEN];
    let result = argon
        .hash_password_into_with_memory(password, salt, &mut output, &mut memory)
        .map_err(|_| ());
    for block in &mut memory {
        *block = Block::default();
    }
    result.map(|()| output)
}

/// Encrypt and authenticate bytes in place with the initial ExpFS AEAD suite.
pub fn seal_storage(
    key: &[u8; STORAGE_KEY_LEN],
    nonce: &[u8; STORAGE_NONCE_LEN],
    associated_data: &[u8],
    bytes: &mut [u8],
) -> Result<[u8; STORAGE_TAG_LEN], ()> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let tag = cipher
        .encrypt_in_place_detached(XNonce::from_slice(nonce), associated_data, bytes)
        .map_err(|_| ())?;
    Ok(tag.into())
}

/// Authenticate before releasing plaintext. A modified payload, CFC identity,
/// generation, slot header or tag is rejected without decoding any records.
pub fn open_storage(
    key: &[u8; STORAGE_KEY_LEN],
    nonce: &[u8; STORAGE_NONCE_LEN],
    associated_data: &[u8],
    bytes: &mut [u8],
    tag: &[u8; STORAGE_TAG_LEN],
) -> Result<(), ()> {
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt_in_place_detached(
            XNonce::from_slice(nonce),
            associated_data,
            bytes,
            Tag::from_slice(tag),
        )
        .map_err(|_| ())
}

/// Mint independent key, salt or nonce material. RDRAND is mixed when present;
/// TSC, an atomic sequence and a caller context prevent reuse on small VMs.
pub fn random_material<const N: usize>(context: &[u8]) -> [u8; N] {
    let mut output = [0_u8; N];
    let mut offset = 0;
    while offset < N {
        let sequence = STORAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let ticks = unsafe { core::arch::x86_64::_rdtsc() };
        let random = hardware_random().unwrap_or(0);
        let mut digest = Sha256::new();
        digest.update(b"ExpOS CFC storage entropy v1");
        digest.update(context);
        digest.update(&sequence.to_le_bytes());
        digest.update(&ticks.to_le_bytes());
        digest.update(&random.to_le_bytes());
        let block = digest.finalize();
        let count = (N - offset).min(block.len());
        output[offset..offset + count].copy_from_slice(&block[..count]);
        offset += count;
    }
    output
}

pub fn storage_nonce(
    key: &[u8; STORAGE_KEY_LEN],
    cfc: &[u8; 16],
    generation: u64,
    domain: u32,
) -> [u8; STORAGE_NONCE_LEN] {
    let generation = generation.to_le_bytes();
    let domain = domain.to_le_bytes();
    let digest = hmac_sha256(key, &[b"ExpOS ExpFS nonce v1", cfc, &generation, &domain]);
    digest[..STORAGE_NONCE_LEN]
        .try_into()
        .unwrap_or([0; STORAGE_NONCE_LEN])
}

#[cfg(target_arch = "x86_64")]
fn hardware_random() -> Option<u64> {
    let features = core::arch::x86_64::__cpuid(1);
    if features.ecx & (1 << 30) == 0 {
        return None;
    }
    for _ in 0..10 {
        let mut value: u64;
        let mut ready: u8;
        unsafe {
            core::arch::asm!(
                "rdrand {value}",
                "setc {ready}",
                value = out(reg) value,
                ready = out(reg_byte) ready,
                options(nomem, nostack)
            );
        }
        if ready != 0 {
            return Some(value);
        }
    }
    None
}

#[cfg_attr(not(feature = "genesis-installer"), allow(dead_code))]
pub fn hardware_entropy_available() -> bool {
    hardware_random().is_some()
}

/// Best-effort clearing for transient plaintext and key-derived buffers.
pub fn wipe(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe { core::ptr::write_volatile(byte, 0) };
    }
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(value: &str) -> [u8; 32] {
        let mut output = [0_u8; 32];
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
        }
        output
    }

    #[test]
    fn sha256_matches_nist_vector() {
        let mut digest = Sha256::new();
        digest.update(b"abc");
        assert_eq!(
            digest.finalize(),
            hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }

    #[test]
    fn hmac_matches_rfc_4231_vector() {
        let key = [0x0b_u8; 20];
        assert_eq!(
            hmac_sha256(&key, &[b"Hi There"]),
            hex("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7")
        );
    }

    #[test]
    fn pbkdf2_matches_standard_vectors() {
        assert_eq!(
            pbkdf2_hmac_sha256(b"password", b"salt", 1),
            hex("120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b")
        );
        assert_eq!(
            pbkdf2_hmac_sha256(b"password", b"salt", 2),
            hex("ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43")
        );
    }

    #[test]
    fn secret_comparison_checks_every_byte() {
        assert!(constant_time_eq(&[1, 2, 3], &[1, 2, 3]));
        assert!(!constant_time_eq(&[0, 2, 3], &[1, 2, 3]));
        assert!(!constant_time_eq(&[1, 2], &[1, 2, 3]));
    }

    #[test]
    fn storage_aead_binds_ciphertext_and_cfc_metadata() {
        let key = [7_u8; STORAGE_KEY_LEN];
        let nonce = [9_u8; STORAGE_NONCE_LEN];
        let mut payload = *b"transactional ExpFS";
        let plaintext = payload;
        let tag = seal_storage(&key, &nonce, b"cfc=1,generation=2", &mut payload).unwrap();
        assert_ne!(payload, plaintext);
        open_storage(&key, &nonce, b"cfc=1,generation=2", &mut payload, &tag).unwrap();
        assert_eq!(payload, plaintext);

        let mut rejected = *b"transactional ExpFS";
        let tag = seal_storage(&key, &nonce, b"cfc=1", &mut rejected).unwrap();
        assert!(open_storage(&key, &nonce, b"cfc=2", &mut rejected, &tag).is_err());
    }

    #[test]
    fn argon2id_kek_uses_password_and_salt() {
        let salt = [3_u8; STORAGE_SALT_LEN];
        let first = storage_kek_with_params_unchecked(b"correct horse", &salt, 32, 2, 1).unwrap();
        let second = storage_kek_with_params_unchecked(b"wrong horse", &salt, 32, 2, 1).unwrap();
        assert_ne!(first, second);
        assert!(storage_kek_with_params(b"password", &salt, 32, 2, 1).is_err());
    }
}
