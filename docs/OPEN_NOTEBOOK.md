# Open Notebook merge — memory, ingestion, studio, agents, CLI & MCP

This document describes how the "Open Notebook" AI engine was restructured and
merged into the Notion desktop backend, following the technical specification's
phased plan. The golden rule throughout: **add capability without breaking the
existing editor or the GNOME calendar companion** — both read/write the same
encrypted `notion.db`, and neither changed.

## Where it lives

```
open-notebook/
  open-notebook-core/   Injectable engine (workspace member, unit-tested in CI)
    src/storage.rs        NotebookStorage trait + MemStorage + SqliteStorage
    src/schema.rs         Additive migrations (embeddings, sources, logs, FTS)
    src/embedding.rs      Embedder trait, HashingEmbedder, cosine, blob (de)serialize
    src/memory.rs         Index + hybrid (vector + keyword) search
    src/ingestion.rs      Record + index sources; optional summary
    src/studio.rs         Summarize / answer / transform via an LLM
    src/agents.rs         Prompt → validated action → execute + log
    src/gateway.rs        LlmClient trait + Ollama backend (feature ollama-http)
    src/mcp.rs            MCP tool registry + JSON-RPC dispatch
  notion-cli/           Terminal client (standalone crate)
  notion-mcp/           Localhost MCP HTTP server (standalone crate)
apps/desktop/src-tauri/src/ai.rs   Tauri command layer + Notebook service bundle
apps/desktop/src/ai/actions.ts     Pure UI logic (vitest-tested)
apps/desktop/src/components/AIStudio.tsx, AiDialog.tsx   AI UI
```

## Phase 0 — the fork as a clean library

`open-notebook-core` is a library crate with **no GUI/WebView dependencies**. It
follows the same feature layout as `notion_core`/`notion-companion`:

- default `sqlcipher` for the real DB backend; `--no-default-features` gives a
  pure-logic build (the fast CI job) that still exercises every service against
  the in-memory `MemStorage`.
- `ollama-http` gates the only networked code (the Ollama client).
- Dependency versions are aligned with `notion_core` (rusqlite 0.32, serde 1,
  thiserror 2) so the merged Tauri binary links **one** copy of SQLCipher.

The storage seam (**Action 3**) is the `NotebookStorage` trait; the host injects
an already-unlocked connection. The crate **never runs Argon2id and never
manages the DB key** (**Action 4**) — it receives the raw SQLCipher key the main
app already derived from the DEK.

## Phase 1 — additive schema

`schema::MIGRATION_SQL` creates `embeddings`, `ingested_sources`, `agent_logs`,
and a `notebook_fts` FTS5 index — all `CREATE TABLE IF NOT EXISTS`, run on every
open. A test asserts the batch contains no `DROP`/`ALTER` and never references
the main app's own tables (`pages`, `sync_updates`, `doc_snapshots`,
`page_search`). The calendar's `calendar_events` columns are untouched, so the
companion stays 100% compatible (**Phase 5.1/5.2**).

> The spec's FTS table was named `block_fts`; we use `notebook_fts` to avoid a
> name clash with the app's existing `page_search` FTS5 table. Both live inside
> the SQLCipher file, so their shadow tables are page-encrypted.

## Phase 2 — backend integration

`apps/desktop/src-tauri/src/ai.rs` holds the `Notebook` service bundle
(`SqliteStorage` + `MemoryService` + `IngestionService` + `StudioService` +
Ollama gateway). On unlock, `AppState::install` opens the notebook against the
same `notion.db` via a **second WAL connection** (concurrent with the editor's
writer) and runs the migrations — **best-effort**, so a failure never blocks
unlock. Commands exposed to the WebView:

| Command                                 | Does                                                       |
| --------------------------------------- | ---------------------------------------------------------- |
| `notebook_enabled`                      | reports the feature flag (drives conditional UI)           |
| `semantic_search`                       | hybrid vector + keyword search                             |
| `reindex_page`                          | index a page's text into vector memory (on save)           |
| `ingest_text` / `list_sources`          | ingest pasted text; list sources                           |
| `run_agent`                             | natural-language action (add event / create page / answer) |
| `studio_summarize` / `studio_transform` | LLM content transforms                                     |
| `list_agent_logs`                       | agent transparency log                                     |

