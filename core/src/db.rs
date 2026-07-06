//! Encrypted local database — audit §1.1, §1.8, §2.6.
//!
//! The blueprint assumed `tauri-plugin-sql` encrypts via SQLCipher out of the
//! box; it does **not** (§1.1). We link SQLCipher directly through `rusqlite`'s
//! `bundled-sqlcipher-vendored-openssl` feature, so the encrypted-at-rest path
//! is real and self-contained.
//!
//! Corrections applied on open:
//!   * §2.6 — the key comes from the HKDF `sqlcipher` subkey and is passed as a
//!     **raw** key (`PRAGMA key = "x'…'"`), so SQLCipher does not run a second
//!     PBKDF2 over an already-KDF'd key.
//!   * §1.8 — `PRAGMA temp_store = MEMORY` so FTS5 rebuilds and large sorts
//!     cannot spill **plaintext** to temp files. `secure_delete` is also on.
//!
//! FTS5 (§1.8): the search index is an ordinary (contentless-external-free)
//! FTS5 table living *inside* the encrypted database, so its shadow tables are
//! covered by whole-file page encryption. We never point an external-content
//! table at an unencrypted store.
//!
//! This layer stores **opaque BLOBs**. Callers pass AEAD-sealed update bytes
//! (nonce‖ciphertext from [`crate::crypto::SealedBox`]); the DB does not itself
//! interpret them. At rest they get SQLCipher's page encryption *and* the
//! caller's AEAD (the same sealed bytes are what sync to the relay).

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

use crate::crdt::UpdateEncoding;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("key must be 64 lowercase hex characters (a 256-bit raw key)")]
    BadKey,
    #[error("unknown update encoding tag: {0}")]
    UnknownEncoding(u8),
}

/// Metadata for a page (the container whose body is a CRDT document).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageMeta {
    pub id: String,
    pub title: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// A stored, encoding-tagged CRDT update read back from the log.
#[derive(Debug, Clone)]
pub struct StoredUpdate {
    pub seq: i64,
    pub encoding: UpdateEncoding,
    /// Opaque sealed bytes as provided by the caller.
    pub sealed: Vec<u8>,
    pub created_at_ms: i64,
}

/// A stored version-history restore point.
#[derive(Debug, Clone)]
pub struct StoredSnapshot {
    pub id: i64,
    pub doc_id: String,
    pub label: Option<String>,
    pub sealed: Vec<u8>,
    pub created_at_ms: i64,
}

/// A flattened calendar event (audit-independent companion projection).
///
/// Timestamps are **Unix seconds (UTC)**. This is the shared contract between
/// the main editor (which writes events derived from Database/Calendar blocks),
/// the companion GTK quick-view (quick add / AI add), and the read-only DBus
/// watcher daemon that surfaces them in the GNOME top bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub start_time: i64,
    pub end_time: i64,
    pub all_day: bool,
    pub location: Option<String>,
    pub description: Option<String>,
    /// Optional link back to the originating CRDT block inside a page body.
    pub block_id: Option<String>,
    pub last_modified: i64,
}

/// An open, encrypted SQLCipher database.
pub struct EncryptedDb {
    conn: Connection,
}

impl EncryptedDb {
    /// Open (or create) an encrypted database at `path` using the raw hex key
    /// derived from the `sqlcipher` HKDF subkey ([`crate::crypto::SubKey::to_hex`]).
    pub fn open(path: &str, raw_key_hex: &str) -> Result<Self, DbError> {
        validate_raw_key(raw_key_hex)?;
        let conn = Connection::open(path)?;
        Self::configure(conn, raw_key_hex)
    }

    /// Open a purely in-memory encrypted database (used by tests).
    pub fn open_in_memory(raw_key_hex: &str) -> Result<Self, DbError> {
        validate_raw_key(raw_key_hex)?;
        let conn = Connection::open_in_memory()?;
        Self::configure(conn, raw_key_hex)
    }

