# Bugs Fixed & Audit Findings Resolved

This document quotes each Blueprint-Audit finding that the current codebase
addresses, and points to the code that resolves it. Findings are quoted from
`BLUEPRINTAUDIT.md`. Severity tags are the audit's own:
**[CRITICAL] / [HIGH] / [MED] / [LOW]**.

Two categories appear here:

1. **Design-anomaly fixes** — the blueprint (v1) would have shipped a bug; the
   audit flagged it; this build implements the _corrected_ behavior.
2. **Implementation bugs** — genuine defects introduced while writing this code
   and caught by the test suite before commit. These are marked **🐞 CODE BUG**.

---

## 🐞 Implementation bugs found & fixed during development

### 🐞 CODE BUG #1 — SSRF bypass via IPv4-compatible IPv6 addresses

- **Where:** `core/src/net/ssrf.rs`, `is_blocked_ip`.
- **Defect:** the first draft used `Ipv6Addr::to_ipv4()` to fold IPv4-mapped
  addresses down to their v4 form. But `to_ipv4()` _also_ matches the deprecated
  **IPv4-compatible** range, so `::1` (loopback) was remapped to `0.0.0.1` and
  then judged **public** — a real SSRF bypass letting `http://[::1]/` through.
- **Caught by:** `blocks_ipv6_locals_and_mapped_v4` and
  `rejects_ip_literal_ssrf_targets` (they failed on `::1`).
- **Fix:** switched to `Ipv6Addr::to_ipv4_mapped()`, which only matches true
  IPv4-mapped addresses (`::ffff:a.b.c.d`). `::1` now correctly falls through to
  the IPv6 loopback check and is blocked. Regression tests cover
  `::ffff:127.0.0.1` and `::ffff:169.254.169.254`.

---

## §1 — Development anomalies & corrections

### [CRITICAL] §1.1 — `tauri-plugin-sql` does not support SQLCipher out of the box

> "The official plugin has no built-in SQLCipher/encryption … Do not assume the
> stock plugin encrypts anything."

- **Fix:** encryption goes through `rusqlite` with the
  `bundled-sqlcipher-vendored-openssl` feature — SQLCipher is statically linked,
  no stock plugin involved. See `core/Cargo.toml` and `core/src/db.rs`.
- **Proof:** `encryption_persists_and_wrong_key_fails` (wrong key can't open the
  file) and `raw_file_bytes_are_not_plaintext` (on-disk bytes contain neither
  our plaintext markers nor the `SQLite format 3` magic).

### [HIGH] §1.2 — Yjs does not use a Hybrid Logical Clock

> "Each item has `ID(clientID, clock)` — a Lamport timestamp … For 'who edited
> last by wall-clock,' store an explicit `editedAt`; never derive it from Yjs's
> clock."

- **Fix:** snapshots and updates carry a **caller-supplied `created_at_ms`**
  wall-clock value; nothing is derived from the CRDT clock. See
  `DocSnapshot.created_at_ms` in `core/src/crdt.rs` and the `created_at` columns
  in `core/src/db.rs`. Documented in the `crdt` module header.

### [HIGH] §1.3 — "Version History" contradicts Yjs garbage collection

> "With GC on (default) … native snapshots can't reconstruct old versions …
> **Recommend (b)** — make 'restore points' explicit full snapshots."

- **Fix:** implemented option (b). `CrdtDocument::snapshot()` takes a **full
  document binary copy** (`encode_full_v1`) rather than a GC-sensitive Yjs
  `snapshot()`. Stored in the `doc_snapshots` table. See `core/src/crdt.rs`
  (`snapshot`/`restore`) and `core/src/db.rs` (`save_snapshot`/`list_snapshots`).
- **Proof:** `snapshot_and_restore`.

### [HIGH] §1.4 — Two CRDT engines, no declared source of truth

> "Declare the split: Yjs owns the live in-editor doc; Rust/`yrs` is an opaque
> byte store + sync relay, never mutating independently … choose the
> update-encoding variant (v1 or v2), tag persisted updates with it, add a
> round-trip conformance test in CI."

- **Fix:** the `crdt` module header states the split explicitly. `yrs` is used
  only as an opaque store/merge surface (`apply_update_v1`, `encode_full_v1`,
  `diff_since_v1`) and never mutates content on its own. Persisted updates are
  tagged with `UpdateEncoding` (v1/v2). See `core/src/crdt.rs`.
