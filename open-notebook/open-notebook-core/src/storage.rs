//! The storage seam — the crate's single point of contact with a database.
//!
//! Following the Phase-0 spec, services never open a connection or hold a key;
//! they take a `&dyn NotebookStorage`. There are two implementations:
//!
//! * [`MemStorage`] — a pure in-memory backend, always compiled, used by the
//!   unit tests of every service so the fast CI job needs no SQLite.
//! * [`SqliteStorage`] — (feature `sqlcipher`) opens an *already-unlocked*
//!   SQLCipher connection to the main app's shared `notion.db`, runs the additive
//!   [`crate::schema`] migrations, and writes AI rows straight into the existing
//!   `pages` / `calendar_events` tables so the calendar companion inherits them.

use serde::{Deserialize, Serialize};

use crate::embedding::EmbeddingError;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage backend error: {0}")]
    Backend(String),
    #[error(transparent)]
    Embedding(#[from] EmbeddingError),
}

// ---------------------------------------------------------------------------
// Records (mirror the columns added by `schema::MIGRATION_SQL`)
// ---------------------------------------------------------------------------

/// A stored embedding for one indexed unit of content (a block or a source).
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingRecord {
    pub id: String,
    /// Links back to the originating block/source (e.g. a CRDT block id).
    pub source_block_id: String,
    /// FNV fingerprint of the indexed text; lets a re-index skip unchanged content.
    pub content_hash: String,
    pub embedding: Vec<f32>,
    pub created_at: i64,
}

/// Metadata about an ingested source (PDF / URL / audio / pasted text).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngestedSource {
    pub id: String,
    #[serde(rename = "sourceType")]
    pub source_type: String,
    #[serde(rename = "sourcePath")]
    pub source_path: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    #[serde(rename = "processedAt")]
    pub processed_at: i64,
}

/// A transparency record of an agent action (what the AI did, and to what).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentLog {
    pub id: String,
    #[serde(rename = "agentType")]
    pub agent_type: String,
    pub prompt: String,
    #[serde(rename = "actionTaken")]
    pub action_taken: String,
    #[serde(rename = "blockAffected")]
    pub block_affected: Option<String>,
    pub timestamp: i64,
}

/// The subset of the main app's `calendar_events` columns an agent writes.
/// Timestamps are Unix **seconds** (UTC), matching the companion's contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotebookCalendarEvent {
    pub id: String,
    pub title: String,
    #[serde(rename = "startTime")]
    pub start_time: i64,
    #[serde(rename = "endTime")]
    pub end_time: i64,
    #[serde(rename = "allDay")]
    pub all_day: bool,
    pub location: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "lastModified")]
    pub last_modified: i64,
}

// ---------------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------------

/// The injected storage every Open Notebook service depends on.
///
/// `Send + Sync` so the Tauri app can hold it behind a `Mutex` in shared state.
pub trait NotebookStorage: Send + Sync {
    // -- vector memory --
    fn upsert_embedding(&self, rec: &EmbeddingRecord) -> Result<(), StorageError>;
    fn delete_embeddings_for_source(&self, source_block_id: &str) -> Result<(), StorageError>;
    fn all_embeddings(&self) -> Result<Vec<EmbeddingRecord>, StorageError>;

    // -- full-text (hybrid search + source lookup) --
    fn fts_upsert(&self, id: &str, title: &str, content: &str) -> Result<(), StorageError>;
    fn fts_search(&self, query: &str, limit: i64) -> Result<Vec<String>, StorageError>;

    // -- ingested sources --
    fn insert_source(&self, source: &IngestedSource) -> Result<(), StorageError>;
    fn get_source(&self, id: &str) -> Result<Option<IngestedSource>, StorageError>;
    fn list_sources(&self) -> Result<Vec<IngestedSource>, StorageError>;

    // -- agent transparency log --
    fn log_agent(&self, log: &AgentLog) -> Result<(), StorageError>;
    fn list_agent_logs(&self, limit: i64) -> Result<Vec<AgentLog>, StorageError>;

    // -- writes into the main app's existing tables (Phase 5.2) --
    fn add_calendar_event(&self, ev: &NotebookCalendarEvent) -> Result<(), StorageError>;
    fn create_page(&self, id: &str, title: &str, now_ms: i64) -> Result<(), StorageError>;
}

