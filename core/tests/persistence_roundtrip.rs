//! End-to-end proof of the app's real data path — the flow the Tauri commands
//! (`persist_updates` / `load_updates` / `take_snapshot`) actually run, exercised
//! through the public core API with a real Yjs update, real AEAD sealing, and a
//! real SQLCipher database.
//!
//! Only meaningful with the encrypted DB, so gate the whole file on the feature.
#![cfg(feature = "sqlcipher")]

use notion_core::crdt::{CrdtDocument, UpdateEncoding};
use notion_core::crypto::{open, seal, DataKey, SealedBox};
use notion_core::db::EncryptedDb;
use yrs::updates::decoder::Decode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact, Update};

/// Produce a real v1 Yjs update carrying `text` under the "content" key — the
/// exact wire format the frontend editor persists.
fn yjs_update_with(text: &str) -> Vec<u8> {
    let doc = Doc::new();
    let t = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        t.insert(&mut txn, 0, text);
    }
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&StateVector::default())
}

/// Read the "content" text out of a full v1 state blob (what a restore yields).
fn read_content(state_v1: &[u8]) -> String {
    let doc = Doc::new();
    let update = Update::decode_v1(state_v1).unwrap();
    doc.transact_mut().apply_update(update).unwrap();
    let t = doc.get_or_insert_text("content");
    let txn = doc.transact();
    t.get_string(&txn)
}

#[test]
fn update_survives_seal_store_load_open_roundtrip() {
    // Keys exactly as the vault derives them: DEK -> content keys.
    let dek = DataKey::generate();
    let content = dek.content_keys();
    let db = EncryptedDb::open_in_memory(&content.sqlcipher_hex()).unwrap();
    db.create_page("p1", "Note", 1).unwrap();

    let update = yjs_update_with("Hello, encrypted world");

    // persist_updates: seal with the sync key, store the opaque bytes.
    let sealed = seal(&content.sync_aead, &update).unwrap();
    db.append_update("p1", UpdateEncoding::V1, &sealed.to_bytes(), 10)
        .unwrap();

    // load_updates: read back, decrypt, and confirm the bytes are byte-identical
    // (so the stored ciphertext really is our update and nothing else).
    let stored = db.load_updates("p1").unwrap();
    assert_eq!(stored.len(), 1);
    let recovered = open(
        &content.sync_aead,
        &SealedBox::from_bytes(&stored[0].sealed).unwrap(),
    )
    .unwrap();
    assert_eq!(
        recovered, update,
        "decrypted update must equal what was sealed"
    );

    // The decrypted bytes are still a valid Yjs update that replays to the doc.
    let cd = CrdtDocument::new();
    cd.apply_update_v1(&recovered).unwrap();
    assert_eq!(read_content(&cd.encode_full_v1()), "Hello, encrypted world");
}

#[test]
fn wrong_sync_key_cannot_decrypt_stored_update() {
    let content = DataKey::generate().content_keys();
    let db = EncryptedDb::open_in_memory(&content.sqlcipher_hex()).unwrap();
    db.create_page("p1", "Note", 1).unwrap();
    let sealed = seal(&content.sync_aead, &yjs_update_with("secret")).unwrap();
    db.append_update("p1", UpdateEncoding::V1, &sealed.to_bytes(), 1)
        .unwrap();

    let stored = db.load_updates("p1").unwrap();
    let sealed = SealedBox::from_bytes(&stored[0].sealed).unwrap();
    // A different DEK's sync key must fail the AEAD (integrity), not silently
    // return garbage.
    let other = DataKey::generate().content_keys();
    assert!(open(&other.sync_aead, &sealed).is_err());
}

#[test]
fn snapshot_rebuilds_from_log_and_restores() {
    // Mirrors take_snapshot: rebuild the doc from the encrypted log, snapshot the
    // full state, seal + store it, then restore from the stored snapshot.
    let content = DataKey::generate().content_keys();
    let db = EncryptedDb::open_in_memory(&content.sqlcipher_hex()).unwrap();
    db.create_page("p1", "Note", 1).unwrap();

    for (i, chunk) in ["alpha ", "beta ", "gamma"].iter().enumerate() {
        // Each edit is an independent update appended to the log.
        let update = yjs_update_with(chunk);
        let sealed = seal(&content.sync_aead, &update).unwrap();
        db.append_update("p1", UpdateEncoding::V1, &sealed.to_bytes(), i as i64)
            .unwrap();
    }

    // Rebuild the merged document from the log.
    let doc = CrdtDocument::new();
    for s in db.load_updates("p1").unwrap() {
        let update = open(
            &content.sync_aead,
            &SealedBox::from_bytes(&s.sealed).unwrap(),
        )
        .unwrap();
        doc.apply_update_v1(&update).unwrap();
    }
    let snap = doc.snapshot(1_700_000_000_000, Some("autosave".into()));
    let sealed = seal(&content.sync_aead, &snap.state_v1).unwrap();
    let id = db
        .save_snapshot(
            "p1",
            snap.label.as_deref(),
            &sealed.to_bytes(),
            snap.created_at_ms,
        )
        .unwrap();
    assert!(id > 0);

    // Restore: decrypt the latest snapshot and rebuild the document from it.
    let latest = db.latest_snapshot("p1").unwrap().unwrap();
    assert_eq!(latest.label.as_deref(), Some("autosave"));
    let state = open(
        &content.sync_aead,
        &SealedBox::from_bytes(&latest.sealed).unwrap(),
    )
    .unwrap();
    let restored = CrdtDocument::from_state_v1(&state).unwrap();

    // The three concurrent single-char inserts all landed (CRDT merge); order of
    // concurrent inserts at the same position isn't guaranteed, so assert on
    // content membership rather than exact ordering.
    let text = read_content(&restored.encode_full_v1());
    for word in ["alpha", "beta", "gamma"] {
        assert!(
            text.contains(word),
            "restored snapshot missing {word:?}: {text:?}"
        );
    }
}
