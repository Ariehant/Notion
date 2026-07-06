//! CRDT store & version history — audit §1.3, §1.4, §1.6.
//!
//! **Source of truth (§1.4):** the live, in-editor document is owned by *Yjs*
//! in the WebView. This Rust/`yrs` side is an **opaque byte store + merge/sync
//! relay** — it never mutates the document independently. It only:
//!   * appends encoded updates to an append-only log (persisted async, §1.6),
//!   * merges updates so a compact state can be produced,
//!   * takes **full-document binary snapshots** for version history.
//!
//! **Version history (§1.3):** Yjs runs with garbage collection *on*, so native
//! `snapshot()` cannot reconstruct deleted content. We therefore implement
//! restore points as explicit **full-document copies** ([`DocSnapshot`]) taken
//! on a schedule/threshold — not via Yjs's GC-sensitive snapshot API.
//!
//! **Wall-clock (§1.2):** Yjs clocks are Lamport, not HLC. "When was this
//! edited" is an explicit `created_at` the caller supplies here; it is never
//! derived from the CRDT clock.

use thiserror::Error;
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

/// Update wire-encoding variant. Persisted updates are tagged with this so a
/// future switch to v2 stays unambiguous (§1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UpdateEncoding {
    V1 = 1,
    V2 = 2,
}

impl UpdateEncoding {
    pub fn tag(self) -> u8 {
        self as u8
    }
    pub fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(UpdateEncoding::V1),
            2 => Some(UpdateEncoding::V2),
            _ => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum CrdtError {
    #[error("failed to decode CRDT update")]
    Decode,
    #[error("failed to apply CRDT update")]
    Apply,
    #[error("unknown update encoding tag: {0}")]
    UnknownEncoding(u8),
}

/// A full-document restore point (§1.3). `created_at` is caller-supplied
/// wall-clock (§1.2), independent of the CRDT's internal Lamport clock.
#[derive(Debug, Clone)]
pub struct DocSnapshot {
    /// Full document state encoded as a v1 update.
    pub state_v1: Vec<u8>,
    /// Wall-clock millis when the snapshot was taken (caller-supplied).
    pub created_at_ms: i64,
    /// Optional human label ("autosave", "before import", …).
    pub label: Option<String>,
}

/// An opaque CRDT document backed by `yrs`. Not the source of truth for live
/// editing (§1.4) — a store/merge surface for persistence and sync.
pub struct CrdtDocument {
    doc: Doc,
}

impl Default for CrdtDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl CrdtDocument {
    pub fn new() -> Self {
        CrdtDocument { doc: Doc::new() }
    }

    /// Rebuild a document from a full v1 state (e.g. restoring a snapshot).
    pub fn from_state_v1(state: &[u8]) -> Result<Self, CrdtError> {
        let me = Self::new();
        me.apply_update_v1(state)?;
        Ok(me)
    }

    /// Apply an opaque v1 update produced by Yjs (or another `yrs` peer).
    pub fn apply_update_v1(&self, update: &[u8]) -> Result<(), CrdtError> {
        let update = Update::decode_v1(update).map_err(|_| CrdtError::Decode)?;
        let mut txn = self.doc.transact_mut();
        txn.apply_update(update).map_err(|_| CrdtError::Apply)?;
        Ok(())
    }

    /// Apply a persisted, encoding-tagged update.
    pub fn apply_tagged(&self, encoding: UpdateEncoding, update: &[u8]) -> Result<(), CrdtError> {
        match encoding {
            UpdateEncoding::V1 => self.apply_update_v1(update),
            // v2 support is intentionally deferred; the tag keeps the door open.
            UpdateEncoding::V2 => Err(CrdtError::UnknownEncoding(UpdateEncoding::V2.tag())),
        }
    }

    /// Encode the entire document as a single v1 update (compaction / snapshot).
    pub fn encode_full_v1(&self) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
    }

    /// The document's state vector, encoded v1 — sent to a peer so it can reply
    /// with just the updates we're missing.
    pub fn state_vector_v1(&self) -> Vec<u8> {
        use yrs::updates::encoder::Encode;
        self.doc.transact().state_vector().encode_v1()
    }