// ---------------------------------------------------------------------------
// In-memory backend (always available; powers unit tests)
// ---------------------------------------------------------------------------

use std::sync::Mutex;

#[derive(Default)]
struct MemInner {
    embeddings: Vec<EmbeddingRecord>,
    fts: Vec<(String, String, String)>, // (id, title, content)
    sources: Vec<IngestedSource>,
    logs: Vec<AgentLog>,
    events: Vec<NotebookCalendarEvent>,
    pages: Vec<(String, String)>, // (id, title)
}

/// A pure in-memory [`NotebookStorage`] for tests and dry runs.
#[derive(Default)]
pub struct MemStorage {
    inner: Mutex<MemInner>,
}

impl MemStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test/inspection helpers.
    pub fn event_count(&self) -> usize {
        self.inner.lock().unwrap().events.len()
    }
    pub fn page_count(&self) -> usize {
        self.inner.lock().unwrap().pages.len()
    }
    pub fn events(&self) -> Vec<NotebookCalendarEvent> {
        self.inner.lock().unwrap().events.clone()
    }
}

impl NotebookStorage for MemStorage {
    fn upsert_embedding(&self, rec: &EmbeddingRecord) -> Result<(), StorageError> {
        let mut g = self.inner.lock().unwrap();
        if let Some(existing) = g.embeddings.iter_mut().find(|e| e.id == rec.id) {
            *existing = rec.clone();
        } else {
            g.embeddings.push(rec.clone());
        }
        Ok(())
    }

    fn delete_embeddings_for_source(&self, source_block_id: &str) -> Result<(), StorageError> {
        self.inner
            .lock()
            .unwrap()
            .embeddings
            .retain(|e| e.source_block_id != source_block_id);
        Ok(())
    }

    fn all_embeddings(&self) -> Result<Vec<EmbeddingRecord>, StorageError> {
        Ok(self.inner.lock().unwrap().embeddings.clone())
    }

    fn fts_upsert(&self, id: &str, title: &str, content: &str) -> Result<(), StorageError> {
        let mut g = self.inner.lock().unwrap();
        g.fts.retain(|(i, _, _)| i != id);
        g.fts
            .push((id.to_string(), title.to_string(), content.to_string()));
        Ok(())
    }

    fn fts_search(&self, query: &str, limit: i64) -> Result<Vec<String>, StorageError> {
        let terms: Vec<String> = crate::embedding::tokenize(query).collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let g = self.inner.lock().unwrap();
        let mut hits: Vec<(String, usize)> = g
            .fts
            .iter()
            .filter_map(|(id, title, content)| {
                let hay = format!("{title} {content}").to_lowercase();
                let score = terms.iter().filter(|t| hay.contains(t.as_str())).count();
                (score > 0).then(|| (id.clone(), score))
            })
            .collect();
        hits.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        hits.truncate(limit.max(0) as usize);
        Ok(hits.into_iter().map(|(id, _)| id).collect())
    }

    fn insert_source(&self, source: &IngestedSource) -> Result<(), StorageError> {
        let mut g = self.inner.lock().unwrap();
        g.sources.retain(|s| s.id != source.id);
        g.sources.push(source.clone());
        Ok(())
    }

