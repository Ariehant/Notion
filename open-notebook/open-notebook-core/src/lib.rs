//! `open_notebook_core` — the "Open Notebook" fork, restructured as a clean,
//! injectable Rust library and merged into the Notion desktop backend.
//!
//! The upstream project shipped ingestion, a vector "memory", an AI "studio",
//! action-executing "agents", a CLI, and an MCP server as a monolith that owned
//! its own storage and keys. This crate is the Phase-0 restructure
//! (see `docs/OPEN_NOTEBOOK.md`):
//!
//! * **No GUI / WebView deps.** Pure logic + optional native backends.
//! * **Injected storage.** Every service takes a [`storage::NotebookStorage`];
//!   the host (the Tauri app / CLI) opens the *already-unlocked* SQLCipher
//!   connection and hands it in. This crate never runs Argon2id, never sees the
//!   password, and never manages the DB key.
//! * **Shared schema, no duplication.** The [`schema`] migrations only *add*
//!   tables (`embeddings`, `ingested_sources`, `agent_logs`, `notebook_fts`).
//!   Agents write AI-generated rows straight into the main app's existing
//!   `pages` and `calendar_events` tables, so the GNOME calendar companion — which
//!   reads the same file — shows them with zero code changes.
//!
//! Module map:
//!
//! | Module        | Role                                                        |
//! |---------------|-------------------------------------------------------------|
//! | [`storage`]   | `NotebookStorage` trait + in-memory + SQLCipher impls       |
//! | [`schema`]    | Additive migrations for the Open Notebook tables            |
//! | [`embedding`] | `Embedder` trait, deterministic `HashingEmbedder`, cosine   |
//! | [`memory`]    | Index + semantic/hybrid search over blocks and sources      |
//! | [`ingestion`] | Record + index PDF/URL/text/audio sources, optional summary |
//! | [`studio`]    | Summarize / answer / transform via an LLM                   |
//! | [`agents`]    | Turn a prompt into a validated, logged action               |
//! | [`gateway`]   | `LlmClient` trait + optional Ollama HTTP backend            |
//! | [`mcp`]       | Model Context Protocol tool registry + JSON-RPC dispatch    |
//!
//! Everything except the network call and the SQLCipher I/O is pure and
//! unit-tested; those two edges hang off the `ollama-http` and `sqlcipher`
//! features so the fast CI job builds neither.

pub mod agents;
pub mod embedding;
pub mod gateway;
pub mod ingestion;
pub mod jsonutil;
pub mod mcp;
pub mod memory;
pub mod schema;
pub mod storage;
pub mod studio;

pub use agents::{AgentAction, AgentOutcome, AgentRunner};
pub use embedding::{blob_to_vec, cosine_similarity, vec_to_blob, Embedder, HashingEmbedder};
pub use gateway::{GatewayError, LlmClient};
pub use ingestion::{ExtractedDoc, IngestionService, SourceExtractor, SourceKind};
pub use memory::{MemoryService, SearchResult};
pub use storage::{
    AgentLog, EmbeddingRecord, IngestedSource, MemStorage, NotebookCalendarEvent, NotebookStorage,
    StorageError,
};
pub use studio::StudioService;

/// The embedding dimensionality used across the crate. Chosen small enough that
/// exact cosine ranking over every stored vector stays cheap for a personal
/// knowledge base, and large enough to keep feature-hash collisions rare.
pub const EMBEDDING_DIM: usize = 256;
