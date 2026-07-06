//! Ingestion: record a source, index its text into [`crate::memory`], and
//! (optionally) summarize it with an LLM.
//!
//! Turning raw bytes into text — parsing a PDF, fetching+scraping a URL,
//! transcribing audio — is genuine I/O and lives at the *edge* (the Tauri app /
//! CLI supply a [`SourceExtractor`]), so this module stays pure and testable.
//! The bundled [`InlineTextExtractor`] handles pasted text; PDF/URL/audio
//! extractors plug in behind the same trait (the app wires `notion_core::net`'s
//! SSRF guard + `sanitize` into the URL one).

use crate::embedding::Embedder;
use crate::gateway::{GatewayError, LlmClient};
use crate::memory::MemoryService;
use crate::storage::{IngestedSource, NotebookStorage, StorageError};
use crate::studio::StudioService;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("could not extract text from the source: {0}")]
    Extract(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Gateway(#[from] GatewayError),
}

/// The kind of thing being ingested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Text,
    Url,
    Pdf,
    Audio,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Text => "text",
            SourceKind::Url => "url",
            SourceKind::Pdf => "pdf",
            SourceKind::Audio => "audio",
        }
    }

    /// Whether `source_path` is meaningful for this kind (URLs/files yes, inline
    /// text no).
    fn has_path(self) -> bool {
        !matches!(self, SourceKind::Text)
    }
}

/// Text extracted from a source, ready to index.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedDoc {
    pub title: String,
    pub text: String,
}

/// Turns a source location (a path, URL, or the text itself) into plain text.
/// Implemented at the edge for real PDF/URL/audio; the trait keeps this crate
/// I/O-free and unit-testable.
pub trait SourceExtractor {
    fn extract(&self, kind: SourceKind, location: &str) -> Result<ExtractedDoc, IngestError>;
}

/// Extractor for inline pasted text: the location *is* the text. The title is
/// the first non-empty line (trimmed, capped), or "Untitled".
pub struct InlineTextExtractor;

impl SourceExtractor for InlineTextExtractor {
    fn extract(&self, _kind: SourceKind, location: &str) -> Result<ExtractedDoc, IngestError> {
        let title = location
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| l.chars().take(80).collect::<String>())
            .unwrap_or_else(|| "Untitled".to_string());
        Ok(ExtractedDoc {
            title,
            text: location.to_string(),
        })
    }
}

pub struct IngestionService;

impl Default for IngestionService {
    fn default() -> Self {
        Self
    }
}

impl IngestionService {
    pub fn new() -> Self {
        Self
    }

