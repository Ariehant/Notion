//! Where the companion gets the SQLCipher key.
//!
//! The main app derives the raw SQLCipher key from the DEK
//! (`DataKey::content_keys().sqlcipher_hex()`) and, on unlock, publishes *only
//! that key* to the GNOME Keyring / Secret Service. The companion retrieves it
//! and opens the shared DB. Publishing the derived DB key (not the DEK root)
//! is least-privilege: the companion can read/write calendar rows but cannot
//! unwrap the CRDT sync log or any other DEK-derived secret.
//!
//! [`KeyProvider`] abstracts the source so tests and headless runs use an
//! in-memory or env-var key, while the real Secret Service backend hides behind
//! the `keyring` feature.

use thiserror::Error;
use zeroize::Zeroizing;

/// Secret Service service name (shared by every component).
pub const KEYRING_SERVICE: &str = "co.merai.notion";
/// Secret Service account/attribute identifying the SQLCipher key entry.
pub const KEYRING_ACCOUNT: &str = "sqlcipher-key";
/// Environment variable an [`EnvKeyProvider`] reads (dev / headless override).
pub const KEY_ENV_VAR: &str = "NOTION_SQLCIPHER_KEY_HEX";

#[derive(Debug, Error)]
pub enum KeyError {
    #[error("the vault is locked (no key available)")]
    Locked,
    #[error("keyring backend error: {0}")]
    Backend(String),
    #[error("the stored key is not 64 lowercase hex characters")]
    Malformed,
}

/// Validate a 256-bit raw key in the exact shape SQLCipher / `EncryptedDb`
/// expects, so a corrupt keyring entry fails fast rather than deep in SQLite.
pub fn validate_key_hex(hex: &str) -> Result<(), KeyError> {
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(KeyError::Malformed)
    }
}

/// A source of the shared SQLCipher key.
pub trait KeyProvider {
    /// The 64-hex SQLCipher key, or `None` if the vault is currently locked.
    fn sqlcipher_key_hex(&self) -> Result<Option<Zeroizing<String>>, KeyError>;
}

/// A fixed key, primarily for tests and for wiring a known key in.
pub struct StaticKeyProvider(Option<Zeroizing<String>>);

impl StaticKeyProvider {
    pub fn new(key_hex: impl Into<String>) -> Result<Self, KeyError> {
        let s = key_hex.into();
        validate_key_hex(&s)?;
        Ok(StaticKeyProvider(Some(Zeroizing::new(s))))
    }

    /// A provider that always reports "locked".
    pub fn locked() -> Self {
        StaticKeyProvider(None)
    }
}

impl KeyProvider for StaticKeyProvider {
    fn sqlcipher_key_hex(&self) -> Result<Option<Zeroizing<String>>, KeyError> {
        Ok(self.0.clone())
    }
}

/// Reads the key from [`KEY_ENV_VAR`]. Useful for development and for headless
/// integration runs where a session keyring is unavailable.
pub struct EnvKeyProvider;

impl KeyProvider for EnvKeyProvider {
    fn sqlcipher_key_hex(&self) -> Result<Option<Zeroizing<String>>, KeyError> {
        match std::env::var(KEY_ENV_VAR) {
            Ok(v) if !v.is_empty() => {
                validate_key_hex(&v)?;
                Ok(Some(Zeroizing::new(v)))
            }
            _ => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Real Secret Service backend (feature `keyring`)
// ---------------------------------------------------------------------------

/// Reads/writes the key in the OS keyring (GNOME Keyring via Secret Service).
#[cfg(feature = "keyring")]
pub struct SecretServiceKeyProvider;

#[cfg(feature = "keyring")]
impl SecretServiceKeyProvider {
    fn entry() -> Result<::keyring::Entry, KeyError> {
        ::keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .map_err(|e| KeyError::Backend(e.to_string()))
    }
}

#[cfg(feature = "keyring")]
impl KeyProvider for SecretServiceKeyProvider {
    fn sqlcipher_key_hex(&self) -> Result<Option<Zeroizing<String>>, KeyError> {
        match Self::entry()?.get_password() {
            Ok(v) => {
                validate_key_hex(&v)?;
                Ok(Some(Zeroizing::new(v)))
            }
            Err(::keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeyError::Backend(e.to_string())),
        }
    }
}

/// Publish the SQLCipher key to the OS keyring (called by the main app on
/// unlock). The key is validated first so we never store garbage.
#[cfg(feature = "keyring")]
pub fn store_key_hex(key_hex: &str) -> Result<(), KeyError> {
    validate_key_hex(key_hex)?;
    SecretServiceKeyProvider::entry()?
        .set_password(key_hex)
        .map_err(|e| KeyError::Backend(e.to_string()))
}

/// Remove the SQLCipher key from the OS keyring (called by the main app on
/// lock). A missing entry is treated as success (idempotent lock).
#[cfg(feature = "keyring")]
pub fn clear_key() -> Result<(), KeyError> {
    match SecretServiceKeyProvider::entry()?.delete_credential() {
        Ok(()) | Err(::keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(KeyError::Backend(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn validates_key_shape() {
        assert!(validate_key_hex(GOOD).is_ok());
        assert!(matches!(
            validate_key_hex("short"),
            Err(KeyError::Malformed)
        ));
        assert!(matches!(
            validate_key_hex(&"z".repeat(64)),
            Err(KeyError::Malformed)
        ));
    }

    #[test]
    fn static_provider_round_trips() {
        let p = StaticKeyProvider::new(GOOD).unwrap();
        assert_eq!(p.sqlcipher_key_hex().unwrap().unwrap().as_str(), GOOD);
        assert!(StaticKeyProvider::locked()
            .sqlcipher_key_hex()
            .unwrap()
            .is_none());
        assert!(StaticKeyProvider::new("nope").is_err());
    }

    #[test]
    fn env_provider_reads_and_validates() {
        // Use a unique guard around the shared env var.
        let prev = std::env::var_os(KEY_ENV_VAR);
        std::env::set_var(KEY_ENV_VAR, GOOD);
        assert_eq!(
            EnvKeyProvider
                .sqlcipher_key_hex()
                .unwrap()
                .unwrap()
                .as_str(),
            GOOD
        );
        std::env::set_var(KEY_ENV_VAR, "garbage");
        assert!(EnvKeyProvider.sqlcipher_key_hex().is_err());
        std::env::remove_var(KEY_ENV_VAR);
        assert!(EnvKeyProvider.sqlcipher_key_hex().unwrap().is_none());
        if let Some(v) = prev {
            std::env::set_var(KEY_ENV_VAR, v);
        }
    }
}
