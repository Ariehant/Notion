//! Vault lifecycle — the piece that turns the crypto core into a usable app.
//!
//! On disk a vault is two files in the app-data directory:
//!   * `notion.db`   — the SQLCipher-encrypted database (opaque page-encrypted).
//!   * `vault.json`  — **non-secret** metadata: the KDF salt/params and the DEK
//!     wrapped two ways (under the password path and under the recovery key).
//!     Salts and wrapped ciphertext are safe to store in the clear.
//!
//! ## Key hierarchy (audit §2.1 / §2.5) — the DEK is the real root
//!
//! ```text
//! password ─Argon2id(salt)→ master ─HKDF→ dek_wrap subkey ─┐
//!                                                          ├─ unwrap ─▶ DEK
//! recovery code ────────────HKDF──────────────────────────┘
//!
//! DEK ─HKDF→ sqlcipher key (raw DB key)   ← content keys hang off the DEK,
//!     └HKDF→ sync-aead key (seal updates)    NOT the password.
//! ```
//!
//! Because the content keys are derived from the DEK, a password reset only
//! re-wraps the DEK (see [`recover`]); it never re-keys the database.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use notion_core::crypto::{
    derive_master_key, subkeys, Argon2Params, DataKey, RecoveryKit, SealedBox,
};
use notion_core::db::EncryptedDb;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

