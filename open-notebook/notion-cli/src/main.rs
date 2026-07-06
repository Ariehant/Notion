//! `notion-cli` — terminal access to the Notion knowledge base.
//!
//! Opens the same encrypted `notion.db` the desktop app uses (key from the GNOME
//! Keyring, or `NOTION_SQLCIPHER_KEY_HEX` for headless/dev), then runs the tested
//! `open_notebook_core` engine:
//!
//! ```text
//! notion-cli search <query…>        semantic + keyword search
//! notion-cli ingest <text… | @file> ingest text into the knowledge base
//! notion-cli ask <prompt…>          run the action agent (add event / page / answer)
//! notion-cli summarize <text|@file> summarize via the local LLM
//! notion-cli sources                list ingested sources
//! notion-cli logs                   show recent agent actions
//! ```
//!
//! Anything requiring generation (ask / summarize) needs a local Ollama; search,
//! ingest, sources, and logs work fully offline.

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use notion_companion::keyring::{EnvKeyProvider, KeyProvider, SecretServiceKeyProvider};
use notion_companion::paths;
use open_notebook_core::gateway::{OllamaClient, DEFAULT_CHAT_MODEL, DEFAULT_OLLAMA_URL};
use open_notebook_core::ingestion::{InlineTextExtractor, SourceKind};
use open_notebook_core::storage::{NotebookStorage, SqliteStorage};
use open_notebook_core::{
    AgentRunner, HashingEmbedder, IngestionService, MemoryService, StudioService,
};

const USAGE: &str = "\
notion-cli — terminal access to your encrypted Notion knowledge base

USAGE:
    notion-cli <command> [args…]

COMMANDS:
    search <query…>          Semantic + keyword search over your notes
    ingest <text… | @file>   Ingest text (or a file's contents) into memory
    ask <prompt…>            Run the AI agent (add event / create page / answer)
    summarize <text | @file> Summarize text via the local LLM
    sources                  List ingested sources (newest first)
    logs                     Show recent AI agent actions
    help                     Show this message

ENV:
    NOTION_SQLCIPHER_KEY_HEX   64-hex DB key (else read from the GNOME Keyring)
    NOTION_DB_PATH             Override the DB path (else the shared app-data DB)
    OLLAMA_URL / OLLAMA_MODEL  Local LLM endpoint / model
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some(cmd) = args.first().map(String::as_str) else {
        print!("{USAGE}");
        return Ok(());
    };
    if matches!(cmd, "help" | "-h" | "--help") {
        print!("{USAGE}");
        return Ok(());
    }

    let rest = &args[1..];
    let ctx = Context::open()?;

    match cmd {
        "search" => ctx.search(&join(rest)?),
        "ingest" => ctx.ingest(&read_text_arg(rest)?),
        "ask" => ctx.ask(&join(rest)?),
        "summarize" => ctx.summarize(&read_text_arg(rest)?),
        "sources" => ctx.sources(),
        "logs" => ctx.logs(),
        other => Err(format!("unknown command `{other}` (try `notion-cli help`)")),
    }
}

/// The opened knowledge base + services.
struct Context {
    storage: SqliteStorage,
    memory: MemoryService<HashingEmbedder>,
    ingestion: IngestionService,
    studio: StudioService,
    ollama_url: String,
    chat_model: String,
}

impl Context {
    fn open() -> Result<Self, String> {
        let db_path = match std::env::var("NOTION_DB_PATH") {
            Ok(p) if !p.is_empty() => p,
            _ => paths::db_path()
                .ok_or("cannot resolve the app-data directory (set NOTION_DB_PATH)")?
                .to_string_lossy()
                .into_owned(),
        };
        let key = resolve_key()?;
        let storage = SqliteStorage::open(&db_path, &key)
            .map_err(|e| format!("open DB at {db_path}: {e}"))?;
        Ok(Self {
            storage,
            memory: MemoryService::new(HashingEmbedder::default()),
            ingestion: IngestionService::new(),
            studio: StudioService::new(),
            ollama_url: env_or("OLLAMA_URL", DEFAULT_OLLAMA_URL),
            chat_model: env_or("OLLAMA_MODEL", DEFAULT_CHAT_MODEL),
        })
    }