    fn get_source(&self, id: &str) -> Result<Option<IngestedSource>, StorageError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .sources
            .iter()
            .find(|s| s.id == id)
            .cloned())
    }

    fn list_sources(&self) -> Result<Vec<IngestedSource>, StorageError> {
        let mut v = self.inner.lock().unwrap().sources.clone();
        v.sort_by(|a, b| b.processed_at.cmp(&a.processed_at));
        Ok(v)
    }

    fn log_agent(&self, log: &AgentLog) -> Result<(), StorageError> {
        self.inner.lock().unwrap().logs.push(log.clone());
        Ok(())
    }

    fn list_agent_logs(&self, limit: i64) -> Result<Vec<AgentLog>, StorageError> {
        let mut v = self.inner.lock().unwrap().logs.clone();
        v.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        v.truncate(limit.max(0) as usize);
        Ok(v)
    }

    fn add_calendar_event(&self, ev: &NotebookCalendarEvent) -> Result<(), StorageError> {
        let mut g = self.inner.lock().unwrap();
        g.events.retain(|e| e.id != ev.id);
        g.events.push(ev.clone());
        Ok(())
    }

    fn create_page(&self, id: &str, title: &str, _now_ms: i64) -> Result<(), StorageError> {
        self.inner
            .lock()
            .unwrap()
            .pages
            .push((id.to_string(), title.to_string()));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SQLCipher backend (feature `sqlcipher`)
// ---------------------------------------------------------------------------

#[cfg(feature = "sqlcipher")]
mod sqlite {
    use super::*;
    use crate::embedding::{blob_to_vec, vec_to_blob};
    use rusqlite::{params, Connection, OptionalExtension};

    impl From<rusqlite::Error> for StorageError {
        fn from(e: rusqlite::Error) -> Self {
            StorageError::Backend(e.to_string())
        }
    }

    /// A read/write [`NotebookStorage`] over the shared encrypted database.
    ///
    /// It opens its **own** connection to the same file the main app uses (the
    /// app keeps the DB in WAL mode, so an extra writer is fine) and takes the
    /// raw SQLCipher key the host already derived — it never runs Argon2id. On
    /// open it applies the additive Open Notebook migrations.
    pub struct SqliteStorage {
        conn: Mutex<Connection>,
    }

    impl SqliteStorage {
        /// Open the shared DB at `path` with the 64-hex raw key and migrate.
        pub fn open(path: &str, raw_key_hex: &str) -> Result<Self, StorageError> {
            validate_key(raw_key_hex)?;
            let conn = Connection::open(path)?;
            Self::configure(conn, raw_key_hex)
        }

        /// Open an in-memory encrypted DB (integration tests). The caller must
        /// first create the base `pages` / `calendar_events` tables the agent
        /// writes into — [`SqliteStorage::create_base_tables_for_test`] does that.
        #[cfg(test)]
        pub fn open_in_memory(raw_key_hex: &str) -> Result<Self, StorageError> {
            validate_key(raw_key_hex)?;
            let conn = Connection::open_in_memory()?;
            Self::configure(conn, raw_key_hex)
        }

        fn configure(conn: Connection, raw_key_hex: &str) -> Result<Self, StorageError> {
            conn.execute_batch(&format!("PRAGMA key = \"x'{raw_key_hex}'\";"))?;
            conn.execute_batch(
                "PRAGMA temp_store = MEMORY;
                 PRAGMA secure_delete = ON;
                 PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = WAL;
                 -- This is a SECOND writer on a DB the main app also writes. WAL
                 -- allows one writer at a time, so wait for the lock (up to 5s)
                 -- instead of erroring with SQLITE_BUSY under contention (e.g. an
                 -- editor save landing while an AI reindex is mid-INSERT).
                 PRAGMA busy_timeout = 5000;",
            )?;
            // Verify the key (wrong key ⇒ \"file is not a database\").
            let _: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
            conn.execute_batch(crate::schema::MIGRATION_SQL)?;
            Ok(SqliteStorage {
                conn: Mutex::new(conn),
            })
        }

        /// Test helper: read a single string column (or `None` if no row).
        #[cfg(test)]
        pub fn scalar_string(&self, sql: &str) -> Option<String> {
            self.conn
                .lock()
                .unwrap()
                .query_row(sql, [], |r| r.get::<_, String>(0))
                .optional()
                .unwrap()
        }

        /// Create the minimal main-app tables an agent writes into, so an
        /// integration test can run without linking `notion_core`. At runtime
        /// the real app has already created the full versions of these.
        #[cfg(test)]
        pub fn create_base_tables_for_test(&self) -> Result<(), StorageError> {
            self.conn.lock().unwrap().execute_batch(
                "CREATE TABLE IF NOT EXISTS pages (
                     id TEXT PRIMARY KEY, parent_id TEXT, title TEXT NOT NULL DEFAULT '',
                     created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, trashed_at INTEGER);
                 CREATE TABLE IF NOT EXISTS calendar_events (
                     id TEXT PRIMARY KEY, title TEXT NOT NULL, start_time INTEGER NOT NULL,
                     end_time INTEGER NOT NULL, all_day INTEGER NOT NULL DEFAULT 0,
                     location TEXT, description TEXT, block_id TEXT, last_modified INTEGER NOT NULL);",
            )?;
            Ok(())
        }
    }

    fn validate_key(key: &str) -> Result<(), StorageError> {
        if key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(())
        } else {
            Err(StorageError::Backend(
                "key must be 64 lowercase hex characters".into(),
            ))
        }
    }

    impl NotebookStorage for SqliteStorage {
        fn upsert_embedding(&self, rec: &EmbeddingRecord) -> Result<(), StorageError> {
            self.conn.lock().unwrap().execute(
                "INSERT INTO embeddings(id, source_block_id, content_hash, embedding, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                     source_block_id = excluded.source_block_id,
                     content_hash    = excluded.content_hash,
                     embedding       = excluded.embedding,
                     created_at      = excluded.created_at",
                params![
                    rec.id,
                    rec.source_block_id,
                    rec.content_hash,
                    vec_to_blob(&rec.embedding),
                    rec.created_at
                ],
            )?;
            Ok(())
        }

        fn delete_embeddings_for_source(&self, source_block_id: &str) -> Result<(), StorageError> {
            self.conn.lock().unwrap().execute(
                "DELETE FROM embeddings WHERE source_block_id = ?1",
                params![source_block_id],
            )?;
            Ok(())
        }

        fn all_embeddings(&self) -> Result<Vec<EmbeddingRecord>, StorageError> {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, source_block_id, content_hash, embedding, created_at FROM embeddings",
            )?;
            let rows = stmt.query_map([], |row| {
                let blob: Vec<u8> = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    blob,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for r in rows {
                let (id, source_block_id, content_hash, blob, created_at) = r?;
                out.push(EmbeddingRecord {
                    id,
                    source_block_id,
                    content_hash,
                    embedding: blob_to_vec(&blob)?,
                    created_at,
                });
            }
            Ok(out)
        }

        fn fts_upsert(&self, id: &str, title: &str, content: &str) -> Result<(), StorageError> {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM notebook_fts WHERE ref_id = ?1", params![id])?;
            conn.execute(
                "INSERT INTO notebook_fts(ref_id, title, content) VALUES (?1, ?2, ?3)",
                params![id, title, content],
            )?;
            Ok(())
        }

        fn fts_search(&self, query: &str, limit: i64) -> Result<Vec<String>, StorageError> {
            let Some(q) = fts_prefix_query(query) else {
                return Ok(Vec::new());
            };
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT ref_id FROM notebook_fts WHERE notebook_fts MATCH ?1 ORDER BY rank LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![q, limit], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        }

        fn insert_source(&self, s: &IngestedSource) -> Result<(), StorageError> {
            self.conn.lock().unwrap().execute(
                "INSERT INTO ingested_sources(id, source_type, source_path, title, summary, processed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                     source_type = excluded.source_type,
                     source_path = excluded.source_path,
                     title       = excluded.title,
                     summary     = excluded.summary,
                     processed_at= excluded.processed_at",
                params![s.id, s.source_type, s.source_path, s.title, s.summary, s.processed_at],
            )?;
            Ok(())
        }

        fn get_source(&self, id: &str) -> Result<Option<IngestedSource>, StorageError> {
            let conn = self.conn.lock().unwrap();
            Ok(conn
                .query_row(
                    "SELECT id, source_type, source_path, title, summary, processed_at
                       FROM ingested_sources WHERE id = ?1",
                    params![id],
                    row_to_source,
                )
                .optional()?)
        }

        fn list_sources(&self) -> Result<Vec<IngestedSource>, StorageError> {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, source_type, source_path, title, summary, processed_at
                   FROM ingested_sources ORDER BY processed_at DESC, id",
            )?;
            let rows = stmt.query_map([], row_to_source)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        }

        fn log_agent(&self, l: &AgentLog) -> Result<(), StorageError> {
            self.conn.lock().unwrap().execute(
                "INSERT INTO agent_logs(id, agent_type, prompt, action_taken, block_affected, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![l.id, l.agent_type, l.prompt, l.action_taken, l.block_affected, l.timestamp],
            )?;
            Ok(())
        }

        fn list_agent_logs(&self, limit: i64) -> Result<Vec<AgentLog>, StorageError> {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, agent_type, prompt, action_taken, block_affected, timestamp
                   FROM agent_logs ORDER BY timestamp DESC, id DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], |row| {
                Ok(AgentLog {
                    id: row.get(0)?,
                    agent_type: row.get(1)?,
                    prompt: row.get(2)?,
                    action_taken: row.get(3)?,
                    block_affected: row.get(4)?,
                    timestamp: row.get(5)?,
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        }

        fn add_calendar_event(&self, ev: &NotebookCalendarEvent) -> Result<(), StorageError> {
            // Writes into the main app's OWN `calendar_events` table (Phase 5.2):
            // the GNOME companion reads the same rows with no code change.
            self.conn.lock().unwrap().execute(
                "INSERT INTO calendar_events
                     (id, title, start_time, end_time, all_day, location, description, block_id, last_modified)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                     title = excluded.title, start_time = excluded.start_time,
                     end_time = excluded.end_time, all_day = excluded.all_day,
                     location = excluded.location, description = excluded.description,
                     last_modified = excluded.last_modified",
                params![
                    ev.id, ev.title, ev.start_time, ev.end_time, ev.all_day as i64,
                    ev.location, ev.description, ev.last_modified
                ],
            )?;
            Ok(())
        }

        fn create_page(&self, id: &str, title: &str, now_ms: i64) -> Result<(), StorageError> {
            self.conn.lock().unwrap().execute(
                "INSERT INTO pages(id, parent_id, title, created_at, updated_at)
                 VALUES (?1, NULL, ?2, ?3, ?3)
                 ON CONFLICT(id) DO NOTHING",
                params![id, title, now_ms],
            )?;
            Ok(())
        }
    }

    fn row_to_source(row: &rusqlite::Row<'_>) -> rusqlite::Result<IngestedSource> {
        Ok(IngestedSource {
            id: row.get(0)?,
            source_type: row.get(1)?,
            source_path: row.get(2)?,
            title: row.get(3)?,
            summary: row.get(4)?,
            processed_at: row.get(5)?,
        })
    }

    /// Turn arbitrary user text into a safe FTS5 prefix query, or `None` if empty.
    /// Mirrors the main app's `fts_prefix_query` so raw input can never be an
    /// FTS5 syntax error.
    fn fts_prefix_query(raw: &str) -> Option<String> {
        let tokens: Vec<String> = crate::embedding::tokenize(raw)
            .map(|t| format!("\"{t}\"*"))
            .collect();
        (!tokens.is_empty()).then(|| tokens.join(" "))
    }
}

#[cfg(feature = "sqlcipher")]
pub use sqlite::SqliteStorage;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_embeddings_upsert_and_delete() {
        let s = MemStorage::new();
        let rec = EmbeddingRecord {
            id: "a".into(),
            source_block_id: "b1".into(),
            content_hash: "h".into(),
            embedding: vec![1.0, 0.0],
            created_at: 1,
        };
        s.upsert_embedding(&rec).unwrap();
        // Upsert replaces rather than duplicates.
        s.upsert_embedding(&EmbeddingRecord {
            content_hash: "h2".into(),
            ..rec.clone()
        })
        .unwrap();
        let all = s.all_embeddings().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].content_hash, "h2");

        s.delete_embeddings_for_source("b1").unwrap();
        assert!(s.all_embeddings().unwrap().is_empty());
    }

    #[test]
    fn mem_fts_ranks_by_term_hits() {
        let s = MemStorage::new();
        s.fts_upsert("x", "Invoice", "please pay the invoice payment")
            .unwrap();
        s.fts_upsert("y", "Trip", "hiking in the mountains")
            .unwrap();
        let hits = s.fts_search("invoice payment", 10).unwrap();
        assert_eq!(hits, vec!["x".to_string()]);
        assert!(s.fts_search("%%%", 10).unwrap().is_empty());
    }

    #[test]
    fn mem_sources_and_logs_order_newest_first() {
        let s = MemStorage::new();
        for (i, id) in ["old", "new"].iter().enumerate() {
            s.insert_source(&IngestedSource {
                id: (*id).into(),
                source_type: "text".into(),
                source_path: None,
                title: (*id).into(),
                summary: None,
                processed_at: i as i64 * 100,
            })
            .unwrap();
        }
        assert_eq!(s.list_sources().unwrap()[0].id, "new");
        assert_eq!(s.get_source("old").unwrap().unwrap().title, "old");
    }
}