const VAULT_FILE: &str = "vault.json";
const DB_FILE: &str = "notion.db";
const SALT_LEN: usize = 16;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("no vault exists yet")]
    Missing,
    #[error("a vault already exists")]
    AlreadyExists,
    #[error("incorrect password")]
    BadPassword,
    #[error("invalid or non-matching recovery code")]
    BadRecoveryCode,
    #[error("vault metadata is corrupt: {0}")]
    Corrupt(String),
    #[error("crypto error: {0}")]
    Crypto(#[from] notion_core::crypto::CryptoError),
    #[error("database error: {0}")]
    Db(#[from] notion_core::db::DbError),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
}

/// On-disk, non-secret vault metadata. Everything here is either public (salt,
/// KDF params) or ciphertext (the two wrapped DEKs), so storing it in cleartext
/// leaks nothing about the password or the data.
#[derive(Serialize, Deserialize)]
struct VaultMeta {
    version: u32,
    argon_mem_kib: u32,
    argon_iterations: u32,
    argon_parallelism: u32,
    salt_hex: String,
    /// DEK sealed under the password-derived `dek_wrap` subkey.
    wrapped_dek_password_hex: String,
    /// DEK sealed under the recovery key (independent of the password).
    wrapped_dek_recovery_hex: String,
}

impl VaultMeta {
    fn argon(&self) -> Argon2Params {
        Argon2Params {
            mem_kib: self.argon_mem_kib,
            iterations: self.argon_iterations,
            parallelism: self.argon_parallelism,
        }
    }
}

/// The result of opening a vault: an unlocked DB handle and the sync-AEAD key,
/// plus (only on creation) the one-time recovery code to show the user.
pub struct OpenVault {
    pub db: EncryptedDb,
    /// Held in a zeroizing buffer so the key is wiped on drop (audit §2.6).
    pub sync_key: Zeroizing<[u8; 32]>,
    /// The raw SQLCipher key (64-hex) published to the OS keyring so the GNOME
    /// companion daemon/app can open the same encrypted file. Zeroized on drop;
    /// this is the *derived DB key*, not the DEK root (least privilege).
    pub sqlcipher_key_hex: Zeroizing<String>,
    pub recovery_code: Option<String>,
}

fn vault_path(dir: &Path) -> PathBuf {
    dir.join(VAULT_FILE)
}

fn db_path(dir: &Path) -> PathBuf {
    dir.join(DB_FILE)
}

/// Whether a vault already exists in `dir`.
pub fn exists(dir: &Path) -> bool {
    vault_path(dir).is_file()
}

fn read_meta(dir: &Path) -> Result<VaultMeta, VaultError> {
    let raw = fs::read_to_string(vault_path(dir)).map_err(|_| VaultError::Missing)?;
    serde_json::from_str(&raw).map_err(|e| VaultError::Corrupt(e.to_string()))
}

fn write_meta(dir: &Path, meta: &VaultMeta) -> Result<(), VaultError> {
    let json =
        serde_json::to_string_pretty(meta).map_err(|e| VaultError::Corrupt(e.to_string()))?;
    // Durable atomic replace. vault.json is the ONLY persistent copy of the
    // wrapped DEK, so we (1) write the temp file and fsync its bytes, (2) rename
    // it over the real file, (3) fsync the directory so the rename is durable.
    // Without the fsyncs, power loss could land the rename while the temp bytes
    // are still buffered, leaving a truncated vault.json and an unrecoverable DB.
    let tmp = vault_path(dir).with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, vault_path(dir))?;
    if let Ok(dir_file) = fs::File::open(dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

fn random_salt() -> Result<[u8; SALT_LEN], VaultError> {
    let mut salt = [0u8; SALT_LEN];
    getrandom::getrandom(&mut salt)
        .map_err(|e| VaultError::Corrupt(format!("rng unavailable: {e}")))?;
    Ok(salt)
}

fn decode_sealed(hex_str: &str) -> Result<SealedBox, VaultError> {
    let bytes = hex::decode(hex_str).map_err(|e| VaultError::Corrupt(e.to_string()))?;
    SealedBox::from_bytes(&bytes).map_err(VaultError::from)
}

/// Open a DB from a DEK: derive the content keys and hand SQLCipher the raw key.
fn open_db_with_dek(dir: &Path, dek: &DataKey) -> Result<OpenVault, VaultError> {
    let content = dek.content_keys();
    let sqlcipher_key_hex = Zeroizing::new(content.sqlcipher_hex());
    let db = EncryptedDb::open(&db_path(dir).to_string_lossy(), &sqlcipher_key_hex)?;
    Ok(OpenVault {
        db,
        sync_key: Zeroizing::new(content.sync_aead),
        sqlcipher_key_hex,
        recovery_code: None,
    })
}

/// Create a brand-new vault protected by `password`. Returns the opened vault
/// with a one-time `recovery_code` the caller must show the user exactly once.
pub fn create(dir: &Path, password: &str) -> Result<OpenVault, VaultError> {
    if exists(dir) {
        return Err(VaultError::AlreadyExists);
    }
    fs::create_dir_all(dir)?;

    let params = Argon2Params::default();
    let salt = random_salt()?;
    let master = derive_master_key(password.as_bytes(), &salt, params)?;
    let dek_wrap = subkeys(&master).dek_wrap;

    // A fresh, password-independent DEK is the root of all content encryption.
    let dek = DataKey::generate();
    let wrapped_pw = dek.wrap_with_key(dek_wrap.as_bytes())?;
    let kit = RecoveryKit::create(&dek)?;

    let meta = VaultMeta {
        version: 1,
        argon_mem_kib: params.mem_kib,
        argon_iterations: params.iterations,
        argon_parallelism: params.parallelism,
        salt_hex: hex::encode(salt),
        wrapped_dek_password_hex: hex::encode(wrapped_pw.to_bytes()),
        wrapped_dek_recovery_hex: hex::encode(kit.wrapped_dek.to_bytes()),
    };

    // Persist the wrapped-DEK metadata FIRST. vault.json is the only durable
    // copy of the DEK, so if the process dies before the DB is created, the
    // vault still "exists" and unlock() re-derives the DEK from the password
    // wrap and creates notion.db with the correct key on the next run. (The old
    // order could strand an encrypted notion.db whose wraps were never written,
    // bricking the directory since exists() keys off vault.json.)
    write_meta(dir, &meta)?;
    let opened = open_db_with_dek(dir, &dek)?;

    Ok(OpenVault {
        recovery_code: Some(kit.printable_code),
        ..opened
    })
}

/// Unlock an existing vault with `password`.
pub fn unlock(dir: &Path, password: &str) -> Result<OpenVault, VaultError> {
    let meta = read_meta(dir)?;
    let salt = hex::decode(&meta.salt_hex).map_err(|e| VaultError::Corrupt(e.to_string()))?;
    let master = derive_master_key(password.as_bytes(), &salt, meta.argon())?;
    let dek_wrap = subkeys(&master).dek_wrap;
    let wrapped = decode_sealed(&meta.wrapped_dek_password_hex)?;
    // A wrong password makes the AEAD open fail — surface it as BadPassword, not
    // a generic crypto error, so the UI can say the right thing.
    let dek = DataKey::unwrap_with_key(dek_wrap.as_bytes(), &wrapped)
        .map_err(|_| VaultError::BadPassword)?;
    open_db_with_dek(dir, &dek)
}

/// Reset the password using the recovery code. Recovers the DEK via the
/// recovery wrap, then re-wraps it under a freshly salted `new_password` — the
/// database and its content are untouched (the DEK is unchanged).
pub fn recover(
    dir: &Path,
    recovery_code: &str,
    new_password: &str,
) -> Result<OpenVault, VaultError> {
    let mut meta = read_meta(dir)?;
    let wrapped_recovery = decode_sealed(&meta.wrapped_dek_recovery_hex)?;
    let dek = RecoveryKit::recover(recovery_code, &wrapped_recovery)
        .map_err(|_| VaultError::BadRecoveryCode)?;

    // Re-wrap the (unchanged) DEK under the new password with a fresh salt.
    let params = Argon2Params::default();
    let salt = random_salt()?;
    let master = derive_master_key(new_password.as_bytes(), &salt, params)?;
    let dek_wrap = subkeys(&master).dek_wrap;
    let wrapped_pw = dek.wrap_with_key(dek_wrap.as_bytes())?;

    meta.argon_mem_kib = params.mem_kib;
    meta.argon_iterations = params.iterations;
    meta.argon_parallelism = params.parallelism;
    meta.salt_hex = hex::encode(salt);
    meta.wrapped_dek_password_hex = hex::encode(wrapped_pw.to_bytes());

    let opened = open_db_with_dek(dir, &dek)?;
    write_meta(dir, &meta)?;
    Ok(opened)
}

#[cfg(test)]
mod tests {
    use super::*;
    use notion_core::crdt::UpdateEncoding;
    use notion_core::crypto::{open, seal, SealedBox};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "notion_vault_test_{}_{}",
                std::process::id(),
                n
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // Seal a byte string with the sync key and store it, the way a command would.
    fn store_update(v: &OpenVault, doc: &str, plaintext: &[u8]) {
        let sealed = seal(&v.sync_key, plaintext).unwrap();
        v.db.append_update(doc, UpdateEncoding::V1, &sealed.to_bytes(), 1)
            .unwrap();
    }

    fn read_one_update(v: &OpenVault, doc: &str) -> Vec<u8> {
        let stored = v.db.load_updates(doc).unwrap();
        let sealed = SealedBox::from_bytes(&stored[0].sealed).unwrap();
        open(&v.sync_key, &sealed).unwrap()
    }

    #[test]
    fn create_then_unlock_preserves_data() {
        let tmp = TempDir::new();
        assert!(!exists(&tmp.0));

        let created = create(&tmp.0, "correct horse battery").unwrap();
        assert!(exists(&tmp.0));
        assert!(created.recovery_code.is_some());
        created.db.create_page("p1", "Hello", 1).unwrap();
        store_update(&created, "p1", b"secret body");
        drop(created);

        // Reopen with the right password: the DEK-derived keys must reproduce
        // both the SQLCipher key (DB opens) and the sync key (update decrypts).
        let opened = unlock(&tmp.0, "correct horse battery").unwrap();
        assert!(opened.recovery_code.is_none());
        assert_eq!(opened.db.list_pages().unwrap()[0].title, "Hello");
        assert_eq!(read_one_update(&opened, "p1"), b"secret body");
    }

    #[test]
    fn wrong_password_is_rejected() {
        let tmp = TempDir::new();
        create(&tmp.0, "right-password").unwrap();
        assert!(matches!(
            unlock(&tmp.0, "wrong-password"),
            Err(VaultError::BadPassword)
        ));
    }

    #[test]
    fn recovery_resets_password_without_touching_data() {
        let tmp = TempDir::new();
        let created = create(&tmp.0, "original-pass").unwrap();
        let code = created.recovery_code.clone().unwrap();
        created.db.create_page("p1", "Kept", 1).unwrap();
        store_update(&created, "p1", b"kept body");
        drop(created);

        // Recover with a new password; data is preserved (DEK unchanged).
        let recovered = recover(&tmp.0, &code, "brand-new-pass").unwrap();
        assert_eq!(recovered.db.list_pages().unwrap()[0].title, "Kept");
        assert_eq!(read_one_update(&recovered, "p1"), b"kept body");
        drop(recovered);

        // New password now unlocks; the old one no longer does.
        assert!(unlock(&tmp.0, "brand-new-pass").is_ok());
        assert!(matches!(
            unlock(&tmp.0, "original-pass"),
            Err(VaultError::BadPassword)
        ));

        // A bogus recovery code is rejected.
        let bad = "0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000";
        assert!(matches!(
            recover(&tmp.0, bad, "whatever-pass"),
            Err(VaultError::BadRecoveryCode)
        ));
    }

    #[test]
    fn missing_vault_reports_missing() {
        let tmp = TempDir::new();
        assert!(matches!(unlock(&tmp.0, "x"), Err(VaultError::Missing)));
    }
}
