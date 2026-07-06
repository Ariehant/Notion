//! Semantic memory: index content into the vector store + FTS, then rank it.
//!
//! [`MemoryService`] is generic over an [`Embedder`] so tests use the
//! deterministic [`HashingEmbedder`](crate::embedding::HashingEmbedder) while
//! production can inject an Ollama-backed embedder. Search is **hybrid**: it
//! blends dense cosine similarity with sparse full-text hits, which keeps exact
//! keyword matches ("invoice #4021") from being lost by a fuzzy vector while
//! still surfacing semantic neighbours.

use crate::embedding::{content_hash, cosine_similarity, Embedder};
use crate::storage::{EmbeddingRecord, NotebookStorage, StorageError};

/// One search hit: which source matched and how strongly (`[0, 1]`).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub source_block_id: String,
    pub score: f32,
}

/// Weight given to the full-text signal when blending with the vector score.
/// Keyword hits are a strong precision signal, so they get a meaningful but
/// not overwhelming boost on top of cosine similarity.
const FTS_BOOST: f32 = 0.25;

pub struct MemoryService<E: Embedder> {
    embedder: E,
}

impl<E: Embedder> MemoryService<E> {
    pub fn new(embedder: E) -> Self {
        Self { embedder }
    }

    /// Index (or re-index) one unit of content. Idempotent per `source_block_id`:
    /// prior vectors for the same source are cleared first, and the stored
    /// [`content_hash`] lets a caller skip work when the text is unchanged.
    pub fn index(
        &self,
        store: &dyn NotebookStorage,
        source_block_id: &str,
        title: &str,
        content: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        let hash = content_hash(content);
        let vector = self.embedder.embed(content);
        store.delete_embeddings_for_source(source_block_id)?;
        store.upsert_embedding(&EmbeddingRecord {
            id: format!("{source_block_id}:{hash}"),
            source_block_id: source_block_id.to_string(),
            content_hash: hash,
            embedding: vector,
            created_at: now,
        })?;
        // Feed the same text to FTS for the sparse half of hybrid search.
        store.fts_upsert(source_block_id, title, content)?;
        Ok(())
    }

    /// Whether `content` differs from what is already indexed for this source
    /// (lets a caller avoid a redundant re-embed).
    pub fn needs_reindex(
        &self,
        store: &dyn NotebookStorage,
        source_block_id: &str,
        content: &str,
    ) -> Result<bool, StorageError> {
        let hash = content_hash(content);
        Ok(!store
            .all_embeddings()?
            .iter()
            .any(|e| e.source_block_id == source_block_id && e.content_hash == hash))
    }

    /// Rank indexed content against `query`, best first, at most `k` results.
    ///
    /// Score = cosine similarity + [`FTS_BOOST`] for sources that also match the
    /// keyword index. Sources are deduplicated (a source has one embedding), and
    /// non-positive-cosine results with no FTS hit are dropped as noise.
    pub fn search(
        &self,
        store: &dyn NotebookStorage,
        query: &str,
        k: usize,
    ) -> Result<Vec<SearchResult>, StorageError> {
        let qv = self.embedder.embed(query);
        let fts_hits: std::collections::HashSet<String> =
            store.fts_search(query, k as i64 * 4)?.into_iter().collect();

        let mut scored: Vec<SearchResult> = store
            .all_embeddings()?
            .into_iter()
            .map(|e| {
                let mut score = cosine_similarity(&qv, &e.embedding);
                if fts_hits.contains(&e.source_block_id) {
                    score += FTS_BOOST;
                }
                SearchResult {
                    source_block_id: e.source_block_id,
                    score,
                }
            })
            .filter(|r| r.score > 0.0)
            .collect();

        // Descending score; ties broken by id for a stable, deterministic order.
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.source_block_id.cmp(&b.source_block_id))
        });
        scored.truncate(k);
        Ok(scored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::HashingEmbedder;
    use crate::storage::MemStorage;

    fn svc() -> MemoryService<HashingEmbedder> {
        MemoryService::new(HashingEmbedder::default())
    }

    #[test]
    fn indexes_and_finds_the_right_source() {
        let store = MemStorage::new();
        let m = svc();
        m.index(
            &store,
            "b1",
            "Invoices",
            "Acme invoice payment due Friday",
            1,
        )
        .unwrap();
        m.index(
            &store,
            "b2",
            "Trip",
            "hiking trip photos from the mountains",
            1,
        )
        .unwrap();

        let results = m
            .search(&store, "when is the invoice payment due", 5)
            .unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].source_block_id, "b1");
    }

    #[test]
    fn reindex_replaces_not_duplicates() {
        let store = MemStorage::new();
        let m = svc();
        m.index(&store, "b1", "t", "first version of the text", 1)
            .unwrap();
        m.index(&store, "b1", "t", "completely different second version", 2)
            .unwrap();
        assert_eq!(store.all_embeddings().unwrap().len(), 1);
    }

    #[test]
    fn needs_reindex_tracks_content_changes() {
        let store = MemStorage::new();
        let m = svc();
        assert!(m.needs_reindex(&store, "b1", "hello").unwrap());
        m.index(&store, "b1", "t", "hello", 1).unwrap();
        assert!(!m.needs_reindex(&store, "b1", "hello").unwrap());
        assert!(m.needs_reindex(&store, "b1", "hello world").unwrap());
    }

    #[test]
    fn fts_boost_lifts_exact_keyword_match() {
        // Two docs; the query shares an exact rare token with only one. The FTS
        // boost should make that one win even if the dense scores are close.
        let store = MemStorage::new();
        let m = svc();
        m.index(&store, "b1", "A", "meeting notes about zephyr project", 1)
            .unwrap();
        m.index(&store, "b2", "B", "general meeting notes and todos", 1)
            .unwrap();
        let results = m.search(&store, "zephyr", 5).unwrap();
        assert_eq!(results[0].source_block_id, "b1");
    }

    #[test]
    fn respects_k_limit() {
        let store = MemStorage::new();
        let m = svc();
        for i in 0..10 {
            m.index(&store, &format!("b{i}"), "t", "shared common words here", i)
                .unwrap();
        }
        assert!(m.search(&store, "shared common words", 3).unwrap().len() <= 3);
    }
}
