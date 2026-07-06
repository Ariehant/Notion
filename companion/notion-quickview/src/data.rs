//! Data glue between the GTK UI and the tested `notion-companion` crate.
//!
//! Everything here is a thin wrapper over functions that ARE unit-tested in the
//! shared crate (`dbaccess`, `ai`, `time`), so the untested surface of this app
//! stays confined to the GTK widget code in `main.rs`.

use notion_companion::ai::{self, Interpretation, OllamaClient};
use notion_companion::dbaccess::{self, AccessError};
use notion_companion::event::CompanionEvent;
use notion_companion::keyring::{EnvKeyProvider, KeyError, KeyProvider, SecretServiceKeyProvider};
use notion_companion::time as ctime;

/// Number of days the quick-view lists ahead (a rolling week).
pub const WEEK_DAYS: i64 = 7;
/// Look-ahead window used for AI conflict detection.
const CONFLICT_HORIZON_DAYS: i64 = 30;

/// Resolve the SQLCipher key from the OS keyring, falling back to the
/// `NOTION_SQLCIPHER_KEY_HEX` env var. Identical policy to the daemon so both
/// open the very same encrypted file.
pub struct KeySource;

impl KeyProvider for KeySource {
    fn sqlcipher_key_hex(&self) -> Result<Option<zeroize::Zeroizing<String>>, KeyError> {
        match SecretServiceKeyProvider.sqlcipher_key_hex() {
            Ok(Some(k)) => Ok(Some(k)),
            _ => EnvKeyProvider.sqlcipher_key_hex(),
        }
    }
}

pub fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The rolling-week agenda (from local midnight today, `WEEK_DAYS` ahead).
pub fn load_week() -> Result<Vec<CompanionEvent>, AccessError> {
    let db = dbaccess::open_writer(&KeySource)?;
    let (start, _) = ctime::day_bounds(now_secs(), ctime::local_offset_secs());
    dbaccess::in_range(&db, start, start + WEEK_DAYS * ctime::SECS_PER_DAY)
}

/// Persist an event (quick-add or accepted AI suggestion). The write lands in
/// the shared DB, which the watcher daemon picks up via inotify.
pub fn save_event(ev: &CompanionEvent) -> Result<(), AccessError> {
    let db = dbaccess::open_writer(&KeySource)?;
    dbaccess::write_event(&db, ev)
}

/// Delete an event by id.
pub fn delete_event(id: &str) -> Result<(), AccessError> {
    let db = dbaccess::open_writer(&KeySource)?;
    db.delete_event(id)?;
    Ok(())
}

/// Build a timed [`CompanionEvent`] from raw quick-add form fields.
///
/// `date` is `YYYY-MM-DD`, `time` is `HH:MM`, `duration_min` the length. Returns
/// a fresh-id event on success or a human-readable error for bad input.
pub fn build_quick_event(
    title: &str,
    date: &str,
    time: &str,
    duration_min: i64,
    location: Option<String>,
) -> Result<CompanionEvent, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Please enter a title.".into());
    }
    let offset = ctime::local_offset_secs();
    let start = ctime::parse_naive_local(&format!("{date} {time}"), offset)
        .map_err(|_| "Could not read the date/time (use YYYY-MM-DD and HH:MM).".to_string())?;
    let end = start + duration_min.max(1) * 60;
    Ok(CompanionEvent {
        id: uuid::Uuid::new_v4().to_string(),
        title: title.to_string(),
        start_time: start,
        end_time: end,
        all_day: false,
        location: location.filter(|s| !s.trim().is_empty()),
        description: None,
        block_id: None,
        last_modified: now_secs(),
    })
}

/// Ask the local model to interpret a natural-language request, checking the
/// upcoming month for conflicts. Runs blocking work (DB + HTTP); call it from a
/// worker thread (`gio::spawn_blocking`), never the GTK main thread.
pub fn ask_ai(text: &str) -> Result<Interpretation, String> {
    let db = dbaccess::open_writer(&KeySource).map_err(|e| e.to_string())?;
    let now = now_secs();
    let offset = ctime::local_offset_secs();
    let (start, _) = ctime::day_bounds(now, offset);
    let existing = dbaccess::in_range(
        &db,
        start,
        start + CONFLICT_HORIZON_DAYS * ctime::SECS_PER_DAY,
    )
    .map_err(|e| e.to_string())?;
    let client = OllamaClient::default();
    let id = uuid::Uuid::new_v4().to_string();
    ai::interpret(&client, text, now, offset, id, None, &existing).map_err(|e| e.to_string())
}
