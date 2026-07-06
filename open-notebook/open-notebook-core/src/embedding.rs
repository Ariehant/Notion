//! Vector embeddings for semantic search.
//!
//! Real semantic quality comes from a learned model (see the Ollama embedder in
//! [`crate::gateway`], feature `ollama-http`). But the *engine* — indexing,
//! storage, cosine ranking — must be testable with no model and no network, so
//! the default [`Embedder`] here is a deterministic **feature-hashing** embedder:
//! it hashes bag-of-words tokens into a fixed-dimensional vector. It is not
//! semantically smart, but it is stable, dependency-free, and good enough that
//! "find the note about invoices" ranks invoice notes first in tests.
//!
//! Vectors are stored in the `embeddings.embedding` BLOB as little-endian `f32`
//! (see [`vec_to_blob`] / [`blob_to_vec`]).

use crate::EMBEDDING_DIM;

/// Anything that turns text into a fixed-length vector.
pub trait Embedder {
    /// The (constant) dimensionality of vectors this embedder produces.
    fn dim(&self) -> usize;
    /// Embed `text`. The returned vector always has length [`Embedder::dim`].
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// A deterministic, dependency-free embedder using the feature-hashing trick.
///
/// Each token is hashed twice: once to pick a dimension, once (a sign bit) to
/// pick +1/-1, which halves the expected collision bias. The accumulated vector
/// is L2-normalized so cosine similarity reduces to a dot product and document
/// length does not dominate the score.
#[derive(Debug, Clone)]
pub struct HashingEmbedder {
    dim: usize,
}

impl Default for HashingEmbedder {
    fn default() -> Self {
        Self { dim: EMBEDDING_DIM }
    }
}

impl HashingEmbedder {
    pub fn new(dim: usize) -> Self {
        assert!(dim > 0, "embedding dim must be positive");
        Self { dim }
    }
}

impl Embedder for HashingEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dim];
        for token in tokenize(text) {
            let h = fnv1a64(token.as_bytes());
            let idx = (h % self.dim as u64) as usize;
            // Use a high bit as the sign so it is independent of `idx`.
            let sign = if (h >> 63) & 1 == 1 { 1.0 } else { -1.0 };
            v[idx] += sign;
        }
        l2_normalize(&mut v);
        v
    }
}

/// Split text into lowercase alphanumeric tokens.
pub fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
}

/// FNV-1a 64-bit — a small, *stable* hash (unlike `DefaultHasher`, whose output
/// is not guaranteed across builds). Determinism matters: a re-index must map a
/// token to the same dimension it did last release, or stored vectors drift.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x00000100000001b3);
    }
    hash
}

/// A stable content fingerprint (hex FNV-1a) used to detect whether a source's
/// text changed since it was last indexed. Not cryptographic — a change
/// detector, not a security primitive.
pub fn content_hash(text: &str) -> String {
    format!("{:016x}", fnv1a64(text.as_bytes()))
}

fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Cosine similarity in `[-1, 1]`. Returns `0.0` for a length mismatch or a
/// zero-magnitude vector rather than `NaN`, so ranking never poisons a sort.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0f32;
    let mut na = 0f32;
    let mut nb = 0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

/// Serialize an `f32` vector to a little-endian byte BLOB for the DB.
pub fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Parse a little-endian `f32` BLOB back into a vector. Errors if the length is
/// not a multiple of 4.
pub fn blob_to_vec(blob: &[u8]) -> Result<Vec<f32>, EmbeddingError> {
    if !blob.len().is_multiple_of(4) {
        return Err(EmbeddingError::BadBlobLength(blob.len()));
    }
    Ok(blob
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EmbeddingError {
    #[error("embedding blob length {0} is not a multiple of 4 bytes")]
    BadBlobLength(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_has_fixed_dim_and_is_deterministic() {
        let e = HashingEmbedder::new(64);
        let a = e.embed("Quarterly invoice from Acme");
        let b = e.embed("Quarterly invoice from Acme");
        assert_eq!(a.len(), 64);
        assert_eq!(a, b);
    }

    #[test]
    fn normalized_vectors_are_unit_length() {
        let e = HashingEmbedder::new(128);
        let v = e.embed("hello world foo bar baz");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm was {norm}");
    }

    #[test]
    fn empty_text_yields_zero_vector_without_nan() {
        let e = HashingEmbedder::new(32);
        let v = e.embed("   %%%  ");
        assert!(v.iter().all(|x| *x == 0.0));
        // Cosine with anything is a clean 0.0, never NaN.
        assert_eq!(cosine_similarity(&v, &e.embed("real text")), 0.0);
    }

    #[test]
    fn related_text_ranks_above_unrelated() {
        // The engine's contract: a query is closer to a matching doc than to an
        // unrelated one. Feature hashing preserves shared-token overlap.
        let e = HashingEmbedder::default();
        let q = e.embed("invoice payment due");
        let hit = e.embed("Please pay the outstanding invoice; payment is due Friday");
        let miss = e.embed("Notes from the hiking trip to the mountains");
        assert!(
            cosine_similarity(&q, &hit) > cosine_similarity(&q, &miss),
            "expected the invoice note to outrank the hiking note"
        );
    }

    #[test]
    fn cosine_of_identical_is_one() {
        let e = HashingEmbedder::default();
        let v = e.embed("some repeated content here");
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn cosine_length_mismatch_is_zero() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn blob_round_trips() {
        let v = vec![0.5f32, -1.25, 3.0, 0.0];
        let blob = vec_to_blob(&v);
        assert_eq!(blob.len(), 16);
        assert_eq!(blob_to_vec(&blob).unwrap(), v);
    }

    #[test]
    fn blob_bad_length_errors() {
        assert_eq!(
            blob_to_vec(&[0, 1, 2]),
            Err(EmbeddingError::BadBlobLength(3))
        );
    }

    #[test]
    fn content_hash_is_stable_and_changes_with_content() {
        assert_eq!(content_hash("abc"), content_hash("abc"));
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }
}