    /// Encode only the delta a peer is missing, given its state vector (sync).
    pub fn diff_since_v1(&self, remote_sv_v1: &[u8]) -> Result<Vec<u8>, CrdtError> {
        let sv = StateVector::decode_v1(remote_sv_v1).map_err(|_| CrdtError::Decode)?;
        Ok(self.doc.transact().encode_state_as_update_v1(&sv))
    }

    /// Take a full-document restore point (§1.3).
    pub fn snapshot(&self, created_at_ms: i64, label: Option<String>) -> DocSnapshot {
        DocSnapshot {
            state_v1: self.encode_full_v1(),
            created_at_ms,
            label,
        }
    }

    /// Restore a document from a snapshot's full state.
    pub fn restore(snapshot: &DocSnapshot) -> Result<Self, CrdtError> {
        Self::from_state_v1(&snapshot.state_v1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{GetString, Text, Transact};

    fn write_text(doc: &Doc, at: u32, s: &str) {
        let text = doc.get_or_insert_text("content");
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, at, s);
    }

    fn read_text(doc: &Doc) -> String {
        let text = doc.get_or_insert_text("content");
        let txn = doc.transact();
        text.get_string(&txn)
    }

    #[test]
    fn full_state_round_trip() {
        // Prove wire compatibility of the v1 encoding across documents (§1.4).
        let a = CrdtDocument::new();
        write_text(&a.doc, 0, "hello world");

        let state = a.encode_full_v1();
        let b = CrdtDocument::from_state_v1(&state).unwrap();
        assert_eq!(read_text(&b.doc), "hello world");
    }

    #[test]
    fn incremental_sync_converges() {
        let a = CrdtDocument::new();
        let b = CrdtDocument::new();
        write_text(&a.doc, 0, "abc");

        // B tells A its state vector; A replies with the delta B is missing.
        let b_sv = b.state_vector_v1();
        let delta = a.diff_since_v1(&b_sv).unwrap();
        b.apply_update_v1(&delta).unwrap();
        assert_eq!(read_text(&b.doc), "abc");

        // Concurrent edits merge without loss (CRDT property).
        write_text(&a.doc, 3, "X");
        write_text(&b.doc, 0, "Y");
        let a_sv = a.state_vector_v1();
        let b_sv = b.state_vector_v1();
        a.apply_update_v1(&b.diff_since_v1(&a_sv).unwrap()).unwrap();
        b.apply_update_v1(&a.diff_since_v1(&b_sv).unwrap()).unwrap();
        assert_eq!(read_text(&a.doc), read_text(&b.doc));
    }

    #[test]
    fn snapshot_and_restore() {
        let doc = CrdtDocument::new();
        write_text(&doc.doc, 0, "v1 content");
        let snap = doc.snapshot(1_700_000_000_000, Some("autosave".into()));

        // Continue editing after the snapshot.
        write_text(&doc.doc, 10, " and more");
        assert_eq!(read_text(&doc.doc), "v1 content and more");

        // Restoring the snapshot yields the earlier state (§1.3 restore point).
        let restored = CrdtDocument::restore(&snap).unwrap();
        assert_eq!(read_text(&restored.doc), "v1 content");
        assert_eq!(snap.created_at_ms, 1_700_000_000_000);
        assert_eq!(snap.label.as_deref(), Some("autosave"));
    }

    #[test]
    fn encoding_tag_round_trips() {
        assert_eq!(UpdateEncoding::from_tag(1), Some(UpdateEncoding::V1));
        assert_eq!(UpdateEncoding::from_tag(2), Some(UpdateEncoding::V2));
        assert_eq!(UpdateEncoding::from_tag(9), None);
        assert_eq!(UpdateEncoding::V1.tag(), 1);
    }

    #[test]
    fn apply_tagged_v1_works_v2_deferred() {
        let a = CrdtDocument::new();
        write_text(&a.doc, 0, "hi");
        let state = a.encode_full_v1();

        let b = CrdtDocument::new();
        assert!(b.apply_tagged(UpdateEncoding::V1, &state).is_ok());
        assert!(b.apply_tagged(UpdateEncoding::V2, &state).is_err());
    }

    #[test]
    fn malformed_update_rejected() {
        let doc = CrdtDocument::new();
        assert!(doc.apply_update_v1(&[0xff, 0xff, 0xff]).is_err());
    }
}
