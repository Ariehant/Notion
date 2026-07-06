//! Tauri commands — the WebView's only entry points into the Rust core.
//!
//! Updates are sealed with the AEAD sync key (§2.3/§2.4) before they touch
//! storage, so the same bytes can later sync to the relay end-to-end encrypted.

use std::time::{SystemTime, UNIX_EPOCH};

use notion_core::crdt::{CrdtDocument, UpdateEncoding};
use notion_core::crypto::{open, seal, SealedBox};
use notion_core::sanitize;
use tauri::State;

use crate::state::AppState;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Seal + append a batch of encoded Yjs updates to the log (§1.6).
#[tauri::command]
pub fn persist_updates(
    state: State<AppState>,
    doc_id: String,
    updates: Vec<Vec<u8>>,
) -> Result<(), String> {
    let db_guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db_guard.as_ref().ok_or("database not open")?;
    let key_guard = state.sync_key.lock().map_err(|_| "state poisoned")?;
    let key = key_guard.as_ref().ok_or("vault locked")?;

    let now = now_ms();
    for update in updates {
        let sealed = seal(key, &update).map_err(|e| e.to_string())?;
        db.append_update(&doc_id, UpdateEncoding::V1, &sealed.to_bytes(), now)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Load + decrypt all stored updates for a document (replayed into Yjs on open).
#[tauri::command]
pub fn load_updates(state: State<AppState>, doc_id: String) -> Result<Vec<Vec<u8>>, String> {
    let db_guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db_guard.as_ref().ok_or("database not open")?;
    let key_guard = state.sync_key.lock().map_err(|_| "state poisoned")?;
    let key = key_guard.as_ref().ok_or("vault locked")?;

    let stored = db.load_updates(&doc_id).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(stored.len());
    for s in stored {
        let sealed = SealedBox::from_bytes(&s.sealed).map_err(|e| e.to_string())?;
        out.push(open(key, &sealed).map_err(|e| e.to_string())?);
    }
    Ok(out)
}

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

/// Take an explicit full-document restore point (§1.3).
#[tauri::command]
pub fn take_snapshot(
    state: State<AppState>,
    doc_id: String,
    label: Option<String>,
) -> Result<i64, String> {
    let db_guard = state.db.lock().map_err(|_| "state poisoned")?;
    let db = db_guard.as_ref().ok_or("database not open")?;
    let key_guard = state.sync_key.lock().map_err(|_| "state poisoned")?;
    let key = key_guard.as_ref().ok_or("vault locked")?;

    // Rebuild the doc from the log, then store a full-document snapshot.
    let doc = CrdtDocument::new();
    for s in db.load_updates(&doc_id).map_err(|e| e.to_string())? {
        let sealed = SealedBox::from_bytes(&s.sealed).map_err(|e| e.to_string())?;
        let update = open(key, &sealed).map_err(|e| e.to_string())?;
        doc.apply_update_v1(&update).map_err(|e| e.to_string())?;
    }
    let snap = doc.snapshot(now_ms(), label.clone());
    let sealed = seal(key, &snap.state_v1).map_err(|e| e.to_string())?;
    db.save_snapshot(&doc_id, label.as_deref(), &sealed.to_bytes(), snap.created_at_ms)
        .map_err(|e| e.to_string())
}
