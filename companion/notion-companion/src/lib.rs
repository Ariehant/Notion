//! `notion_companion` — shared logic for the GNOME Companion Calendar & Dynamic
//! Island ecosystem (Ubuntu 24.04+).
//!
//! The companion is a set of small, native processes that share the main app's
//! encrypted SQLite file instead of booting a second WebView:
//!
//! * **Component A** — a Rust DBus watcher daemon (`notion-watcher`) that reads
//!   the shared DB read-only, watches it with `inotify`, and broadcasts an
//!   `EventsUpdated` DBus signal.
//! * **Component B** — a GNOME Shell extension (GJS) that renders today's agenda
//!   in the top-bar calendar drop-down and updates live from the DBus signal.
//! * **Component C** — a GTK4/libadwaita quick-view app (`notion-quickview`) for
//!   week/agenda browsing, quick-add, and the local-AI "Ask" mode.
//!
//! This crate is the shared, testable heart of all three. It is deliberately
//! split so headless CI exercises the logic that has no GUI dependency:
//!
//! | Module        | What it owns                                             |
//! |---------------|----------------------------------------------------------|
//! | [`event`]     | The `CompanionEvent` model + JSON wire format (DBus/AI)  |
//! | [`time`]      | Local-day boundary math + `YYYY-MM-DD HH:MM` parsing      |
//! | [`ai`]        | Ollama prompt building, JSON parsing, validation, conflict detection |
//! | [`paths`]     | XDG resolution of the shared vault dir / DB path         |
//! | [`keyring`]   | `KeyProvider` trait + env/static/Secret-Service backends |
//! | [`dbaccess`]  | (feature `sqlcipher`) open the shared DB read-only/read-write |
//!
//! The DB-touching pieces live behind the `sqlcipher` feature so the fast CI
//! layer builds without linking a bundled SQLCipher, matching `notion_core`.

pub mod ai;
pub mod event;
pub mod keyring;
pub mod paths;
pub mod time;

#[cfg(feature = "sqlcipher")]
pub mod dbaccess;

#[cfg(test)]
pub(crate) mod testutil {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// Serializes tests that mutate process-wide environment variables
    /// (`XDG_DATA_HOME`, `HOME`, …) so they never race across threads.
    pub fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        // Recover a poisoned lock: a panicking test still leaves the env in a
        // known state via its own restore, and we only need mutual exclusion.
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
}
