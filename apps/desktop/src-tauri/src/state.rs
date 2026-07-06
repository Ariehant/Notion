use std::sync::Mutex;

use notion_core::db::EncryptedDb;

/// App-wide state. The encrypted DB handle and the AEAD sync key live here in
/// Rust and are never exposed to the WebView (audit §2.6).
#[derive(Default)]
pub struct AppState {
    pub db: Mutex<Option<EncryptedDb>>,
    /// The HKDF `sync-aead` subkey used to seal updates before storage/upload.
    pub sync_key: Mutex<Option<[u8; 32]>>,
}
