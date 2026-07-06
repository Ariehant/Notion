//! Key derivation — audit §2.2 and §2.6.
//!
//! Two stages:
//!   1. **Argon2id** stretches the user password (+ per-vault salt) into a
//!      256-bit master key. Parameters are tuned toward disk-unlock cost
//!      (~0.5–1 s), higher than the blueprint's original 64 MB (§2.6).
//!   2. **HKDF-SHA256** expands that master key into purpose-separated subkeys
//!      using distinct `info` labels. We never slice raw bytes off a master key
//!      (§2.2) — that invites cross-context key reuse.

use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::{CryptoError, KEY_LEN};

/// Argon2id cost parameters. Defaults target roughly 0.5–1 s on unlock (§2.6).
#[derive(Debug, Clone, Copy)]
pub struct Argon2Params {
    /// Memory cost in KiB.
    pub mem_kib: u32,
    /// Number of iterations (time cost).
    pub iterations: u32,
    /// Degree of parallelism.
    pub parallelism: u32,
}

impl Default for Argon2Params {
    fn default() -> Self {
        // 128 MiB / t=3 / p=1 — the audit (§2.6) flags the blueprint's 64 MiB as
        // low for a disk-unlock KDF and suggests 128–256 MiB.
        Self {
            mem_kib: 128 * 1024,
            iterations: 3,
            parallelism: 1,
        }
    }
}

impl Argon2Params {
    /// Deliberately cheap parameters for fast unit tests only. NEVER ship these.
    #[cfg(test)]
    pub(crate) fn insecure_fast() -> Self {
        Self {
            mem_kib: 8,
            iterations: 1,
            parallelism: 1,
        }
    }
}

/// 256-bit master key from Argon2id. Zeroized on drop; never leaves Rust (§2.6).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; KEY_LEN]);

impl MasterKey {
    fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

/// Derive the master key from a password and a per-vault random salt.
///
/// `salt` should be a stored, per-vault random value of at least 16 bytes.
pub fn derive_master_key(
    password: &[u8],
    salt: &[u8],
    params: Argon2Params,
) -> Result<MasterKey, CryptoError> {
    if salt.len() < 16 {
        return Err(CryptoError::KeyDerivation);
    }
    let argon = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(
            params.mem_kib,
            params.iterations,
            params.parallelism,
            Some(KEY_LEN),
        )
        .map_err(|_| CryptoError::KeyDerivation)?,
    );
    let mut out = [0u8; KEY_LEN];
    argon
        .hash_password_into(password, salt, &mut out)
        .map_err(|_| CryptoError::KeyDerivation)?;
    Ok(MasterKey(out))
}

/// Purpose labels for HKDF `info`. Each derives an independent subkey (§2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubkeyLabel {
    /// Raw key handed to SQLCipher `PRAGMA key` (§2.6 — avoids a double KDF).
    SqlCipher,
    /// Key for the append-only encrypted sync-update log AEAD.
    SyncAead,
    /// Key used to wrap/unwrap the DEK from the password path.
    DekWrap,
}

impl SubkeyLabel {
    fn info(self) -> &'static [u8] {
        match self {
            SubkeyLabel::SqlCipher => b"notion.v1.sqlcipher",
            SubkeyLabel::SyncAead => b"notion.v1.sync-aead",
            SubkeyLabel::DekWrap => b"notion.v1.dek-wrap",
        }
    }
}

/// A single 256-bit HKDF-derived subkey. Zeroized on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SubKey([u8; KEY_LEN]);

impl SubKey {
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
    /// Lowercase hex, as SQLCipher expects for a raw key: `PRAGMA key = "x'..'"`.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

/// The three subkeys derived from a master key.
pub struct SubKeys {
    pub sqlcipher: SubKey,
    pub sync_aead: SubKey,
    pub dek_wrap: SubKey,
}

/// Expand a master key into purpose-separated subkeys via HKDF-SHA256 (§2.2).
pub fn subkeys(master: &MasterKey) -> SubKeys {
    let derive = |label: SubkeyLabel| -> SubKey {
        // No salt argument: the master key is already high-entropy Argon2id output,
        // so HKDF-Extract with an empty salt is the correct/standard usage here.
        let hk = Hkdf::<Sha256>::new(None, master.as_bytes());
        let mut okm = [0u8; KEY_LEN];
        // expand() only fails for absurd output lengths; KEY_LEN is fine.
        hk.expand(label.info(), &mut okm)
            .expect("HKDF expand of 32 bytes never fails");
        SubKey(okm)
    };
    SubKeys {
        sqlcipher: derive(SubkeyLabel::SqlCipher),
        sync_aead: derive(SubkeyLabel::SyncAead),
        dek_wrap: derive(SubkeyLabel::DekWrap),
    }
}

/// Derive a single labelled subkey from arbitrary 32-byte key material.
/// Used to turn an X25519 ECDH shared secret or a recovery key into an AEAD key.
pub(crate) fn hkdf_subkey(ikm: &[u8], info: &[u8]) -> [u8; KEY_LEN] {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut okm = [0u8; KEY_LEN];
    hk.expand(info, &mut okm)
        .expect("HKDF expand of 32 bytes never fails");
    okm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_key_is_deterministic() {
        let p = Argon2Params::insecure_fast();
        let salt = b"0123456789abcdef";
        let a = derive_master_key(b"correct horse", salt, p).unwrap();
        let b = derive_master_key(b"correct horse", salt, p).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn different_password_or_salt_changes_key() {
        let p = Argon2Params::insecure_fast();
        let base = derive_master_key(b"pw", b"0123456789abcdef", p).unwrap();
        let other_pw = derive_master_key(b"pw2", b"0123456789abcdef", p).unwrap();
        let other_salt = derive_master_key(b"pw", b"fedcba9876543210", p).unwrap();
        assert_ne!(base.as_bytes(), other_pw.as_bytes());
        assert_ne!(base.as_bytes(), other_salt.as_bytes());
    }

    #[test]
    fn short_salt_rejected() {
        let p = Argon2Params::insecure_fast();
        assert!(derive_master_key(b"pw", b"short", p).is_err());
    }

    #[test]
    fn subkeys_are_distinct_and_stable() {
        let p = Argon2Params::insecure_fast();
        let mk = derive_master_key(b"pw", b"0123456789abcdef", p).unwrap();
        let s1 = subkeys(&mk);
        let s2 = subkeys(&mk);
        // Stable across calls.
        assert_eq!(s1.sqlcipher.as_bytes(), s2.sqlcipher.as_bytes());
        // Distinct across labels (no correlation from slicing — §2.2).
        assert_ne!(s1.sqlcipher.as_bytes(), s1.sync_aead.as_bytes());
        assert_ne!(s1.sqlcipher.as_bytes(), s1.dek_wrap.as_bytes());
        assert_ne!(s1.sync_aead.as_bytes(), s1.dek_wrap.as_bytes());
    }

    #[test]
    fn sqlcipher_key_is_64_hex_chars() {
        let p = Argon2Params::insecure_fast();
        let mk = derive_master_key(b"pw", b"0123456789abcdef", p).unwrap();
        let hex = subkeys(&mk).sqlcipher.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