    /// Ingest a source: extract text, record a row in `ingested_sources`, and
    /// index the text into semantic memory. No LLM call — summary is `None`.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest<E: Embedder, X: SourceExtractor>(
        &self,
        store: &dyn NotebookStorage,
        memory: &MemoryService<E>,
        extractor: &X,
        id: &str,
        kind: SourceKind,
        location: &str,
        now: i64,
    ) -> Result<IngestedSource, IngestError> {
        let doc = extractor.extract(kind, location)?;
        let source = IngestedSource {
            id: id.to_string(),
            source_type: kind.as_str().to_string(),
            source_path: kind.has_path().then(|| location.to_string()),
            title: doc.title.clone(),
            summary: None,
            processed_at: now,
        };
        store.insert_source(&source)?;
        memory.index(store, id, &doc.title, &doc.text, now)?;
        Ok(source)
    }

    /// Like [`ingest`](Self::ingest) but also asks the LLM for a summary and
    /// persists it on the source row.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest_and_summarize<E: Embedder, X: SourceExtractor, L: LlmClient>(
        &self,
        store: &dyn NotebookStorage,
        memory: &MemoryService<E>,
        extractor: &X,
        llm: &L,
        id: &str,
        kind: SourceKind,
        location: &str,
        now: i64,
    ) -> Result<IngestedSource, IngestError> {
        let doc = extractor.extract(kind, location)?;
        let summary = StudioService.summarize(llm, &doc.text)?;
        let source = IngestedSource {
            id: id.to_string(),
            source_type: kind.as_str().to_string(),
            source_path: kind.has_path().then(|| location.to_string()),
            title: doc.title.clone(),
            summary: Some(summary),
            processed_at: now,
        };
        store.insert_source(&source)?;
        memory.index(store, id, &doc.title, &doc.text, now)?;
        Ok(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::HashingEmbedder;
    use crate::gateway::testutil::FakeLlm;
    use crate::storage::MemStorage;

    fn memory() -> MemoryService<HashingEmbedder> {
        MemoryService::new(HashingEmbedder::default())
    }

    #[test]
    fn inline_extractor_titles_from_first_line() {
        let doc = InlineTextExtractor
            .extract(SourceKind::Text, "  My Note\nbody line")
            .unwrap();
        assert_eq!(doc.title, "My Note");
        assert_eq!(doc.text, "  My Note\nbody line");
    }

    #[test]
    fn ingest_records_source_and_indexes_it() {
        let store = MemStorage::new();
        let mem = memory();
        let src = IngestionService
            .ingest(
                &store,
                &mem,
                &InlineTextExtractor,
                "s1",
                SourceKind::Text,
                "Budget\nThe Q3 budget spreadsheet numbers",
                100,
            )
            .unwrap();
        assert_eq!(src.source_type, "text");
        assert_eq!(src.source_path, None); // inline text has no path
        assert_eq!(src.title, "Budget");

        // It is now searchable through memory.
        let hits = mem.search(&store, "budget spreadsheet", 5).unwrap();
        assert_eq!(hits[0].source_block_id, "s1");
        // And recorded.
        assert_eq!(store.list_sources().unwrap().len(), 1);
    }

    #[test]
    fn url_kind_keeps_the_path() {
        let store = MemStorage::new();
        let mem = memory();

        struct FakeUrl;
        impl SourceExtractor for FakeUrl {
            fn extract(&self, _k: SourceKind, loc: &str) -> Result<ExtractedDoc, IngestError> {
                assert_eq!(loc, "https://example.com/post");
                Ok(ExtractedDoc {
                    title: "Example Post".into(),
                    text: "scraped article body".into(),
                })
            }
        }

        let src = IngestionService
            .ingest(
                &store,
                &mem,
                &FakeUrl,
                "u1",
                SourceKind::Url,
                "https://example.com/post",
                1,
            )
            .unwrap();
        assert_eq!(src.source_path.as_deref(), Some("https://example.com/post"));
        assert_eq!(src.title, "Example Post");
    }

    #[test]
    fn ingest_and_summarize_persists_summary() {
        let store = MemStorage::new();
        let mem = memory();
        let llm = FakeLlm::new(["This note is about the quarterly budget."]);
        let src = IngestionService
            .ingest_and_summarize(
                &store,
                &mem,
                &InlineTextExtractor,
                &llm,
                "s1",
                SourceKind::Text,
                "Long budget document text ...",
                1,
            )
            .unwrap();
        assert_eq!(
            src.summary.as_deref(),
            Some("This note is about the quarterly budget.")
        );
        assert_eq!(
            store.get_source("s1").unwrap().unwrap().summary,
            src.summary
        );
    }

    #[test]
    fn extraction_failure_is_surfaced() {
        struct Boom;
        impl SourceExtractor for Boom {
            fn extract(&self, _k: SourceKind, _l: &str) -> Result<ExtractedDoc, IngestError> {
                Err(IngestError::Extract("bad pdf".into()))
            }
        }
        let store = MemStorage::new();
        let mem = memory();
        let err = IngestionService
            .ingest(&store, &mem, &Boom, "x", SourceKind::Pdf, "/tmp/x.pdf", 1)
            .unwrap_err();
        assert!(matches!(err, IngestError::Extract(_)));
        // Nothing was recorded on failure.
        assert!(store.list_sources().unwrap().is_empty());
    }
}