The DB key never crosses into JS — same posture as the rest of the app.

## Phase 3 — frontend

- **AI Studio** drawer (`AIStudio.tsx`): tabs for semantic search, quick text
  ingestion, and the agent activity log.
- **Ask AI ✨** dialog (`AiDialog.tsx`): the magic-wand floating button and the
  `/ai` slash command both open it. "Add a calendar event for tomorrow at 3pm"
  or "make a page for Q3 planning" run the action agent.
- **Drag-and-drop ingestion**: dropping text or `.txt`/`.md` files ingests them;
  PDFs are directed to the CLI (the native extractor lives at the edge).

All the fiddly decisions (prompt normalization, drop classification, score
formatting, outcome labels) are the pure `ai/actions.ts` module, unit-tested
with vitest; the React components are thin glue.

## Phase 4 — CLI & MCP

- **`notion-cli`** reads the same encrypted DB (key from the GNOME Keyring or
  `NOTION_SQLCIPHER_KEY_HEX`): `search`, `ingest`, `ask`, `summarize`,
  `sources`, `logs`. No GUI.
- **`notion-mcp`** runs a localhost JSON-RPC 2.0 server exposing `search_notes`,
  `create_page`, and `add_event` so external clients (Claude Desktop, Cursor, a
  browser extension) can edit the vault. It binds **loopback only** — it resolves
  the bind address and refuses it unless every resolved socket is loopback. The
  routing/validation is the tested `open_notebook_core::mcp` module; the binary
  is a thin HTTP shell.

## Phase 5 — calendar compatibility

The agent and the `add_event` MCP tool write straight into the app's existing
`calendar_events` table. An integration test
(`storage::sqlite_tests::agent_writes_land_in_the_main_app_tables`) proves an
AI-added event lands in that exact table, which the read-only companion daemon
then surfaces with no code change. The companion opens the DB with
`PRAGMA query_only = TRUE` (already enforced in `notion_core`), so it can never
write or disturb the CRDT `sync_updates` log.

## Phase 9 — rollback flag

Every AI feature is gated by the `ENABLE_OPEN_NOTEBOOK` environment variable.
Unset (the default), the notebook is never opened and each command returns a
clear "disabled" message; the app runs exactly as before. This ships the merge
safely and lets the AI features roll out incrementally.

## Semantic search without a model

Indexing/search use a deterministic **feature-hashing** embedder by default, so
search works fully offline and reproducibly (and is unit-testable). Swapping in
Ollama embeddings is a one-line change in `Notebook::open` — it only requires a
re-index, since stored vectors would change dimension/space. Generative features
(summarize, agent planning, studio) always use the local Ollama gateway.

## Tests

`open-notebook-core` carries 61 tests (55 pure + 6 SQLCipher integration): JSON
extraction, embedding math + blob round-trip, memory ranking + hybrid boost,
ingestion, studio prompts, agent validation/clamping/overflow, MCP dispatch,
and the real encrypted-DB backend. `notion-mcp` adds loopback-guard tests, and
the frontend adds 13 vitest cases for the pure AI UI logic. The CLI and MCP
server build in dedicated CI jobs.

## Status

Merged to `main`, and CI is green across all eight jobs — Rust core, frontend,
desktop (which also bundles an installable `.deb`), the three companion jobs,
and the two Open Notebook jobs (`notion-cli`, `notion-mcp`).

An adversarial review pass then hardened the untrusted-input edges:

- Agent + MCP event timestamps use **saturating** arithmetic, so an extreme
  model/client value (e.g. `i64::MIN`/`i64::MAX`) is clamped to the default
  block instead of overflowing (a debug panic / release wraparound before).
- The `notion-mcp` loopback guard **resolves** the bind address and requires
  every resolved socket to be loopback (a textual `127.` prefix check was
  bypassable by a hostname that resolves off-host).
- Id generation falls back to a timestamp if the RNG is unavailable, so it can
  never emit a constant id that would collide via `ON CONFLICT`.
