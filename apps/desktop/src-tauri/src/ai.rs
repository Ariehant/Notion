//! Open Notebook integration: the AI service bundle + the Tauri commands that
//! expose semantic search, ingestion, the studio, and the action agent to the
//! WebView.
//!
//! The engine lives in the tested `open_notebook_core` crate; this file is the
//! thin shell that (a) opens a *second* SQLCipher connection to the same
//! `notion.db` the editor uses — WAL mode makes concurrent readers/writers safe —
//! and (b) marshals command arguments. All AI features hang off the
//! `ENABLE_OPEN_NOTEBOOK` flag (Phase 9 rollback): with it unset, the notebook is
//! never opened and every command returns a clear "disabled" error.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use open_notebook_core::gateway::{OllamaClient, DEFAULT_CHAT_MODEL, DEFAULT_OLLAMA_URL};
use open_notebook_core::ingestion::{InlineTextExtractor, SourceKind};
use open_notebook_core::storage::{AgentLog, IngestedSource, NotebookStorage, SqliteStorage};
use open_notebook_core::{
    AgentRunner, HashingEmbedder, IngestionService, MemoryService, StudioService,
};
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

/// Environment flag that gates every Open Notebook feature (Phase 9).
pub const ENABLE_ENV: &str = "ENABLE_OPEN_NOTEBOOK";
const DB_FILE: &str = "notion.db";
const DEFAULT_SEARCH_LIMIT: usize = 10;