// Integration tests for the real encrypted SQLite backend. Compiled only in the
// full `cargo test` job (feature `sqlcipher`), never in the fast crypto-only run.
#[cfg(all(test, feature = "sqlcipher"))]
mod sqlite_tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn open() -> SqliteStorage {
        let s = SqliteStorage::open_in_memory(KEY).unwrap();
        s.create_base_tables_for_test().unwrap();
        s
    }

    #[test]
    fn rejects_bad_key() {
        assert!(SqliteStorage::open_in_memory("short").is_err());
    }

    #[test]
    fn embeddings_round_trip_through_blob() {
        let s = open();
        let rec = EmbeddingRecord {
            id: "e1".into(),
            source_block_id: "b1".into(),
            content_hash: "h".into(),
            embedding: vec![0.25, -0.5, 1.0],
            created_at: 7,
        };
        s.upsert_embedding(&rec).unwrap();
        let all = s.all_embeddings().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].embedding, vec![0.25, -0.5, 1.0]);

        // Upsert on the same id replaces; delete-by-source clears.
        s.upsert_embedding(&EmbeddingRecord {
            embedding: vec![9.0],
            ..rec.clone()
        })
        .unwrap();
        assert_eq!(s.all_embeddings().unwrap().len(), 1);
        s.delete_embeddings_for_source("b1").unwrap();
        assert!(s.all_embeddings().unwrap().is_empty());
    }

    #[test]
    fn fts5_search_handles_raw_input_safely() {
        let s = open();
        s.fts_upsert("x", "Invoice", "please pay the acme invoice")
            .unwrap();
        s.fts_upsert("y", "Trip", "mountain hiking notes").unwrap();
        assert_eq!(s.fts_search("invoice", 10).unwrap(), vec!["x".to_string()]);
        // Raw input with FTS5 operators must not error (prefix-quoting).
        assert!(s.fts_search("invoice AND \"", 10).unwrap().len() <= 1);
        assert!(s.fts_search("", 10).unwrap().is_empty());
    }

    #[test]
    fn sources_and_agent_logs_persist_and_order() {
        let s = open();
        s.insert_source(&IngestedSource {
            id: "s1".into(),
            source_type: "pdf".into(),
            source_path: Some("/tmp/a.pdf".into()),
            title: "A".into(),
            summary: Some("sum".into()),
            processed_at: 100,
        })
        .unwrap();
        assert_eq!(
            s.get_source("s1").unwrap().unwrap().summary.as_deref(),
            Some("sum")
        );

        for (i, id) in ["l1", "l2"].iter().enumerate() {
            s.log_agent(&AgentLog {
                id: (*id).into(),
                agent_type: "add_event".into(),
                prompt: "p".into(),
                action_taken: "did".into(),
                block_affected: None,
                timestamp: i as i64,
            })
            .unwrap();
        }
        let logs = s.list_agent_logs(10).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].id, "l2"); // newest first
    }

    #[test]
    fn agent_writes_land_in_the_main_app_tables() {
        // Proves Phase 5.2: an AI-added event lands in the SAME `calendar_events`
        // table the GNOME companion reads — verified here by querying it directly.
        let s = open();
        s.add_calendar_event(&NotebookCalendarEvent {
            id: "ev1".into(),
            title: "AI Event".into(),
            start_time: 1000,
            end_time: 4600,
            all_day: false,
            location: Some("Home".into()),
            description: None,
            last_modified: 1000,
        })
        .unwrap();
        s.create_page("pg1", "AI Page", 1000).unwrap();

        assert_eq!(
            s.scalar_string("SELECT title FROM calendar_events WHERE id='ev1'")
                .as_deref(),
            Some("AI Event")
        );
        assert_eq!(
            s.scalar_string("SELECT title FROM pages WHERE id='pg1'")
                .as_deref(),
            Some("AI Page")
        );
    }

    #[test]
    fn migration_created_all_open_notebook_tables() {
        let s = open();
        for table in crate::schema::OPEN_NOTEBOOK_TABLES {
            assert_eq!(
                s.scalar_string(&format!(
                    "SELECT name FROM sqlite_master WHERE name = '{table}'"
                ))
                .as_deref(),
                Some(*table),
                "table `{table}` should exist after migration"
            );
        }
    }
}