    fn storage(&self) -> &dyn NotebookStorage {
        &self.storage
    }

    fn llm(&self) -> OllamaClient {
        OllamaClient::new(self.ollama_url.clone(), self.chat_model.clone())
    }

    fn search(&self, query: &str) -> Result<(), String> {
        let hits = self
            .memory
            .search(self.storage(), query, 15)
            .map_err(|e| e.to_string())?;
        if hits.is_empty() {
            println!("No matches.");
        }
        for h in hits {
            let title = self
                .storage()
                .get_source(&h.source_block_id)
                .ok()
                .flatten()
                .map(|s| s.title)
                .unwrap_or_else(|| h.source_block_id.clone());
            println!("  {:>3}%  {}", (h.score * 100.0).round() as i64, title);
        }
        Ok(())
    }

    fn ingest(&self, text: &str) -> Result<(), String> {
        let src = self
            .ingestion
            .ingest(
                self.storage(),
                &self.memory,
                &InlineTextExtractor,
                &new_id("src"),
                SourceKind::Text,
                text,
                now_secs(),
            )
            .map_err(|e| e.to_string())?;
        println!("Ingested “{}” ({} bytes).", src.title, text.len());
        Ok(())
    }

    fn ask(&self, prompt: &str) -> Result<(), String> {
        let runner: AgentRunner<OllamaClient> = AgentRunner::new(self.llm());
        let outcome = runner
            .run(self.storage(), prompt, None, &new_id("row"), now_secs())
            .map_err(|e| e.to_string())?;
        println!("{}", outcome.message);
        Ok(())
    }

    fn summarize(&self, text: &str) -> Result<(), String> {
        let out = self
            .studio
            .summarize(&self.llm(), text)
            .map_err(|e| e.to_string())?;
        println!("{out}");
        Ok(())
    }

    fn sources(&self) -> Result<(), String> {
        for s in self.storage().list_sources().map_err(|e| e.to_string())? {
            println!("  [{}] {}", s.source_type, s.title);
        }
        Ok(())
    }

    fn logs(&self) -> Result<(), String> {
        for l in self
            .storage()
            .list_agent_logs(30)
            .map_err(|e| e.to_string())?
        {
            println!("  {} — {}", l.agent_type, l.action_taken);
        }
        Ok(())
    }
}

/// Try the env-var key first (dev/headless), then the GNOME Keyring.
fn resolve_key() -> Result<String, String> {
    for provider in [
        &EnvKeyProvider as &dyn KeyProvider,
        &SecretServiceKeyProvider,
    ] {
        match provider.sqlcipher_key_hex() {
            Ok(Some(k)) => return Ok(k.as_str().to_string()),
            Ok(None) => continue,
            Err(e) => return Err(format!("keyring: {e}")),
        }
    }
    Err("vault is locked — unlock the desktop app once, or set NOTION_SQLCIPHER_KEY_HEX".into())
}

fn env_or(var: &str, default: &str) -> String {
    std::env::var(var)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A best-effort unique id without pulling in a UUID dependency: nanosecond
/// clock ⊕ pid, hex-encoded. Uniqueness is sufficient for one interactive CLI.
fn new_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{:x}-{:x}", std::process::id(), nanos)
}

/// Join the remaining args into one string (for multi-word queries/prompts).
fn join(rest: &[String]) -> Result<String, String> {
    if rest.is_empty() {
        return Err("expected some text after the command".into());
    }
    Ok(rest.join(" "))
}

/// Like [`join`] but if the single arg is `@path`, read the file's contents.
fn read_text_arg(rest: &[String]) -> Result<String, String> {
    if rest.len() == 1 {
        if let Some(path) = rest[0].strip_prefix('@') {
            return std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"));
        }
    }
    join(rest)
}