/// Whether the AI features are switched on. Any non-empty, non-"0" value enables.
pub fn enabled() -> bool {
    std::env::var(ENABLE_ENV)
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// The live Open Notebook services, bound to the unlocked shared database.
pub struct Notebook {
    storage: SqliteStorage,
    memory: MemoryService<HashingEmbedder>,
    ingestion: IngestionService,
    studio: StudioService,
    ollama_url: String,
    chat_model: String,
}

impl Notebook {
    /// Open the notebook against the vault's `notion.db` with the (already
    /// derived) raw SQLCipher key. Runs the additive Open Notebook migrations.
    pub fn open(vault_dir: &Path, sqlcipher_key_hex: &str) -> Result<Self, String> {
        let path = vault_dir.join(DB_FILE);
        let storage = SqliteStorage::open(&path.to_string_lossy(), sqlcipher_key_hex)
            .map_err(|e| format!("open notebook DB: {e}"))?;
        Ok(Self {
            storage,
            // Offline, deterministic embeddings so search works without a model.
            // Swapping in an Ollama embedder is a one-liner (requires re-index).
            memory: MemoryService::new(HashingEmbedder::default()),
            ingestion: IngestionService::new(),
            studio: StudioService::new(),
            ollama_url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| DEFAULT_OLLAMA_URL.to_string()),
            chat_model: std::env::var("OLLAMA_MODEL")
                .unwrap_or_else(|_| DEFAULT_CHAT_MODEL.to_string()),
        })
    }

    fn llm(&self) -> OllamaClient {
        OllamaClient::new(self.ollama_url.clone(), self.chat_model.clone())
    }

    fn storage(&self) -> &dyn NotebookStorage {
        &self.storage
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A random 128-bit id, hex-encoded, prefixed for readability in the DB.
fn new_id(prefix: &str) -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom::getrandom(&mut bytes);
    format!("{prefix}-{}", hex::encode(bytes))
}

/// Run `f` with the open notebook, or fail with a clear message if AI is
/// disabled or the vault is locked.
fn with_notebook<T>(
    state: &State<AppState>,
    f: impl FnOnce(&Notebook) -> Result<T, String>,
) -> Result<T, String> {
    if !enabled() {
        return Err("Open Notebook AI features are disabled (set ENABLE_OPEN_NOTEBOOK=1)".into());
    }
    let guard = state.notebook.lock().map_err(|_| "state poisoned")?;
    let notebook = guard.as_ref().ok_or("vault is locked")?;
    f(notebook)
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct SearchHitDto {
    #[serde(rename = "sourceId")]
    pub source_id: String,
    pub score: f32,
    /// Best-effort human title (from an ingested source), else the id.
    pub title: String,
}

#[derive(Serialize)]
pub struct AgentOutcomeDto {
    pub kind: String,
    pub message: String,
    #[serde(rename = "createdId")]
    pub created_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Whether AI features are enabled (drives conditional UI in the frontend).
#[tauri::command]
pub fn notebook_enabled() -> bool {
    enabled()
}

/// Hybrid semantic + keyword search over indexed notes and ingested sources.
#[tauri::command]
pub fn semantic_search(
    state: State<AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<SearchHitDto>, String> {
    with_notebook(&state, |nb| {
        let k = limit.unwrap_or(DEFAULT_SEARCH_LIMIT).clamp(1, 50);
        let hits = nb
            .memory
            .search(nb.storage(), &query, k)
            .map_err(|e| e.to_string())?;
        Ok(hits
            .into_iter()
            .map(|h| {
                let title = nb
                    .storage()
                    .get_source(&h.source_block_id)
                    .ok()
                    .flatten()
                    .map(|s| s.title)
                    .unwrap_or_else(|| h.source_block_id.clone());
                SearchHitDto {
                    source_id: h.source_block_id,
                    score: h.score,
                    title,
                }
            })
            .collect())
    })
}

/// Index (or re-index) a page's text into semantic memory so it is searchable.
/// Called by the frontend alongside the FTS `index_page` on save.
#[tauri::command]
pub fn reindex_page(
    state: State<AppState>,
    page_id: String,
    title: String,
    body: String,
) -> Result<(), String> {
    with_notebook(&state, |nb| {
        nb.memory
            .index(nb.storage(), &page_id, &title, &body, now_secs())
            .map_err(|e| e.to_string())
    })
}

/// Ingest pasted text as a source (recorded + indexed). Returns the source row.
#[tauri::command]
pub fn ingest_text(state: State<AppState>, text: String) -> Result<IngestedSource, String> {
    with_notebook(&state, |nb| {
        let id = new_id("src");
        nb.ingestion
            .ingest(
                nb.storage(),
                &nb.memory,
                &InlineTextExtractor,
                &id,
                SourceKind::Text,
                &text,
                now_secs(),
            )
            .map_err(|e| e.to_string())
    })
}

/// List ingested sources, newest first.
#[tauri::command]
pub fn list_sources(state: State<AppState>) -> Result<Vec<IngestedSource>, String> {
    with_notebook(&state, |nb| {
        nb.storage().list_sources().map_err(|e| e.to_string())
    })
}

/// Run the action agent on a natural-language prompt (the "magic wand" / `/ai`).
#[tauri::command]
pub fn run_agent(
    state: State<AppState>,
    prompt: String,
    block_id: Option<String>,
) -> Result<AgentOutcomeDto, String> {
    with_notebook(&state, |nb| {
        let runner: AgentRunner<OllamaClient> = AgentRunner::new(nb.llm());
        let outcome = runner
            .run(
                nb.storage(),
                &prompt,
                block_id.as_deref(),
                &new_id("row"),
                now_secs(),
            )
            .map_err(|e| e.to_string())?;
        Ok(AgentOutcomeDto {
            kind: outcome.action.kind().to_string(),
            message: outcome.message,
            created_id: outcome.created_id,
        })
    })
}

/// Summarize arbitrary text via the local LLM.
#[tauri::command]
pub fn studio_summarize(state: State<AppState>, text: String) -> Result<String, String> {
    with_notebook(&state, |nb| {
        nb.studio
            .summarize(&nb.llm(), &text)
            .map_err(|e| e.to_string())
    })
}

/// Rewrite text according to an instruction via the local LLM.
#[tauri::command]
pub fn studio_transform(
    state: State<AppState>,
    text: String,
    instruction: String,
) -> Result<String, String> {
    with_notebook(&state, |nb| {
        nb.studio
            .transform(&nb.llm(), &text, &instruction)
            .map_err(|e| e.to_string())
    })
}

/// The agent transparency log (what the AI did), newest first.
#[tauri::command]
pub fn list_agent_logs(
    state: State<AppState>,
    limit: Option<i64>,
) -> Result<Vec<AgentLog>, String> {
    with_notebook(&state, |nb| {
        nb.storage()
            .list_agent_logs(limit.unwrap_or(50).clamp(1, 500))
            .map_err(|e| e.to_string())
    })
}
