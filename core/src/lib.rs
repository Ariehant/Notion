//! `notion_core` — the offline-first, encrypted engine for the desktop app.
//!
//! This crate is the single home for business logic and all security-critical
//! code, so it can be compiled and unit-tested in headless CI (the Tauri shell,
//! which needs GUI system libraries to build, is a thin layer on top).
//!
//! Module map (with the Blueprint-Audit findings each one resolves):
//!
//! | Module        | Audit findings addressed                        |
//! |---------------|-------------------------------------------------|
//! | [`crypto`]    | §2.1 keys/pairing, §2.2 HKDF, §2.3 AEAD, §2.4 nonces, §2.5 recovery, §2.6 hygiene |
//! | [`crdt`]      | §1.3 snapshots, §1.4 source of truth, §1.6 batched persistence |
//! | [`db`]        | §1.1 SQLCipher, §1.8 temp_store, §2.6 raw key    |
//! | [`net`]       | §2.7 SSRF guard                                  |
//! | [`sanitize`]  | §2.8 unified HTML sanitizer                      |

pub mod crdt;
pub mod crypto;
pub mod net;
pub mod sanitize;

#[cfg(feature = "sqlcipher")]
pub mod db;