- **Proof:** `full_state_round_trip`, `incremental_sync_converges`,
  `encoding_tag_round_trips` (round-trip conformance wired into CI via
  `cargo test`).

### [HIGH] §1.5 — `y-indexeddb` is redundant inside a Tauri app

> "SQLite is the source of truth — drop `y-indexeddb`."

- **Fix:** the frontend persistence provider writes **only** to the Rust/SQLite
  layer; `y-indexeddb` is not a dependency. See
  `apps/desktop/src/crdt/persistence.ts` and its header note.

### [HIGH] §1.6 — "Synchronously written to SQLite" per change will jank

> "Keep the in-memory Yjs doc as the fast path; batch/debounce encoded updates
> and persist asynchronously … Never block the editor on disk."

- **Fix:** `BatchedPersistence` debounces Yjs updates and flushes them
  asynchronously (idle/interval/size threshold), never on the edit path. See
  `apps/desktop/src/crdt/persistence.ts`.
- **Proof:** `apps/desktop/src/crdt/persistence.test.ts` (debounce, size-flush,
  and "edits never block" behavior).

### [HIGH] §1.7 / §1.8 — vector search & FTS5 over SQLCipher

> "§1.8 … SQLCipher does not encrypt transient temp/sort-spill files by default
> … index rebuilds/large sorts can spill plaintext to disk. Fix: Set
> `PRAGMA temp_store = MEMORY`."

- **Fix (§1.8):** every connection sets `PRAGMA temp_store = MEMORY` (plus
  `secure_delete = ON`). The FTS5 index is an ordinary table inside the encrypted
  DB (no external-content pointing at an unencrypted store). See
  `core/src/db.rs` (`configure`, `migrate`).
- **Proof:** `temp_store_is_memory`, `fts_search_finds_pages`.
- **§1.7 (vectors):** deferred to Phase 4 per the plan; documented in
  `docs/ARCHITECTURE.md` as "one engine, statically linked," not started here.

---

## §2 — Security & privacy corrections

### [CRITICAL] §2.1 — Multi-device key distribution is undefined

> "A per-user identity keypair (Ed25519) for relay auth + device enrollment; a
> random data-encryption key (DEK) wrapped per device … Keep the DEK independent
> of the password."

- **Fix:** implemented in `core/src/crypto/keys.rs` + `core/src/crypto/pairing.rs`:
  - `DataKey` — random 256-bit DEK, independent of the password.
  - `IdentityKeypair` — Ed25519 identity for relay challenge + enrollment.
  - `DeviceKeypair` (X25519) + `WrappedDek` — DEK sealed to a device's public key.
  - `PairingGrant` + `sas_code` — signed enrollment + Short Authentication String
    for MITM detection.
- **Proof:** `full_pairing_flow`, `dek_cannot_be_unwrapped_by_wrong_device`,
  `grant_from_untrusted_authorizer_rejected`, `sas_is_order_independent_and_stable`.

### [HIGH] §2.2 — Derive subkeys with HKDF, not by slicing the master key

> "Run the Argon2id output through HKDF-SHA256 with distinct `info` labels …
> Slicing 32 raw bytes invites key correlation/reuse."

- **Fix:** `subkeys()` derives each subkey via `HKDF-SHA256` with a distinct
  `info` label (`notion.v1.sqlcipher`, `notion.v1.sync-aead`, `notion.v1.dek-wrap`).
  See `core/src/crypto/kdf.rs`.
- **Proof:** `subkeys_are_distinct_and_stable`.

### [HIGH] §2.3 — XChaCha20-Poly1305 is already authenticated; the separate HMAC is redundant

> "Drop it, or state exactly what it covers (prefer AEAD associated-data for
> envelope headers)."

- **Fix:** no separate HMAC key exists. Integrity/authenticity come from the
  AEAD; routing/envelope metadata is bound as **associated data** via
  `seal_with_aad`/`open_with_aad`. See `core/src/crypto/aead.rs`.
- **Proof:** `aad_is_authenticated`.

### [HIGH] §2.4 — Nonce management unspecified (reuse = catastrophic)

> "Specify random 24-byte nonces per message … Each stored encrypted update in
> the append-only log needs a unique nonce."

