//! Additive schema migrations for the Open Notebook tables (Phase 1).
//!
//! The golden rule of the merge: **add, never alter.** We only `CREATE TABLE IF
//! NOT EXISTS` new tables. We do not touch `pages`, `sync_updates`,
//! `doc_snapshots`, `page_search`, or `calendar_events` — the columns the main
//! editor and the GNOME calendar companion depend on are untouched, so the
//! companion stays 100% compatible with zero code changes (Phase 5).
//!
//! The FTS index is named `notebook_fts` (not the spec's generic `block_fts`) to
//! avoid colliding with the main app's existing `page_search` FTS5 table; both
//! live *inside* the SQLCipher file so their shadow tables are page-encrypted.

/// The migration batch. Idempotent: safe to run on every startup / every open.
pub const MIGRATION_SQL: &str = "
-- 1. Vector storage for semantic search. `embedding` is little-endian f32.
CREATE TABLE IF NOT EXISTS embeddings (
    id              TEXT PRIMARY KEY,
    source_block_id TEXT NOT NULL,   -- links back to a block / ingested source
    content_hash    TEXT NOT NULL,   -- change detector to skip re-embedding
    embedding       BLOB NOT NULL,   -- f32[] serialized little-endian
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_embeddings_source
    ON embeddings(source_block_id);

-- 2. Ingestion metadata (PDF / URL / audio / pasted text).
CREATE TABLE IF NOT EXISTS ingested_sources (
    id           TEXT PRIMARY KEY,
    source_type  TEXT NOT NULL,      -- 'pdf' | 'url' | 'audio' | 'text'
    source_path  TEXT,               -- path or URL; NULL for inline text
    title        TEXT NOT NULL,
    summary      TEXT,
    processed_at INTEGER NOT NULL
);

-- 3. Agent execution log (transparency: what the AI did, and to what).
CREATE TABLE IF NOT EXISTS agent_logs (
    id             TEXT PRIMARY KEY,
    agent_type     TEXT NOT NULL,
    prompt         TEXT NOT NULL,
    action_taken   TEXT NOT NULL,
    block_affected TEXT,
    timestamp      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_logs_time
    ON agent_logs(timestamp);

-- 4. Full-text index for hybrid (vector + keyword) search over notebook
--    content. Distinct from the app's `page_search` to avoid a name clash.
CREATE VIRTUAL TABLE IF NOT EXISTS notebook_fts USING fts5(
    ref_id UNINDEXED,
    title,
    content,
    tokenize = 'porter unicode61 remove_diacritics 2'
);
";

/// The table names this migration introduces, for tests / diagnostics.
pub const OPEN_NOTEBOOK_TABLES: &[&str] = &[
    "embeddings",
    "ingested_sources",
    "agent_logs",
    "notebook_fts",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_only_creates_if_not_exists() {
        // Guard against a future edit sneaking in a destructive statement.
        let upper = MIGRATION_SQL.to_uppercase();
        assert!(!upper.contains("DROP "));
        assert!(!upper.contains("ALTER "));
        // Every CREATE is guarded so re-running never errors.
        let creates = upper.matches("CREATE ").count();
        let if_not_exists = upper.matches("IF NOT EXISTS").count();
        assert_eq!(
            creates, if_not_exists,
            "every CREATE must be IF NOT EXISTS (idempotent migration)"
        );
    }

    #[test]
    fn does_not_touch_main_app_tables() {
        // Strip `--` comments first: the explanatory prose legitimately names
        // `page_search`; what must never appear is an actual *statement* against
        // a main-app table.
        let sql: String = MIGRATION_SQL
            .lines()
            .map(|l| l.split("--").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        for owned in ["pages", "sync_updates", "doc_snapshots", "page_search"] {
            assert!(
                !sql.contains(owned),
                "migration must not reference the main app's `{owned}` table in a statement"
            );
        }
    }
}
