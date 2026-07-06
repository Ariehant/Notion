//! Component A — the Notion calendar DBus watcher daemon.
//!
//! A tiny (~single-digit-MB) background process that is the "glue" of the
//! companion ecosystem. It:
//!
//!   1. Opens the shared SQLCipher database **read-only** (key from the GNOME
//!      keyring, published by the main app on unlock).
//!   2. Watches the database file with `inotify` (kernel-level; no polling).
//!   3. On every change, recomputes the current agenda and broadcasts a
//!      `com.notion.Calendar.EventsUpdated` DBus signal carrying the events as
//!      JSON, so the GNOME Shell extension can refresh the top bar instantly.
//!   4. Answers on-demand `GetTodayEvents` / `GetUpcoming` method calls.
//!
//! It never writes to the database and holds no long-lived plaintext: each
//! refresh opens a fresh read-only connection, so lock/unlock transitions and
//! password changes are handled transparently (a locked vault simply yields an
//! empty agenda until the key reappears).

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use inotify::{Inotify, WatchMask};
use notion_companion::keyring::{EnvKeyProvider, KeyProvider, SecretServiceKeyProvider};
use notion_companion::{dbaccess, event, paths, time as ctime};
use zbus::object_server::SignalEmitter;
use zbus::{connection, interface};

/// Well-known DBus name + object path this daemon owns.
const DBUS_NAME: &str = "com.notion.Calendar";
const DBUS_PATH: &str = "/com/notion/Calendar";

/// How many days the top-bar agenda spans (today + tomorrow).
const AGENDA_DAYS: i64 = 2;
/// Default cap for `GetUpcoming` / signal payloads.
const DEFAULT_LIMIT: i64 = 25;
/// Debounce window: coalesce a burst of inotify events (WAL churn) into one
/// recompute so a single save doesn't emit a storm of signals.
const DEBOUNCE: Duration = Duration::from_millis(150);

/// Resolve the SQLCipher key from the OS keyring, falling back to the
/// `NOTION_SQLCIPHER_KEY_HEX` env var (useful for development / headless runs).
struct KeySource;

impl KeyProvider for KeySource {
    fn sqlcipher_key_hex(
        &self,
    ) -> Result<Option<zeroize::Zeroizing<String>>, notion_companion::keyring::KeyError> {
        match SecretServiceKeyProvider.sqlcipher_key_hex() {
            Ok(Some(k)) => Ok(Some(k)),
            // Keyring empty or unreachable → try the env override before giving up.
            _ => EnvKeyProvider.sqlcipher_key_hex(),
        }
    }
}

/// Current wall-clock second. Kept in one place so the daemon's notion of "now"
/// is consistent across a refresh.
fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Compute the current agenda (today + tomorrow) as JSON. Never fails outward:
/// a locked vault or a not-yet-created DB yields an empty array so the top bar
/// shows "No events" instead of an error.
fn compute_agenda_json() -> String {
    let events = tokio_agenda().unwrap_or_default();
    event::events_to_json(&events)
}

/// Blocking DB work: open read-only and read the agenda window.
fn tokio_agenda() -> Option<Vec<event::CompanionEvent>> {
    let db = dbaccess::open_reader(&KeySource).ok()?;
    let now = now_secs();
    let (start, end) = ctime::multiday_bounds(now, ctime::local_offset_secs(), AGENDA_DAYS);
    dbaccess::in_range(&db, start, end).ok()
}

/// Blocking DB work: the next `limit` not-yet-ended events.
fn read_upcoming(limit: i64) -> Vec<event::CompanionEvent> {
    (|| -> Option<Vec<event::CompanionEvent>> {
        let db = dbaccess::open_reader(&KeySource).ok()?;
        dbaccess::upcoming(&db, now_secs(), limit).ok()
    })()
    .unwrap_or_default()
}

/// The DBus interface object. `com.notion.Calendar`.
struct Calendar;

#[interface(name = "com.notion.Calendar")]
impl Calendar {
    /// Today + tomorrow's events as a JSON array string.
    async fn get_today_events(&self) -> String {
        tokio::task::spawn_blocking(compute_agenda_json)
            .await
            .unwrap_or_else(|_| "[]".to_string())
    }

