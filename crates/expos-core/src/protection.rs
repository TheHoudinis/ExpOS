//! Genesis storage-protection policy.
//!
//! This module is an enforceable configuration contract, not a cryptographic
//! implementation. The native ExpFS block layer must generate, wrap, encrypt,
//! authenticate and wipe key material before it may claim this policy is
//! active.

use crate::CfcFin;

pub const STORAGE_KEY_BYTES: usize = 32;
pub const ARGON2ID_SALT_BYTES: usize = 16;
pub const ARGON2ID_VERSION_13: u32 = 0x13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallationMode {
    Basic,
    Architect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootSupportPolicy {
    /// Native UEFI is the normal path. BIOS/GRUB remains a non-default
    /// compatibility, recovery and development fallback.
    UefiPrimaryWithBiosFallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnlockMethod {
    OperatorPasswordArgon2id,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentProtection {
    AuthenticatedEncryption,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordChangeAction {
    /// Rewrap the unchanged random CFC storage key instead of re-encrypting
    /// every protected block.
    RewrapStorageKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Argon2idParameters {
    memory_kib: u32,
    passes: u32,
    lanes: u8,
    version: u32,
}

impl Argon2idParameters {
    pub fn new(memory_kib: u32, passes: u32, lanes: u8) -> Result<Self, ProtectionError> {
        let minimum_memory = (lanes as u32).saturating_mul(8);
        if lanes == 0 || passes == 0 || memory_kib < minimum_memory {
            return Err(ProtectionError::InvalidArgon2idParameters);
        }
        Ok(Self {
            memory_kib,
            passes,
            lanes,
            version: ARGON2ID_VERSION_13,
        })
    }

    pub const fn memory_kib(self) -> u32 {
        self.memory_kib
    }

    pub const fn passes(self) -> u32 {
        self.passes
    }

    pub const fn lanes(self) -> u8 {
        self.lanes
    }

    pub const fn version(self) -> u32 {
        self.version
    }
}

/// Versioned authenticated-encryption suite descriptor. The concrete initial
/// suite remains an ExpFS format decision; suite zero is reserved and a suite
/// must provide a full 128-bit authentication tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AeadSuite {
    id: u16,
    nonce_bytes: u8,
    tag_bytes: u8,
}

impl AeadSuite {
    pub fn new(id: u16, nonce_bytes: u8, tag_bytes: u8) -> Result<Self, ProtectionError> {
        if id == 0 || nonce_bytes < 12 || tag_bytes < 16 {
            return Err(ProtectionError::InvalidAeadSuite);
        }
        Ok(Self {
            id,
            nonce_bytes,
            tag_bytes,
        })
    }

    pub const fn id(self) -> u16 {
        self.id
    }

    pub const fn nonce_bytes(self) -> u8 {
        self.nonce_bytes
    }

    pub const fn tag_bytes(self) -> u8 {
        self.tag_bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncryptedStoragePolicy {
    kdf: Argon2idParameters,
    aead: AeadSuite,
}

impl EncryptedStoragePolicy {
    pub const fn kdf(self) -> Argon2idParameters {
        self.kdf
    }

    pub const fn aead(self) -> AeadSuite {
        self.aead
    }

    pub const fn storage_key_bytes(self) -> usize {
        STORAGE_KEY_BYTES
    }

    pub const fn unlock_method(self) -> UnlockMethod {
        UnlockMethod::OperatorPasswordArgon2id
    }

    pub const fn kdf_salt_bytes(self) -> usize {
        ARGON2ID_SALT_BYTES
    }

    pub const fn content_protection(self) -> ContentProtection {
        ContentProtection::AuthenticatedEncryption
    }

    pub const fn binds_cfc_metadata_as_associated_data(self) -> bool {
        true
    }

    pub const fn password_change_action(self) -> PasswordChangeAction {
        PasswordChangeAction::RewrapStorageKey
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtectionError {
    InvalidCfcFin,
    InvalidArgon2idParameters,
    InvalidAeadSuite,
    EncryptionRequired,
}

/// Per-CFC Genesis decision. An encrypted policy always means an independent,
/// randomly generated 256-bit storage key. There is intentionally no API for
/// supplying or sharing a data key between CFCs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageProtectionPolicy {
    cfc: CfcFin,
    mode: InstallationMode,
    encrypted: Option<EncryptedStoragePolicy>,
}

impl StorageProtectionPolicy {
    pub fn basic(
        cfc: CfcFin,
        kdf: Argon2idParameters,
        aead: AeadSuite,
    ) -> Result<Self, ProtectionError> {
        Self::build(InstallationMode::Basic, cfc, Some((kdf, aead)))
    }

    pub fn architect(
        cfc: CfcFin,
        encryption: Option<(Argon2idParameters, AeadSuite)>,
    ) -> Result<Self, ProtectionError> {
        Self::build(InstallationMode::Architect, cfc, encryption)
    }

    fn build(
        mode: InstallationMode,
        cfc: CfcFin,
        encryption: Option<(Argon2idParameters, AeadSuite)>,
    ) -> Result<Self, ProtectionError> {
        if cfc.is_zero() {
            return Err(ProtectionError::InvalidCfcFin);
        }
        if mode == InstallationMode::Basic && encryption.is_none() {
            return Err(ProtectionError::EncryptionRequired);
        }
        Ok(Self {
            cfc,
            mode,
            encrypted: encryption.map(|(kdf, aead)| EncryptedStoragePolicy { kdf, aead }),
        })
    }

    pub const fn cfc(self) -> CfcFin {
        self.cfc
    }

    pub const fn mode(self) -> InstallationMode {
        self.mode
    }

    pub const fn encryption(self) -> Option<EncryptedStoragePolicy> {
        self.encrypted
    }

    pub const fn is_encrypted(self) -> bool {
        self.encrypted.is_some()
    }

    pub const fn uses_independent_random_storage_key(self) -> bool {
        self.encrypted.is_some()
    }

    pub const fn boot_support(self) -> BootSupportPolicy {
        BootSupportPolicy::UefiPrimaryWithBiosFallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfc(value: u128) -> CfcFin {
        CfcFin::from_u128(value)
    }

    fn kdf() -> Argon2idParameters {
        Argon2idParameters::new(65_536, 3, 1).unwrap()
    }

    fn aead() -> AeadSuite {
        AeadSuite::new(1, 24, 16).unwrap()
    }

    #[test]
    fn basic_always_uses_random_key_wrapping_and_authenticated_encryption() {
        let policy = StorageProtectionPolicy::basic(cfc(1), kdf(), aead()).unwrap();
        let encrypted = policy.encryption().unwrap();
        assert_eq!(policy.mode(), InstallationMode::Basic);
        assert!(policy.uses_independent_random_storage_key());
        assert_eq!(encrypted.kdf().memory_kib(), 65_536);
        assert_eq!(encrypted.kdf().passes(), 3);
        assert_eq!(encrypted.kdf().lanes(), 1);
        assert_eq!(encrypted.kdf().version(), ARGON2ID_VERSION_13);
        assert_eq!(encrypted.aead().id(), 1);
        assert_eq!(encrypted.aead().nonce_bytes(), 24);
        assert_eq!(encrypted.aead().tag_bytes(), 16);
        assert_eq!(encrypted.storage_key_bytes(), 32);
        assert_eq!(encrypted.kdf_salt_bytes(), 16);
        assert_eq!(
            encrypted.unlock_method(),
            UnlockMethod::OperatorPasswordArgon2id
        );
        assert_eq!(
            encrypted.content_protection(),
            ContentProtection::AuthenticatedEncryption
        );
        assert!(encrypted.binds_cfc_metadata_as_associated_data());
        assert_eq!(
            encrypted.password_change_action(),
            PasswordChangeAction::RewrapStorageKey
        );
    }

    #[test]
    fn architect_may_disable_encryption_without_weakening_basic() {
        let architect = StorageProtectionPolicy::architect(cfc(1), None).unwrap();
        assert!(!architect.is_encrypted());
        assert_eq!(architect.mode(), InstallationMode::Architect);
        assert_eq!(
            StorageProtectionPolicy::build(InstallationMode::Basic, cfc(1), None),
            Err(ProtectionError::EncryptionRequired)
        );
    }

    #[test]
    fn encrypted_cfcs_have_distinct_ownership_even_with_the_same_profiles() {
        let first = StorageProtectionPolicy::architect(cfc(1), Some((kdf(), aead()))).unwrap();
        let second = StorageProtectionPolicy::architect(cfc(2), Some((kdf(), aead()))).unwrap();
        assert_ne!(first.cfc(), second.cfc());
        assert!(first.uses_independent_random_storage_key());
        assert!(second.uses_independent_random_storage_key());
        assert_eq!(
            first.boot_support(),
            BootSupportPolicy::UefiPrimaryWithBiosFallback
        );
    }

    #[test]
    fn invalid_algorithm_descriptors_fail_closed() {
        assert_eq!(
            Argon2idParameters::new(7, 3, 1),
            Err(ProtectionError::InvalidArgon2idParameters)
        );
        assert_eq!(
            Argon2idParameters::new(64, 0, 1),
            Err(ProtectionError::InvalidArgon2idParameters)
        );
        assert_eq!(
            AeadSuite::new(0, 24, 16),
            Err(ProtectionError::InvalidAeadSuite)
        );
        assert_eq!(
            AeadSuite::new(1, 8, 16),
            Err(ProtectionError::InvalidAeadSuite)
        );
    }
}
