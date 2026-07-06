// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri command layer. The WebView calls these; all crypto/keys/DB access
//! happens here in Rust (audit §2.6) and only decrypted *content*, sanitized
//! HTML, or opaque bytes cross back to JS.

mod commands;
mod state;

use state::AppState;

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::persist_updates,
            commands::load_updates,
            commands::sanitize_html,
            commands::sandboxed_embed,
            commands::take_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Notion desktop app");
}