    /// Open an existing encrypted database as a **read-only** reader (the
    /// companion DBus watcher daemon).
    ///
    /// We deliberately open a normal read/write handle and then set
    /// `PRAGMA query_only = TRUE`: the main app keeps the database in WAL mode,
    /// and a hard `SQLITE_OPEN_READONLY` handle cannot create the `-shm` file a
    /// WAL reader needs, so it would fail to read a live database. `query_only`
    /// gives us the blueprint's write-protection guarantee (§companion) while
    /// still reading WAL correctly. We do **not** migrate: the main app owns the
    /// schema, and a query-only connection cannot run `CREATE TABLE` anyway.
    pub fn open_query_only(path: &str, raw_key_hex: &str) -> Result<Self, DbError> {
        validate_raw_key(raw_key_hex)?;
        let conn = Connection::open(path)?;
        conn.execute_batch(&format!("PRAGMA key = \"x'{raw_key_hex}'\";"))?;
        // Verify the key before we promise a working reader (wrong key ⇒ error).
        let _: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
        conn.execute_batch(
            "PRAGMA temp_store = MEMORY;
             PRAGMA query_only = TRUE;",
        )?;
        Ok(EncryptedDb { conn })
    }

    /// Whether this connection is read-only (`PRAGMA query_only`).
    pub fn is_query_only(&self) -> Result<bool, DbError> {
        Ok(self
            .conn
            .query_row("PRAGMA query_only", [], |r| r.get::<_, i64>(0))?
            != 0)
    }