- **Fix:** every `seal` generates a fresh random 24-byte XChaCha20 nonce and
  stores it in the self-describing `SealedBox` (`nonce ‖ ciphertext`). See
  `core/src/crypto/aead.rs`.
- **Proof:** `nonces_are_unique_per_seal`.

### [HIGH] §2.5 — No account recovery = permanent, total data loss

> "A recovery kit (printable recovery key wrapping the DEK) … No silent
> irreversible loss."

- **Fix:** `RecoveryKit` generates a printable recovery code whose derived key
  wraps the DEK; the code is independent of the password so it survives password
  changes. See `core/src/crypto/recovery.rs`.
- **Proof:** `recovery_round_trip`, `recovery_tolerates_formatting_noise`.

### [MED] §2.6 — Crypto parameter hygiene

> "Argon2id … tune to ~0.5–1 s (consider 128–256 MB) … With an Argon2id-derived
> key, pass a raw key (`PRAGMA key` raw hex) to avoid a double KDF … never hand
> keys … to the WebView/JS layer."

- **Fix:**
  - Argon2id default is **128 MiB / t=3 / p=1** (`Argon2Params::default`), up
    from the blueprint's 64 MiB. See `core/src/crypto/kdf.rs`.
  - SQLCipher is keyed with a **raw hex key** (`PRAGMA key = "x'…'"`), skipping a
    second PBKDF2. See `core/src/db.rs`.
  - All key types are `Zeroize`/`ZeroizeOnDrop`; keys never cross into JS (the
    Rust core exposes decrypted _content_, not key material).

### [HIGH] §2.7 — Scraper isolation overstated; SSRF unaddressed

> "Add SSRF guards: allow only http/https, resolve + block loopback/link-local/
> private IPs, cap redirects. `robots.txt` ≠ SSRF protection."

- **Fix:** `core/src/net/ssrf.rs` — scheme allowlist (`http`/`https`), IP
  classification blocking loopback/link-local (incl. `169.254.169.254`
  metadata)/private/CGNAT/unique-local/etc., IPv4-mapped normalization, a
  `localhost` name block, and `MAX_REDIRECTS`.
- **Proof:** `blocks_cloud_metadata_and_locals`, `blocks_ipv6_locals_and_mapped_v4`,
  `rejects_dangerous_schemes`, `rejects_ip_literal_ssrf_targets`.

### [HIGH] §2.8 — Arbitrary iframe/embed + pasted rich text = XSS

> "(a) Render embeds in sandboxed iframes (never `allow-scripts` +
> `allow-same-origin` together) … (b) Run all untrusted HTML — scraped and pasted
> — through one sanitizer before it becomes a block."

- **Fix:** `core/src/sanitize.rs` — a single `sanitize_html` used for **both**
  scraped and pasted content (strips scripts, event handlers, JS/data URLs,
  iframes/objects), and `sandboxed_embed` which builds a locked-down `<iframe>`
  whose `sandbox` never combines `allow-scripts` with `allow-same-origin`, with an
  https-only, SSRF-guarded `src`.
- **Proof:** `pasted_html_is_sanitized_same_as_scraped`,
  `sandboxed_embed_is_locked_down`, `embed_rejects_non_https_and_ssrf`.

---

## §3 — Missing engineering tracks now scaffolded

> "★ Testing/QA strategy … none. ★ CI/CD pipeline … none. Undo/redo as
> first-class Phase-1 … a11y/i18n … none."

- **Testing/QA:** Rust unit/integration tests (`cargo test`) and TypeScript unit
  tests (`vitest`) exist from commit one.
- **CI/CD:** `.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy
-D warnings`, `cargo test`, and the TS lint/test pipeline.
- The remaining §3 product features (toggle/callout/column/table blocks,
  backlinks, formula engine, etc.) are scoped in `docs/ARCHITECTURE.md` and the
  block schema is stubbed in `apps/desktop/src/blocks/`.

---

## §0 — Collaboration model decision

The audit requires choosing a collaboration model first. Per its recommendation
we adopt **Option C**: ship single-user/multi-device (A), but architect the
CRDT + key design to be collaboration-ready (identity keypair, wrapped DEK,
opaque sync updates). Recorded in `docs/ARCHITECTURE.md §0`.
