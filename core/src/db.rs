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
             CREATE TABLE IF NOT EXISTS sync_updates (
                 seq        INTEGER PRIMARY KEY AUTOINCREMENT,
                 doc_id     TEXT NOT NULL,
                 encoding   INTEGER NOT NULL,
                 sealed     BLOB NOT NULL,
                 created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_sync_updates_doc
                 ON sync_updates(doc_id, seq);
             -- Full-document restore points for version history (§1.3).
             CREATE TABLE IF NOT EXISTS doc_snapshots (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 doc_id     TEXT NOT NULL,
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
             );",
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
    fn update_log_round_trips() {
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
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

    #[test]
    fn rejects_out_of_range_encoding_tag() {
        // A corrupt/tampered row with encoding 257 must NOT be mis-decoded as V1
        // (257 as u8 == 1). Validated before narrowing (§ review #8).
        let db = EncryptedDb::open_in_memory(KEY).unwrap();
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
