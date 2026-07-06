//! Recovery kit — audit §2.5.
//!
//! A password-derived key with no recovery path means a forgotten password is
//! **total, silent data loss**. The audit requires an explicit, disclosed
//! recovery mechanism. Here a random 256-bit recovery secret wraps the DEK; its
//! printable form is the "recovery kit" the user stores offline. The recovery
//! secret is independent of the password, so it keeps working across password
//! changes.

use rand::rngs::OsRng;
use rand::RngCore;
use zeroize::Zeroize;

use super::aead::{open, seal, SealedBox};
use super::kdf::hkdf_subkey;
use super::keys::DataKey;
use super::{CryptoError, KEY_LEN};

/// Length of the raw recovery secret (256-bit).
pub const RECOVERY_KEY_BYTES: usize = 32;

/// The recovery kit: a printable code the user keeps, plus the DEK wrapped under
/// the key derived from that code.
pub struct RecoveryKit {
    /// Human-facing recovery code, grouped hex (e.g. `A1B2-C3D4-…`). Show once.
    pub printable_code: String,
    /// The DEK sealed under the recovery key.
    pub wrapped_dek: SealedBox,
}

impl RecoveryKit {
    /// Create a fresh recovery kit for `dek`. Show `printable_code` to the user
    /// exactly once and persist only `wrapped_dek`.
    pub fn create(dek: &DataKey) -> Result<Self, CryptoError> {
        let mut secret = [0u8; RECOVERY_KEY_BYTES];
        OsRng.fill_bytes(&mut secret);

        let wrap_key = hkdf_subkey(&secret, b"notion.v1.recovery");
        let wrapped_dek = seal(&wrap_key, dek.as_bytes())?;
        let printable_code = format_code(&secret);

        secret.zeroize();
        Ok(RecoveryKit {
            printable_code,
            wrapped_dek,
        })
    }

    /// Recover the DEK from a user-entered code + the stored wrapped DEK.
    /// Tolerant of dashes, whitespace, and letter case in the entered code.
    pub fn recover(entered_code: &str, wrapped_dek: &SealedBox) -> Result<DataKey, CryptoError> {
        let mut secret = parse_code(entered_code)?;
        let wrap_key = hkdf_subkey(&secret, b"notion.v1.recovery");
        secret.zeroize();
        let bytes = open(&wrap_key, wrapped_dek)?;
        let arr: [u8; KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::InvalidKey)?;
        Ok(DataKey::from_bytes(arr))
    }
}

/// Render the recovery secret as uppercase hex grouped in fours: `A1B2-C3D4-…`.
fn format_code(secret: &[u8; RECOVERY_KEY_BYTES]) -> String {
    let hex = hex::encode_upper(secret);
    hex.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join("-")
}

/// Parse a user-entered recovery code back to the raw secret.
fn parse_code(code: &str) -> Result<[u8; RECOVERY_KEY_BYTES], CryptoError> {
    let cleaned: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let bytes = hex::decode(cleaned).map_err(|_| CryptoError::InvalidRecoveryKey)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::InvalidRecoveryKey)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_round_trip() {
        let dek = DataKey::generate();
        let kit = RecoveryKit::create(&dek).unwrap();
        let recovered = RecoveryKit::recover(&kit.printable_code, &kit.wrapped_dek).unwrap();
        assert_eq!(dek.as_bytes(), recovered.as_bytes());
    }

    #[test]
    fn recovery_tolerates_formatting_noise() {
        let dek = DataKey::generate();
        let kit = RecoveryKit::create(&dek).unwrap();
        // User retypes with spaces, lowercase, and no dashes.
        let messy = kit.printable_code.replace('-', " ").to_lowercase();
        let recovered = RecoveryKit::recover(&messy, &kit.wrapped_dek).unwrap();
        assert_eq!(dek.as_bytes(), recovered.as_bytes());
    }

    #[test]
    fn wrong_code_fails() {
        let dek = DataKey::generate();
        let kit = RecoveryKit::create(&dek).unwrap();
        let wrong =
            "0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000";
        assert!(RecoveryKit::recover(wrong, &kit.wrapped_dek).is_err());
    }

    #[test]
    fn malformed_code_rejected() {
        let dek = DataKey::generate();
        let kit = RecoveryKit::create(&dek).unwrap();
        assert!(RecoveryKit::recover("not-a-valid-code", &kit.wrapped_dek).is_err());
    }

    #[test]
    fn printable_code_has_expected_shape() {
        let dek = DataKey::generate();
        let kit = RecoveryKit::create(&dek).unwrap();
        // 64 hex chars in 16 groups of 4 joined by 15 dashes.
        assert_eq!(kit.printable_code.len(), 64 + 15);
        assert_eq!(kit.printable_code.matches('-').count(), 15);
    }
}
