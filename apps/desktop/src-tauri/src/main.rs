// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Desktop entry point. All application logic lives in the library crate
//! (`lib.rs`) so the exact same code powers the mobile (Android/iOS) targets,
//! where Tauri invokes the `mobile_entry_point`-annotated `run()` directly and
//! there is no `main`. Keeping `main` a one-liner is the canonical Tauri 2
//! desktop+mobile layout.

fn main() {
    notion_desktop_lib::run();
}