    fn configure(conn: Connection, raw_key_hex: &str) -> Result<Self, DbError> {
        // The key PRAGMA must be the very first statement on the connection.
        // raw_key_hex is validated to be 64 hex chars, so this format string is
        // not an injection vector.
        conn.execute_batch(&format!("PRAGMA key = \"x'{raw_key_hex}'\";"))?;

        // §1.8: keep transient data off disk in plaintext.
        conn.execute_batch(
            "PRAGMA temp_store = MEMORY;
             PRAGMA secure_delete = ON;
             PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;",
        )?;

        // Verify the key is correct: with a wrong key this read fails with
        // \"file is not a database\".
        let _: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;

        let db = EncryptedDb { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<(), DbError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS pages (
                 id         TEXT PRIMARY KEY,
                 parent_id  TEXT REFERENCES pages(id) ON DELETE CASCADE,
                 title      TEXT NOT NULL DEFAULT '',
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 trashed_at INTEGER
             );
             -- Append-only encrypted CRDT update log (§1.6): flushed async.
             -- doc_id references its page so (a) deleting a page cascades to its
             -- updates and (b) a late async flush that lands after the page is
             -- gone fails the INSERT instead of orphaning encrypted rows.
             CREATE TABLE IF NOT EXISTS sync_updates (
                 seq        INTEGER PRIMARY KEY AUTOINCREMENT,
                 doc_id     TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
                 encoding   INTEGER NOT NULL,
                 sealed     BLOB NOT NULL,
                 created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_sync_updates_doc
                 ON sync_updates(doc_id, seq);
             -- Full-document restore points for version history (§1.3).
             CREATE TABLE IF NOT EXISTS doc_snapshots (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 doc_id     TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
                 label      TEXT,
                 sealed     BLOB NOT NULL,
                 created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_snapshots_doc
                 ON doc_snapshots(doc_id, created_at);
             -- FTS5 index lives inside the encrypted DB (§1.8).
             CREATE VIRTUAL TABLE IF NOT EXISTS page_search USING fts5(
                 page_id UNINDEXED,
                 title,
                 body,
                 tokenize = 'unicode61 remove_diacritics 2'
             );
             -- Flattened calendar events shared with the GNOME companion.
             -- Timestamps are Unix *seconds* (UTC). `block_id` links back to the
             -- originating CRDT block inside a page body (opaque; blocks are not
             -- rows, so there is no foreign key). This table is a projection the
             -- companion daemon can read cheaply without touching the CRDT log.
             CREATE TABLE IF NOT EXISTS calendar_events (
                 id            TEXT PRIMARY KEY,
                 title         TEXT NOT NULL,
                 start_time    INTEGER NOT NULL,
                 end_time      INTEGER NOT NULL,
                 all_day       INTEGER NOT NULL DEFAULT 0,
                 location      TEXT,
                 description   TEXT,
                 block_id      TEXT,
                 last_modified INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_calendar_start
                 ON calendar_events(start_time);",
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_meta(key, value) VALUES ('schema_version', '1')",
            [],
        )?;
        Ok(())
    }

    /// Confirm `temp_store` is MEMORY (§1.8). Returns the raw pragma value
    /// (2 = MEMORY). Used by tests to prove no plaintext temp spill.
    pub fn temp_store(&self) -> Result<i64, DbError> {
        Ok(self.conn.query_row("PRAGMA temp_store", [], |r| r.get(0))?)
    }

    /// Create a page row. The page id doubles as its CRDT document id.
    pub fn create_page(&self, id: &str, title: &str, now_ms: i64) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO pages(id, parent_id, title, created_at, updated_at)
             VALUES (?1, NULL, ?2, ?3, ?3)",
            params![id, title, now_ms],
        )?;
        Ok(())
    }

    /// List non-trashed pages, most-recently-updated first.
    pub fn list_pages(&self) -> Result<Vec<PageMeta>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, created_at, updated_at
               FROM pages WHERE trashed_at IS NULL
               ORDER BY updated_at DESC, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PageMeta {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at_ms: row.get(2)?,
                updated_at_ms: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Rename a page and bump its `updated_at`.
    pub fn rename_page(&self, id: &str, title: &str, now_ms: i64) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE pages SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, title, now_ms],
        )?;
        Ok(())
    }

    /// Bump a page's `updated_at` (called when its body changes).
    pub fn touch_page(&self, id: &str, now_ms: i64) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE pages SET updated_at = ?2 WHERE id = ?1",
            params![id, now_ms],
        )?;
        Ok(())
    }

    /// Permanently delete a page and everything belonging to it: its CRDT
    /// update log, its snapshots, and its search-index row. Runs in one
    /// transaction so a page never half-exists.
    pub fn delete_page(&self, id: &str) -> Result<(), DbError> {
        self.conn.execute_batch("BEGIN;")?;
        let result = (|| -> Result<(), DbError> {
            self.conn
                .execute("DELETE FROM sync_updates WHERE doc_id = ?1", params![id])?;
            self.conn
                .execute("DELETE FROM doc_snapshots WHERE doc_id = ?1", params![id])?;
            self.conn
                .execute("DELETE FROM page_search WHERE page_id = ?1", params![id])?;
            self.conn
                .execute("DELETE FROM pages WHERE id = ?1", params![id])?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// Append an opaque, encoding-tagged sealed update to the log (§1.6).
    pub fn append_update(
        &self,
        doc_id: &str,
        encoding: UpdateEncoding,
        sealed: &[u8],
        created_at_ms: i64,
    ) -> Result<i64, DbError> {
        self.conn.execute(
            "INSERT INTO sync_updates(doc_id, encoding, sealed, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![doc_id, encoding.tag() as i64, sealed, created_at_ms],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Load all updates for a document in insertion order.
    pub fn load_updates(&self, doc_id: &str) -> Result<Vec<StoredUpdate>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, encoding, sealed, created_at
               FROM sync_updates WHERE doc_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![doc_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (seq, enc_tag, sealed, created_at_ms) = r?;
            // Validate the full i64 BEFORE narrowing: `enc_tag as u8` alone
            // would truncate e.g. 257 -> 1 and mis-accept it as V1 (§ review #8).
            let encoding = u8::try_from(enc_tag)
                .ok()
                .and_then(UpdateEncoding::from_tag)
                .ok_or(DbError::UnknownEncoding(enc_tag as u8))?;
            out.push(StoredUpdate {
                seq,
                encoding,
                sealed,
                created_at_ms,
            });
        }
        Ok(out)
    }

    /// Save a full-document snapshot (§1.3).
    pub fn save_snapshot(
        &self,
        doc_id: &str,
        label: Option<&str>,
        sealed: &[u8],
        created_at_ms: i64,
    ) -> Result<i64, DbError> {
        self.conn.execute(
            "INSERT INTO doc_snapshots(doc_id, label, sealed, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![doc_id, label, sealed, created_at_ms],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List snapshots for a document, newest first.
    pub fn list_snapshots(&self, doc_id: &str) -> Result<Vec<StoredSnapshot>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, doc_id, label, sealed, created_at
               FROM doc_snapshots WHERE doc_id = ?1 ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map(params![doc_id], |row| {
            Ok(StoredSnapshot {
                id: row.get(0)?,
                doc_id: row.get(1)?,
                label: row.get(2)?,
                sealed: row.get(3)?,
                created_at_ms: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The most recent snapshot for a document, if any.
    pub fn latest_snapshot(&self, doc_id: &str) -> Result<Option<StoredSnapshot>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, doc_id, label, sealed, created_at
                   FROM doc_snapshots WHERE doc_id = ?1
                   ORDER BY created_at DESC, id DESC LIMIT 1",
                params![doc_id],
                |row| {
                    Ok(StoredSnapshot {
                        id: row.get(0)?,
                        doc_id: row.get(1)?,
                        label: row.get(2)?,
                        sealed: row.get(3)?,
                        created_at_ms: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Index (or re-index) a page's searchable text in the encrypted FTS5 table.
    pub fn index_page(&self, page_id: &str, title: &str, body: &str) -> Result<(), DbError> {
        self.conn.execute(
            "DELETE FROM page_search WHERE page_id = ?1",
            params![page_id],
        )?;
        self.conn.execute(
            "INSERT INTO page_search(page_id, title, body) VALUES (?1, ?2, ?3)",
            params![page_id, title, body],
        )?;
        Ok(())
    }

    /// Full-text search over indexed pages; returns matching page ids ranked.
    pub fn search(&self, query: &str) -> Result<Vec<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT page_id FROM page_search WHERE page_search MATCH ?1 ORDER BY rank")?;
        let rows = stmt.query_map(params![query], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // -----------------------------------------------------------------------
    // Calendar events (shared with the GNOME companion)
    // -----------------------------------------------------------------------

    /// Insert or replace a calendar event (upsert on primary key `id`).
    pub fn upsert_event(&self, ev: &CalendarEvent) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO calendar_events
                 (id, title, start_time, end_time, all_day, location, description, block_id, last_modified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 start_time = excluded.start_time,
                 end_time = excluded.end_time,
                 all_day = excluded.all_day,
                 location = excluded.location,
                 description = excluded.description,
                 block_id = excluded.block_id,
                 last_modified = excluded.last_modified",
            params![
                ev.id,
                ev.title,
                ev.start_time,
                ev.end_time,
                ev.all_day as i64,
                ev.location,
                ev.description,
                ev.block_id,
                ev.last_modified,
            ],
        )?;
        Ok(())
    }

    /// Delete a calendar event by id. Returns whether a row was removed.
    pub fn delete_event(&self, id: &str) -> Result<bool, DbError> {
        let n = self
            .conn
            .execute("DELETE FROM calendar_events WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    /// Fetch a single event by id.
    pub fn get_event(&self, id: &str) -> Result<Option<CalendarEvent>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, title, start_time, end_time, all_day, location, description, block_id, last_modified
                   FROM calendar_events WHERE id = ?1",
                params![id],
                row_to_event,
            )
            .optional()?)
    }

    /// Events overlapping the half-open interval `[start, end)`, earliest first.
    /// An event overlaps when it starts before `end` and ends after `start`.
    pub fn events_in_range(&self, start: i64, end: i64) -> Result<Vec<CalendarEvent>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, start_time, end_time, all_day, location, description, block_id, last_modified
               FROM calendar_events
              WHERE start_time < ?2 AND end_time > ?1
              ORDER BY start_time, id",
        )?;
        let rows = stmt.query_map(params![start, end], row_to_event)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The next `limit` events that have not yet ended as of `now` (seconds),
    /// ordered by start time. Powers the top-bar "upcoming" list.
    pub fn upcoming_events(&self, now: i64, limit: i64) -> Result<Vec<CalendarEvent>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, start_time, end_time, all_day, location, description, block_id, last_modified
               FROM calendar_events
              WHERE end_time > ?1
              ORDER BY start_time, id
              LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![now, limit], row_to_event)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// All events, earliest first (used by month/week views).
    pub fn all_events(&self) -> Result<Vec<CalendarEvent>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, start_time, end_time, all_day, location, description, block_id, last_modified
               FROM calendar_events ORDER BY start_time, id",
        )?;
        let rows = stmt.query_map([], row_to_event)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

/// Map a `calendar_events` row (in the canonical column order used above) to a
/// [`CalendarEvent`].
fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<CalendarEvent> {
    Ok(CalendarEvent {
        id: row.get(0)?,
        title: row.get(1)?,
        start_time: row.get(2)?,
        end_time: row.get(3)?,
        all_day: row.get::<_, i64>(4)? != 0,
        location: row.get(5)?,
        description: row.get(6)?,
        block_id: row.get(7)?,
        last_modified: row.get(8)?,
    })
}

/// A raw SQLCipher key must be exactly 64 lowercase hex chars (256-bit).
fn validate_raw_key(key: &str) -> Result<(), DbError> {
    if key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(DbError::BadKey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_path() -> String {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!("notion_db_test_{}_{}.db", std::process::id(), n))
            .to_string_lossy()
            .into_owned()
    }

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const KEY2: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    struct TempDb(String);
    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(format!("{}-wal", self.0));
            let _ = std::fs::remove_file(format!("{}-shm", self.0));
        }
    }

    #[test]
    fn rejects_malformed_key() {
        assert!(matches!(
            EncryptedDb::open_in_memory("short"),
            Err(DbError::BadKey)
        ));
        assert!(matches!(
            EncryptedDb::open_in_memory(&"z".repeat(64)),
            Err(DbError::BadKey)
        ));
    }

    #[test]
    fn temp_store_is_memory() {
        // §1.8: proves transient data won't spill to plaintext temp files.
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        assert_eq!(db.temp_store().unwrap(), 2); // 2 == MEMORY
    }

    #[test]
    fn page_crud_round_trips() {
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        db.create_page("p1", "First", 100).unwrap();
        db.create_page("p2", "Second", 200).unwrap();

        // Newest-updated first.
        let pages = db.list_pages().unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].id, "p2");
        assert_eq!(pages[1].title, "First");

        // Rename bumps updated_at, reordering the list.
        db.rename_page("p1", "First (renamed)", 300).unwrap();
        let pages = db.list_pages().unwrap();
        assert_eq!(pages[0].id, "p1");
        assert_eq!(pages[0].title, "First (renamed)");
        assert_eq!(pages[0].updated_at_ms, 300);
    }

    #[test]
    fn append_for_missing_page_is_rejected() {
        // The FK (with foreign_keys=ON) means a late async flush that lands
        // after its page was deleted fails instead of orphaning encrypted rows.
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        assert!(db
            .append_update("ghost", UpdateEncoding::V1, b"x", 1)
            .is_err());
        assert!(db.save_snapshot("ghost", None, b"x", 1).is_err());
    }

    #[test]
    fn deleting_page_cascades_to_updates_and_snapshots() {
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        db.create_page("p1", "t", 1).unwrap();
        db.append_update("p1", UpdateEncoding::V1, b"u", 1).unwrap();
        db.save_snapshot("p1", None, b"s", 1).unwrap();
        // Delete only the page row; the FK cascade removes its children.
        db.conn
            .execute("DELETE FROM pages WHERE id = 'p1'", [])
            .unwrap();
        assert!(db.load_updates("p1").unwrap().is_empty());
        assert!(db.list_snapshots("p1").unwrap().is_empty());
    }

    #[test]
    fn delete_page_purges_all_page_data() {
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        db.create_page("p1", "Doomed", 1).unwrap();
        db.append_update("p1", UpdateEncoding::V1, b"u", 2).unwrap();
        db.save_snapshot("p1", None, b"s", 3).unwrap();
        db.index_page("p1", "Doomed", "secret body").unwrap();

        db.delete_page("p1").unwrap();

        assert!(db.list_pages().unwrap().is_empty());
        assert!(db.load_updates("p1").unwrap().is_empty());
        assert!(db.list_snapshots("p1").unwrap().is_empty());
        assert!(db.search("secret").unwrap().is_empty());
    }

    #[test]
    fn update_log_round_trips() {
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        db.create_page("doc1", "d1", 1).unwrap();
        db.create_page("doc2", "d2", 1).unwrap();
        db.append_update("doc1", UpdateEncoding::V1, b"sealed-a", 1000)
            .unwrap();
        db.append_update("doc1", UpdateEncoding::V1, b"sealed-b", 1001)
            .unwrap();
        db.append_update("doc2", UpdateEncoding::V1, b"other", 1002)
            .unwrap();

        let updates = db.load_updates("doc1").unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].sealed, b"sealed-a");
        assert_eq!(updates[1].sealed, b"sealed-b");
        assert_eq!(updates[0].encoding, UpdateEncoding::V1);
    }

    #[test]
    fn snapshots_round_trip_newest_first() {
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        db.create_page("doc1", "d1", 1).unwrap();
        db.save_snapshot("doc1", Some("autosave"), b"snap-1", 100)
            .unwrap();
        db.save_snapshot("doc1", None, b"snap-2", 200).unwrap();

        let snaps = db.list_snapshots("doc1").unwrap();
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].sealed, b"snap-2"); // newest first
        assert_eq!(
            db.latest_snapshot("doc1").unwrap().unwrap().sealed,
            b"snap-2"
        );
        assert!(db.latest_snapshot("missing").unwrap().is_none());
    }

    #[test]
    fn fts_search_finds_pages() {
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        db.index_page("p1", "Grocery List", "milk eggs bread")
            .unwrap();
        db.index_page("p2", "Trip Notes", "flight to Tokyo")
            .unwrap();

        assert_eq!(db.search("eggs").unwrap(), vec!["p1".to_string()]);
        assert_eq!(db.search("Tokyo").unwrap(), vec!["p2".to_string()]);
        assert!(db.search("nonexistent").unwrap().is_empty());

        // Re-indexing replaces old content.
        db.index_page("p1", "Grocery List", "milk only").unwrap();
        assert!(db.search("eggs").unwrap().is_empty());
    }

    #[test]
    fn encryption_persists_and_wrong_key_fails() {
        let path = temp_path();
        let _guard = TempDb(path.clone());
        {
            let db = EncryptedDb::open(&path, KEY).unwrap();
            db.create_page("doc1", "d1", 1).unwrap();
            db.append_update("doc1", UpdateEncoding::V1, b"persisted", 1)
                .unwrap();
        }
        // Correct key reopens and reads the data.
        {
            let db = EncryptedDb::open(&path, KEY).unwrap();
            assert_eq!(db.load_updates("doc1").unwrap()[0].sealed, b"persisted");
        }
        // Wrong key cannot open the encrypted file (§1.1).
        assert!(EncryptedDb::open(&path, KEY2).is_err());
    }

    #[test]
    fn raw_file_bytes_are_not_plaintext() {
        // §1.1: the on-disk file must not contain our plaintext markers.
        let path = temp_path();
        let _guard = TempDb(path.clone());
        {
            let db = EncryptedDb::open(&path, KEY).unwrap();
            db.index_page("p1", "SECRETMARKER", "TOPSECRETBODY")
                .unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        // SQLCipher also encrypts the header, so it should not start with the
        // classic "SQLite format 3" magic either.
        assert!(!bytes.starts_with(b"SQLite format 3\0"));
        let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(!contains(b"SECRETMARKER"));
        assert!(!contains(b"TOPSECRETBODY"));
    }

    fn sample_event(id: &str, start: i64, end: i64) -> CalendarEvent {
        CalendarEvent {
            id: id.to_string(),
            title: format!("Event {id}"),
            start_time: start,
            end_time: end,
            all_day: false,
            location: Some("Room 1".into()),
            description: None,
            block_id: None,
            last_modified: start,
        }
    }

    #[test]
    fn calendar_events_upsert_get_and_delete() {
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        let mut ev = sample_event("e1", 1_000, 2_000);
        db.upsert_event(&ev).unwrap();
        assert_eq!(db.get_event("e1").unwrap().unwrap(), ev);

        // Upsert on the same id replaces (does not duplicate).
        ev.title = "Renamed".into();
        ev.location = None;
        ev.last_modified = 3_000;
        db.upsert_event(&ev).unwrap();
        let got = db.get_event("e1").unwrap().unwrap();
        assert_eq!(got.title, "Renamed");
        assert_eq!(got.location, None);
        assert_eq!(db.all_events().unwrap().len(), 1);

        assert!(db.delete_event("e1").unwrap());
        assert!(!db.delete_event("e1").unwrap()); // already gone
        assert!(db.get_event("e1").unwrap().is_none());
    }

    #[test]
    fn calendar_events_range_and_upcoming() {
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        db.upsert_event(&sample_event("a", 100, 200)).unwrap();
        db.upsert_event(&sample_event("b", 150, 250)).unwrap();
        db.upsert_event(&sample_event("c", 400, 500)).unwrap();

        // [180, 420): overlaps a (ends 200 > 180), b (150 < 420), c (starts 400 < 420).
        let ids: Vec<_> = db
            .events_in_range(180, 420)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);

        // A window strictly between events returns nothing.
        assert!(db.events_in_range(260, 399).unwrap().is_empty());

        // Upcoming from now=160 excludes nothing that still runs; ordered by start.
        let up: Vec<_> = db
            .upcoming_events(160, 2)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(up, vec!["a", "b"]); // a still running (ends 200), then b; limit 2

        // now=300 drops a and b (both ended), leaving c.
        let up: Vec<_> = db
            .upcoming_events(300, 5)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(up, vec!["c"]);
    }

    #[test]
    fn query_only_reader_cannot_write() {
        // A reader opened query-only sees committed rows but rejects writes,
        // matching the companion daemon's guarantee.
        let path = temp_path();
        let _guard = TempDb(path.clone());
        {
            let db = EncryptedDb::open(&path, KEY).unwrap();
            db.upsert_event(&sample_event("e1", 10, 20)).unwrap();
        }
        let reader = EncryptedDb::open_query_only(&path, KEY).unwrap();
        assert!(reader.is_query_only().unwrap());
        assert_eq!(reader.all_events().unwrap().len(), 1);
        // Writes are rejected at the SQL layer.
        assert!(reader.upsert_event(&sample_event("e2", 30, 40)).is_err());
        // A wrong key cannot open the reader at all.
        assert!(EncryptedDb::open_query_only(&path, KEY2).is_err());
    }

    #[test]
    fn rejects_out_of_range_encoding_tag() {
        // A corrupt/tampered row with encoding 257 must NOT be mis-decoded as V1
        // (257 as u8 == 1). Validated before narrowing (§ review #8).
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
        db.create_page("d", "d", 1).unwrap();
        db.conn
            .execute(
                "INSERT INTO sync_updates(doc_id, encoding, sealed, created_at)
                 VALUES ('d', 257, x'00', 1)",
                [],
            )
            .unwrap();
        assert!(matches!(
            db.load_updates("d"),
            Err(DbError::UnknownEncoding(_))
        ));
    }
}
