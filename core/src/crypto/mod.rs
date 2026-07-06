//! Cryptographic core.
//!
//! This module implements the corrected key pipeline from the Blueprint Audit:
//!
//! * §2.2 — subkeys are derived with **HKDF-SHA256** using distinct `info`
//!   labels, never by slicing a master key.
//! * §2.3 — transport/at-rest confidentiality + integrity comes from the
//!   **XChaCha20-Poly1305 AEAD**; there is no redundant separate HMAC. When we
//!   need to authenticate routing metadata we bind it as AEAD *associated data*.
//! * §2.4 — every sealed message carries a fresh **random 24-byte nonce**
//!   (safe with XChaCha20's extended nonce) stored alongside the ciphertext.
//! * §2.1 — a random **data-encryption key (DEK)** is generated independently of
//!   the password and *wrapped* per device (X25519 sealed box). A per-user
//!   **Ed25519 identity** keypair backs relay auth + device enrollment.
//! * §2.5 — a printable **recovery kit** wraps the DEK so a forgotten password
//!   is not total, silent data loss.
//! * §2.6 — key material is held in [`zeroize`]-backed buffers and never handed
//!   to the JS/WebView layer.
//!
//! The design intentionally keeps the DEK independent of the password: a
//! password change re-wraps the DEK, it does not re-encrypt the database.

mod aead;
mod kdf;
mod keys;
mod pairing;
mod recovery;

pub use aead::{open, open_with_aad, seal, seal_with_aad, SealedBox, NONCE_LEN, TAG_LEN};
pub use kdf::{derive_master_key, subkeys, Argon2Params, MasterKey, SubKeys, SubkeyLabel};
pub use keys::{
    DataKey, DeviceKeypair, DevicePublicKey, IdentityKeypair, IdentityPublicKey, WrappedDek,
};
pub use pairing::{
    sas_code, verify_commitment, PairingCommitment, PairingContribution, PairingGrant, SAS_WORDS,
    SAS_WORD_COUNT,
};
pub use recovery::{RecoveryKit, RECOVERY_KEY_BYTES};

use thiserror::Error;

/// Errors surfaced by the crypto core. Variants are intentionally coarse so
/// they never echo secret-dependent detail (which could become an oracle).
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("key derivation failed")]
    KeyDerivation,
    #[error("authenticated encryption failed")]
    Encryption,
    #[error("authenticated decryption failed (wrong key, tampered data, or wrong nonce)")]
    Decryption,
    #[error("malformed ciphertext envelope")]
    MalformedEnvelope,
    #[error("invalid key material")]
    InvalidKey,
    #[error("invalid recovery key format")]
    InvalidRecoveryKey,
    #[error("signature verification failed")]
    BadSignature,
    #[error("degenerate (non-contributory) key agreement")]
    WeakKeyAgreement,
    #[error("pairing commitment did not match the revealed contribution")]
    PairingCommitmentMismatch,
}

/// Length in bytes of every symmetric key we derive/generate (256-bit).
pub const KEY_LEN: usize = 32;
