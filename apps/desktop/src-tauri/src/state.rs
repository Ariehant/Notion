use std::path::PathBuf;
use std::sync::Mutex;

use notion_core::db::EncryptedDb;
use zeroize::Zeroizing;

use crate::ai::Notebook;

/// App-wide state. The encrypted DB handle and the AEAD sync key live here in
/// Rust and are never exposed to the WebView (audit §2.6). The vault starts
/// locked (all `None`) until `create_vault`/`unlock_vault`/`recover_vault`.
pub struct AppState {
    /// Directory holding `vault.json` + `notion.db` (the OS app-data dir).
    pub vault_dir: PathBuf,
    pub db: Mutex<Option<EncryptedDb>>,
    /// The DEK-derived `sync-aead` key used to seal updates before storage.
    /// Kept in a zeroizing buffer and wiped on lock/replace (audit §2.6).
    pub sync_key: Mutex<Option<Zeroizing<[u8; 32]>>>,
    /// The Open Notebook AI services, bound to the same unlocked DB. `None` when
    /// locked or when `ENABLE_OPEN_NOTEBOOK` is unset (Phase 9 rollback flag).
    pub notebook: Mutex<Option<Notebook>>,
}

impl AppState {
    pub fn new(vault_dir: PathBuf) -> Self {
        Self {
            vault_dir,
            db: Mutex::new(None),
            sync_key: Mutex::new(None),
            notebook: Mutex::new(None),
        }
    }

    /// Replace the live DB handle + sync key after a successful open, and
    /// publish the SQLCipher key to the OS keyring so the GNOME companion can
    /// open the same encrypted file. Keyring publishing is best-effort: on a
    /// headless box (or with no Secret Service) the main app still works — the
    /// companion simply has no key until the next unlock in a graphical session.
    ///
    /// When `ENABLE_OPEN_NOTEBOOK` is set, this also opens the Open Notebook
    /// engine against the same `notion.db` (a second WAL connection) and runs its
    /// additive migrations. That is best-effort too: a failure logs and leaves
    /// the AI features unavailable rather than blocking unlock.
    pub fn install(&self, opened: crate::vault::OpenVault) -> Result<(), String> {
        if let Err(e) = notion_companion::keyring::store_key_hex(&opened.sqlcipher_key_hex) {
            eprintln!("notion: could not publish key to keyring (companion disabled): {e}");
        }
        if crate::ai::enabled() {
            match Notebook::open(&self.vault_dir, &opened.sqlcipher_key_hex) {
                Ok(nb) => *self.notebook.lock().map_err(|_| "state poisoned")? = Some(nb),
                Err(e) => eprintln!("notion: Open Notebook unavailable: {e}"),
            }
        }
        *self.db.lock().map_err(|_| "state poisoned")? = Some(opened.db);
        *self.sync_key.lock().map_err(|_| "state poisoned")? = Some(opened.sync_key);
        Ok(())
    }

    /// Drop all key material + close the DB (lock the vault) and remove the key
    /// from the OS keyring so the companion also locks. Keyring removal is
    /// best-effort for the same reason as publishing.
    pub fn clear(&self) -> Result<(), String> {
        if let Err(e) = notion_companion::keyring::clear_key() {
            eprintln!("notion: could not clear key from keyring: {e}");
        }
        *self.notebook.lock().map_err(|_| "state poisoned")? = None;
        *self.db.lock().map_err(|_| "state poisoned")? = None;
        *self.sync_key.lock().map_err(|_| "state poisoned")? = None;
        Ok(())
    }
}
