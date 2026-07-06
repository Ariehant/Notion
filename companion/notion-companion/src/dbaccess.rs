//! Opening the shared encrypted database from a companion process.
//!
//! The daemon opens it **read-only** (`open_query_only`); the GTK quick-view
//! opens it **read/write** so quick-add / AI-add land immediately and the main
//! app (and daemon, via `inotify`) see the change. Both derive the same key from
//! a [`KeyProvider`], so there is exactly one encrypted file and no data copy.
//!
//! Only compiled with the `sqlcipher` feature.

use notion_core::db::{CalendarEvent, DbError, EncryptedDb};
use thiserror::Error;

use crate::event::CompanionEvent;
use crate::keyring::{KeyError, KeyProvider};
use crate::paths;

#[derive(Debug, Error)]
pub enum AccessError {
    #[error("could not resolve the shared data directory (no HOME/XDG_DATA_HOME)")]
    NoDataDir,
    #[error("the vault is locked")]
    Locked,
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error(transparent)]
    Db(#[from] DbError),
}

fn resolve_key(provider: &dyn KeyProvider) -> Result<zeroize::Zeroizing<String>, AccessError> {
    provider.sqlcipher_key_hex()?.ok_or(AccessError::Locked)
}

/// Open the shared DB **read-only** for the watcher daemon.
pub fn open_reader(provider: &dyn KeyProvider) -> Result<EncryptedDb, AccessError> {
    let path = paths::db_path().ok_or(AccessError::NoDataDir)?;
    let key = resolve_key(provider)?;
    Ok(EncryptedDb::open_query_only(&path.to_string_lossy(), &key)?)
}

/// Open the shared DB **read/write** for the GTK quick-view (quick-add / AI).
pub fn open_writer(provider: &dyn KeyProvider) -> Result<EncryptedDb, AccessError> {
    let path = paths::db_path().ok_or(AccessError::NoDataDir)?;
    let key = resolve_key(provider)?;
    Ok(EncryptedDb::open(&path.to_string_lossy(), &key)?)
}

/// Events that have not yet ended as of `now`, as wire-ready [`CompanionEvent`]s.
pub fn upcoming(
    db: &EncryptedDb,
    now: i64,
    limit: i64,
) -> Result<Vec<CompanionEvent>, AccessError> {
    Ok(db
        .upcoming_events(now, limit)?
        .into_iter()
        .map(CompanionEvent::from)
        .collect())
}

/// Events overlapping `[start, end)` as wire-ready [`CompanionEvent`]s.
pub fn in_range(
    db: &EncryptedDb,
    start: i64,
    end: i64,
) -> Result<Vec<CompanionEvent>, AccessError> {
    Ok(db
        .events_in_range(start, end)?
        .into_iter()
        .map(CompanionEvent::from)
        .collect())
}

/// Persist an event (quick-add / AI-add) via a read/write handle.
pub fn write_event(db: &EncryptedDb, event: &CompanionEvent) -> Result<(), AccessError> {
    let row: CalendarEvent = event.clone().into();
    db.upsert_event(&row)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyring::StaticKeyProvider;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    // A writer opened via a StaticKeyProvider persists an event that a
    // subsequent reader over the same file surfaces — exercising the full
    // key-provider → open → upsert → query path against a real encrypted DB.
    #[test]
    fn writer_persists_event_reader_sees_it() {
        let _g = crate::testutil::env_lock();
        // Point the shared-path resolver at a throwaway XDG dir.
        let dir = std::env::temp_dir().join(format!("notion_companion_db_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_DATA_HOME", &dir);
        std::fs::create_dir_all(dir.join(paths::APP_ID)).unwrap();

        let provider = StaticKeyProvider::new(KEY).unwrap();
        {
            let db = open_writer(&provider).unwrap();
            let ev = CompanionEvent {
                id: "e1".into(),
                title: "Dentist".into(),
                start_time: 1_000,
                end_time: 4_600,
                all_day: false,
                location: Some("Clinic".into()),
                description: None,
                block_id: None,
                last_modified: 1_000,
            };
            write_event(&db, &ev).unwrap();
        }
        {
            let db = open_reader(&provider).unwrap();
            assert!(db.is_query_only().unwrap());
            let up = upcoming(&db, 0, 10).unwrap();
            assert_eq!(up.len(), 1);
            assert_eq!(up[0].title, "Dentist");
            let ranged = in_range(&db, 500, 2_000).unwrap();
            assert_eq!(ranged.len(), 1);
        }

        let _ = std::fs::remove_dir_all(&dir);
        std::env::remove_var("XDG_DATA_HOME");
    }

    #[test]
    fn locked_provider_reports_locked() {
        let provider = StaticKeyProvider::locked();
        assert!(matches!(open_reader(&provider), Err(AccessError::Locked)));
    }
}
