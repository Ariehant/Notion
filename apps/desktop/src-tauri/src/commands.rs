//! Tauri commands — the WebView's only entry points into the Rust core.
//!
//! All key material and crypto stay in Rust (audit §2.6); the WebView only ever
//! sees decrypted *content*, sanitized HTML, opaque update bytes, and — exactly
//! once, at creation — the recovery code. Updates are sealed with the DEK-rooted
//! sync-AEAD key (§2.3/§2.4) before they touch storage, so the same bytes can
//! later sync to a relay end-to-end encrypted.

use std::sync::MutexGuard;
use std::time::{SystemTime, UNIX_EPOCH};

use notion_core::crdt::{CrdtDocument, UpdateEncoding};
use notion_core::crypto::{open, seal, SealedBox};
use notion_core::db::{EncryptedDb, PageMeta};
use notion_core::sanitize;
use serde::Serialize;
use tauri::State;

use crate::state::AppState;
use crate::vault;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A page as sent to the WebView.
#[derive(Serialize)]
pub struct PageDto {
    pub id: String,
    pub title: String,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

impl From<PageMeta> for PageDto {
    fn from(p: PageMeta) -> Self {
        PageDto {
            id: p.id,
            title: p.title,
            created_at_ms: p.created_at_ms,
            updated_at_ms: p.updated_at_ms,
        }
    }
}

/// Borrow the open DB, or fail with a clear "locked" message.
fn db<'a>(guard: &'a MutexGuard<'a, Option<EncryptedDb>>) -> Result<&'a EncryptedDb, String> {
    guard.as_ref().ok_or_else(|| "vault is locked".to_string())
}

// ---------------------------------------------------------------------------
// Vault lifecycle
// ---------------------------------------------------------------------------

/// Whether a vault already exists on disk (decides create vs. unlock UI).
#[tauri::command]
pub fn vault_exists(state: State<AppState>) -> bool {
    vault::exists(&state.vault_dir)
}

/// Whether the vault is currently unlocked in memory.
#[tauri::command]
pub fn is_unlocked(state: State<AppState>) -> Result<bool, String> {
    Ok(state.db.lock().map_err(|_| "state poisoned")?.is_some())
}

/// Create a new vault; returns the one-time recovery code to show the user.
#[tauri::command]
pub fn create_vault(state: State<AppState>, password: String) -> Result<String, String> {
    let opened = vault::create(&state.vault_dir, &password).map_err(|e| e.to_string())?;
    let code = opened
        .recovery_code
        .clone()
        .ok_or("recovery code missing")?;
    state.install(opened)?;
    Ok(code)
}

/// Unlock an existing vault with the password.
#[tauri::command]
pub fn unlock_vault(state: State<AppState>, password: String) -> Result<(), String> {
    let opened = vault::unlock(&state.vault_dir, &password).map_err(|e| e.to_string())?;
    state.install(opened)
}

/// Reset the password using the recovery code (data is preserved).
#[tauri::command]
pub fn recover_vault(
    state: State<AppState>,
    recovery_code: String,
    new_password: String,
) -> Result<(), String> {
    let opened = vault::recover(&state.vault_dir, &recovery_code, &new_password)
        .map_err(|e| e.to_string())?;
    state.install(opened)
}

/// Lock the vault: drop all key material and close the DB.
#[tauri::command]
pub fn lock_vault(state: State<AppState>) -> Result<(), String> {
    state.clear()
}

// ---------------------------------------------------------------------------
// Pages
// ---------------------------------------------------------------------------

/// Create a page. The frontend supplies a fresh UUID that doubles as the doc id.
#[tauri::command]
pub fn create_page(state: State<AppState>, id: String, title: String) -> Result<PageDto, String> {
    let guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db(&guard)?;
    let now = now_ms();
    db.create_page(&id, &title, now)
        .map_err(|e| e.to_string())?;
    Ok(PageDto {
        id,
        title,
        created_at_ms: now,
        updated_at_ms: now,
    })
}

/// List all pages, most-recently-updated first.
#[tauri::command]
pub fn list_pages(state: State<AppState>) -> Result<Vec<PageDto>, String> {
    let guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db(&guard)?;
    Ok(db
        .list_pages()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(PageDto::from)
        .collect())
}

/// Rename a page.
#[tauri::command]
pub fn rename_page(state: State<AppState>, id: String, title: String) -> Result<(), String> {
    let guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db(&guard)?;
    db.rename_page(&id, &title, now_ms())
        .map_err(|e| e.to_string())
}

/// Permanently delete a page and all of its content.
#[tauri::command]
pub fn delete_page(state: State<AppState>, id: String) -> Result<(), String> {
    let guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db(&guard)?;
    db.delete_page(&id).map_err(|e| e.to_string())
}

/// (Re)index a page's plaintext for full-text search (called on save).
#[tauri::command]
pub fn index_page(
    state: State<AppState>,
    page_id: String,
    title: String,
    body: String,
) -> Result<(), String> {
    let guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db(&guard)?;
    db.index_page(&page_id, &title, &body)
        .map_err(|e| e.to_string())
}

/// Full-text search; returns matching page ids ranked by relevance.
#[tauri::command]
pub fn search_pages(state: State<AppState>, query: String) -> Result<Vec<String>, String> {
    let guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db(&guard)?;
    // Raw user input can contain FTS5 operators/quotes that would be a syntax
    // error. Reduce it to quoted prefix tokens (`"foo"* "bar"*`) so any input is
    // safe and search-as-you-type does prefix matching.
    match fts_prefix_query(&query) {
        Some(q) => db.search(&q).map_err(|e| e.to_string()),
        None => Ok(Vec::new()),
    }
}

/// Turn arbitrary user text into a safe FTS5 prefix query, or `None` if empty.
fn fts_prefix_query(raw: &str) -> Option<String> {
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.to_lowercase()))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

