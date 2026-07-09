//! Shared Tauri application setup for **both desktop and mobile** (Android/iOS).
//!
//! All crypto/keys/DB access happens here in Rust (audit §2.6); only decrypted
//! *content*, sanitized HTML, or opaque bytes ever cross back to JS. `main.rs`
//! is a thin desktop entry point that calls [`run`]; on mobile, Tauri invokes
//! the `#[tauri::mobile_entry_point]`-annotated [`run`] directly (there is no
//! `main` on Android/iOS — the platform loads this crate as a shared library).
//!
//! The command layer, vault lifecycle, and Open Notebook AI wiring are identical
//! across platforms: the same `notion_core` engine, the same encrypted
//! `notion.db`. The only platform difference is the entry point and (on Android)
//! where `app_data_dir()` resolves — Tauri returns the app-private internal
//! storage path (`/data/data/co.merai.notion/…`), which needs no runtime
//! permission and is not world-readable.

mod ai;
mod commands;
mod state;
mod vault;

use state::AppState;
use tauri::Manager;

/// Build and run the Tauri application. Shared by the desktop binary and the
/// mobile (Android/iOS) library entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // The vault lives in the OS per-app data directory. On desktop this
            // is the usual per-user app-data path; on Android it is the
            // app-private internal storage directory (scoped-storage safe, no
            // permission required). Resolve it once and hand it to shared state.
            let dir = app.path().app_data_dir().expect("resolve app data dir");
            std::fs::create_dir_all(&dir).expect("create app data dir");
            app.manage(AppState::new(dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::vault_exists,
            commands::is_unlocked,
            commands::create_vault,
            commands::unlock_vault,
            commands::recover_vault,
            commands::lock_vault,
            commands::create_page,
            commands::list_pages,
            commands::rename_page,
            commands::delete_page,
            commands::index_page,
            commands::search_pages,
            commands::persist_updates,
            commands::load_updates,
            commands::take_snapshot,
            commands::sanitize_html,
            commands::sandboxed_embed,
            // Open Notebook AI (gated by ENABLE_OPEN_NOTEBOOK)
            ai::notebook_enabled,
            ai::semantic_search,
            ai::reindex_page,
            ai::ingest_text,
            ai::list_sources,
            ai::run_agent,
            ai::studio_summarize,
            ai::studio_transform,
            ai::list_agent_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Notion app");
}
