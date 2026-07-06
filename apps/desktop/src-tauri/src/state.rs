use std::path::PathBuf;
use std::sync::Mutex;

use notion_core::db::EncryptedDb;
use zeroize::Zeroizing;

/// App-wide state. The encrypted DB handle and the AEAD sync key live here in
/// Rust and are never exposed to the WebView (audit §2.6). The vault starts
/// locked (both `None`) until `create_vault`/`unlock_vault`/`recover_vault`.
pub struct AppState {
    /// Directory holding `vault.json` + `notion.db` (the OS app-data dir).
    pub vault_dir: PathBuf,
    pub db: Mutex<Option<EncryptedDb>>,
    /// The DEK-derived `sync-aead` key used to seal updates before storage.
    /// Kept in a zeroizing buffer and wiped on lock/replace (audit §2.6).
    pub sync_key: Mutex<Option<Zeroizing<[u8; 32]>>>,
}

impl AppState {
    pub fn new(vault_dir: PathBuf) -> Self {
        Self {
            vault_dir,
            db: Mutex::new(None),
            sync_key: Mutex::new(None),
        }
    }

    /// Replace the live DB handle + sync key after a successful open.
    pub fn install(&self, opened: crate::vault::OpenVault) -> Result<(), String> {
        *self.db.lock().map_err(|_| "state poisoned")? = Some(opened.db);
        *self.sync_key.lock().map_err(|_| "state poisoned")? = Some(opened.sync_key);
        Ok(())
    }

    /// Drop all key material + close the DB (lock the vault).
    pub fn clear(&self) -> Result<(), String> {
        *self.db.lock().map_err(|_| "state poisoned")? = None;
        *self.sync_key.lock().map_err(|_| "state poisoned")? = None;
        Ok(())
    }
}