// ---------------------------------------------------------------------------
// Document persistence (§1.6) + version history (§1.3)
// ---------------------------------------------------------------------------

/// Seal + append a batch of encoded Yjs updates to the log (§1.6).
#[tauri::command]
pub fn persist_updates(
    state: State<AppState>,
    doc_id: String,
    updates: Vec<Vec<u8>>,
) -> Result<(), String> {
    let db_guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db(&db_guard)?;
    let key_guard = state.sync_key.lock().map_err(|_| "state poisoned")?;
    let key = key_guard.as_ref().ok_or("vault is locked")?;

    let now = now_ms();
    for update in updates {
        let sealed = seal(key, &update).map_err(|e| e.to_string())?;
        db.append_update(&doc_id, UpdateEncoding::V1, &sealed.to_bytes(), now)
            .map_err(|e| e.to_string())?;
    }
    // Bump the page's updated_at so the sidebar reflects recent activity.
    db.touch_page(&doc_id, now).map_err(|e| e.to_string())?;
    Ok(())
}

/// Load + decrypt all stored updates for a document (replayed into Yjs on open).
#[tauri::command]
pub fn load_updates(state: State<AppState>, doc_id: String) -> Result<Vec<Vec<u8>>, String> {
    let db_guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db(&db_guard)?;
    let key_guard = state.sync_key.lock().map_err(|_| "state poisoned")?;
    let key = key_guard.as_ref().ok_or("vault is locked")?;

    let stored = db.load_updates(&doc_id).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(stored.len());
    for s in stored {
        let sealed = SealedBox::from_bytes(&s.sealed).map_err(|e| e.to_string())?;
        out.push(open(key, &sealed).map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Take an explicit full-document restore point (§1.3).
#[tauri::command]
pub fn take_snapshot(
    state: State<AppState>,
    doc_id: String,
    label: Option<String>,
) -> Result<i64, String> {
    let db_guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db(&db_guard)?;
    let key_guard = state.sync_key.lock().map_err(|_| "state poisoned")?;
    let key = key_guard.as_ref().ok_or("vault is locked")?;

    // Rebuild the doc from the log, then store a full-document snapshot.
    let doc = CrdtDocument::new();
    for s in db.load_updates(&doc_id).map_err(|e| e.to_string())? {
        let sealed = SealedBox::from_bytes(&s.sealed).map_err(|e| e.to_string())?;
        let update = open(key, &sealed).map_err(|e| e.to_string())?;
        doc.apply_update_v1(&update).map_err(|e| e.to_string())?;
    }
    let snap = doc.snapshot(now_ms(), label.clone());
    let sealed = seal(key, &snap.state_v1).map_err(|e| e.to_string())?;
    db.save_snapshot(
        &doc_id,
        label.as_deref(),
        &sealed.to_bytes(),
        snap.created_at_ms,
    )
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Untrusted HTML (§2.7 / §2.8)
// ---------------------------------------------------------------------------

/// Sanitize untrusted HTML (pasted OR scraped) — one sanitizer for both (§2.8).
#[tauri::command]
pub fn sanitize_html(dirty: String) -> String {
    sanitize::sanitize_html(&dirty)
}

/// Build a locked-down, SSRF-guarded, sandboxed iframe for an embed (§2.7/§2.8).
#[tauri::command]
pub fn sandboxed_embed(src: String) -> Result<String, String> {
    sanitize::sandboxed_embed(&src).map_err(|e| e.to_string())
}
