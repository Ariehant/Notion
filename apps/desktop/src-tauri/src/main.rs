// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri command layer. The WebView calls these; all crypto/keys/DB access
//! happens here in Rust (audit §2.6) and only decrypted *content*, sanitized
//! HTML, or opaque bytes cross back to JS.

mod commands;
mod state;
mod vault;

use state::AppState;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // The vault lives in the OS per-app data directory. Resolve it once
            // at startup and hand it to the shared state.
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Notion desktop app");
}