    /// The next `count` upcoming events as a JSON array string. A non-positive
    /// or oversized `count` is clamped to a sane default.
    async fn get_upcoming(&self, count: i32) -> String {
        let limit = if (1..=200).contains(&count) {
            count as i64
        } else {
            DEFAULT_LIMIT
        };
        let events = tokio::task::spawn_blocking(move || read_upcoming(limit))
            .await
            .unwrap_or_default();
        event::events_to_json(&events)
    }

    /// Broadcast when the underlying database changes. Carries the fresh agenda
    /// as a JSON array string so listeners need no follow-up call.
    #[zbus(signal)]
    async fn events_updated(emitter: &SignalEmitter<'_>, json_data: String) -> zbus::Result<()>;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("notion-watcher: starting; DBus name {DBUS_NAME}");

    // Claim our well-known name on the session bus and serve the interface.
    let connection = connection::Builder::session()?
        .name(DBUS_NAME)?
        .serve_at(DBUS_PATH, Calendar)?
        .build()
        .await?;

    // Resolve the directory to watch. It should exist once the main app has run;
    // if not, we watch as soon as it appears (created below).
    let data_dir = paths::data_dir().ok_or("cannot resolve XDG data directory")?;
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("notion-watcher: could not ensure data dir {data_dir:?}: {e}");
    }
    eprintln!("notion-watcher: watching {data_dir:?}");

    // A shared emitter handle so the inotify task can broadcast signals.
    let iface_ref = connection
        .object_server()
        .interface::<_, Calendar>(DBUS_PATH)
        .await?;
    let emitter = Arc::new(iface_ref.signal_emitter().clone());

    // Emit an initial agenda so listeners that start after us get current data.
    emit_agenda(&emitter).await;

    // inotify: watch the *directory* (not the file) so we survive the atomic
    // rename SQLCipher/WAL checkpoints and vault rewrites do (the inode changes).
    let inotify = Inotify::init()?;
    inotify.watches().add(
        &data_dir,
        WatchMask::MODIFY
            | WatchMask::CREATE
            | WatchMask::MOVED_TO
            | WatchMask::CLOSE_WRITE
            | WatchMask::DELETE,
    )?;
    let mut buffer = [0u8; 4096];
    let mut stream = inotify.into_event_stream(&mut buffer)?;

    // Also refresh on SIGTERM shutdown is unnecessary; just handle Ctrl-C/term.
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    let watch_emitter = Arc::clone(&emitter);
    loop {
        tokio::select! {
            maybe_event = stream.next() => {
                match maybe_event {
                    Some(Ok(_ev)) => {
                        // Coalesce a burst: drain anything that arrives within the
                        // debounce window, then recompute once.
                        drain_for(&mut stream, DEBOUNCE).await;
                        emit_agenda(&watch_emitter).await;
                    }
                    Some(Err(e)) => eprintln!("notion-watcher: inotify error: {e}"),
                    None => break,
                }
            }
            _ = sigterm.recv() => {
                eprintln!("notion-watcher: SIGTERM, shutting down");
                break;
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("notion-watcher: interrupted, shutting down");
                break;
            }
        }
    }
    Ok(())
}

/// Recompute the agenda off the async runtime and broadcast it.
async fn emit_agenda(emitter: &SignalEmitter<'_>) {
    let json = tokio::task::spawn_blocking(compute_agenda_json)
        .await
        .unwrap_or_else(|_| "[]".to_string());
    if let Err(e) = Calendar::events_updated(emitter, json).await {
        eprintln!("notion-watcher: failed to emit EventsUpdated: {e}");
    }
}

/// Swallow further inotify events for `window` so a burst becomes one refresh.
async fn drain_for<S>(stream: &mut S, window: Duration)
where
    S: futures_util::Stream + Unpin,
{
    let deadline = tokio::time::sleep(window);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            next = stream.next() => {
                if next.is_none() { break; }
                // else: keep draining until the window elapses
            }
        }
    }
}
