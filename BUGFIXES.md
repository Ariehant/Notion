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

## 🔍 Adversarial security review — findings found & fixed

After the initial build, five independent reviewers audited the crypto, SSRF,
sanitizer, CRDT, and persistence code with distinct lenses. Confirmed findings
below were fixed and covered by regression tests.

### 🐞 CODE BUG #2 — [CRITICAL] Grindable pairing SAS → MITM could steal the DEK

- **Where:** `core/src/crypto/pairing.rs` (`sas_code`).
- **Defect:** the SAS was a deterministic, unsalted ~36-bit hash of only the two
  X25519 device keys — no session nonce, no commit-then-reveal, and it never
  bound the Ed25519 identity that `PairingGrant::accept` anchors trust on. An
  active relay could pick its substituted keys _after_ seeing the victims' and
  grind ~2¹⁸ keygens (seconds) to force both displayed SAS strings to match,
  then wrap the DEK to its own key → full DEK compromise despite a "verified"
  pairing.
- **Fix:** each device now contributes a fresh random **nonce** and exchanges a
  hash **commitment** (`PairingContribution::commitment`) before revealing its
  contribution — so keys/nonce cannot be chosen adaptively, defeating grinding.
  The SAS transcript now binds **both device keys, both identity keys, and both
  nonces**.
- **Tests:** `commitment_detects_post_commit_key_change`,
  `sas_binds_identity_and_nonce`, `full_commit_reveal_and_grant_flow`.

### 🐞 CODE BUG #3 — [HIGH] Flush-chain poisoning → silent, permanent data loss

- **Where:** `apps/desktop/src/crdt/persistence.ts`.
- **Defect:** a single rejected `sink.flush` left the internal promise chain in a
  rejected state; every subsequent flush's `.then` callback never ran, so the
  already-dequeued batch was dropped and **all future edits were silently lost**
  for the object's lifetime (fire-and-forget flushes surfaced nothing).
- **Fix:** replaced the chain with a single-flight drain loop that `catch`es
  errors, **re-queues the failed batch at the front** (preserving order), and
  schedules a backoff retry — a failure never poisons future writes.
- **Tests:** `re-queues and retries after a failed flush — no data loss, no poisoning`.

### 🐞 CODE BUG #4 — [MEDIUM] No real backpressure → unbounded memory

- **Where:** `apps/desktop/src/crdt/persistence.ts`.
- **Defect:** the size threshold only reset `pending`; under a slow/stalled sink
  the buffered bytes were merely relocated onto an ever-growing promise chain
  (a memory-exhaustion DoS). The "bounded memory" comment was false.
- **Fix:** when the buffer exceeds `maxBytes` the pending updates are
  **coalesced** with `Y.mergeUpdates` into a single update, bounding retained
  memory to the merged-delta size regardless of edit rate.
- **Tests:** `bounds memory by coalescing when the sink stalls`.

### 🐞 CODE BUG #5 — [MEDIUM] Trailing-dot `localhost.` bypassed the SSRF name block

- **Where:** `core/src/net/ssrf.rs` (`guard_url`).
- **Defect:** `http://localhost./` parses to the domain `"localhost."`, which is
  neither `== "localhost"` nor `ends_with(".localhost")`, so it skipped the block
  and (via the embed path) reached loopback.
- **Fix:** normalize the host by stripping trailing dots (`trim_end_matches('.')`)
  and lowercasing before the local-name check.
- **Tests:** `rejects_localhost_names_including_trailing_dot`.

### 🐞 CODE BUG #6 — [MEDIUM] `sandboxed_embed` accepted unresolved domains (SSRF)

- **Where:** `core/src/sanitize.rs` (`sandboxed_embed`).
- **Defect:** the `NeedsDnsCheck` (domain) arm did nothing, so
  `sandboxed_embed("https://internal.example/…")` (or a rebinding host) emitted
  an iframe pointing at an internal service — contradicting the "SSRF-guarded"
  guarantee. Iframes cannot be DNS-pinned by us.
- **Fix:** embeds now require an **allowlisted provider host**
  (`EMBED_HOST_ALLOWLIST`, matched exactly or as a subdomain); bare IP hosts are
  rejected outright.
- **Tests:** `embed_rejects_non_https_ssrf_and_unknown_hosts`,
  `embed_allows_known_provider_subdomains`.

### 🐞 CODE BUG #7 — [LOW] Incomplete IP blocklists (`0.0.0.0/8`, IPv6-embedded IPv4)

- **Where:** `core/src/net/ssrf.rs` (`is_blocked_ip`).
- **Defect:** only the single address `0.0.0.0` was blocked (not the whole
  `0.0.0.0/8`); and IPv4 addresses embedded in IPv6 via the deprecated
  IPv4-**compatible** form (`::a.b.c.d`), **6to4** (`2002::/16`), and **NAT64**
  (`64:ff9b::/96`) were not unwrapped, so e.g. `2002:7f00:1::` (6to4 of
  `127.0.0.1`) or `64:ff9b::7f00:1` (NAT64 of loopback) passed as public.
- **Fix:** block the full `0.0.0.0/8` (`o[0] == 0`), and add `embedded_ipv4()`
  which extracts the embedded v4 from mapped/compatible/6to4/NAT64 forms and
  runs it through the v4 blocklist. Public embeddings stay allowed (precise).
- **Tests:** `blocks_cloud_metadata_and_locals`,
  `blocks_ipv6_locals_and_embedded_v4`, `allows_public_addresses`.

### 🐞 CODE BUG #8 — [LOW] Encoding tag truncated before validation

- **Where:** `core/src/db.rs` (`load_updates`).
- **Defect:** `enc_tag as u8` narrowed the `i64` column _before_ validation, so a
  tampered/corrupt value like `257` truncated to `1` and was mis-accepted as
  `V1` instead of erroring.
- **Fix:** validate the full `i64` with `u8::try_from(...).and_then(from_tag)`
  before narrowing.
- **Tests:** `rejects_out_of_range_encoding_tag`.

### 🐞 CODE BUG #9 — [LOW] Non-contributory X25519 agreement not rejected (defensive)

- **Where:** `core/src/crypto/keys.rs` (`WrappedDek::seal_to`, `unwrap_dek`).
- **Defect (defensive):** X25519 `diffie_hellman` was used without a
  `was_contributory()` check, so a low-order `ephemeral_pub` in a malicious wrap
  could force an all-zero shared secret. Not exploitable in the current
  `accept`-gated flow, but hardened before the API surface grows.
- **Fix:** reject non-contributory agreements with `CryptoError::WeakKeyAgreement`.
- **Tests:** `low_order_ephemeral_is_rejected`.

### 🐞 CODE BUG #10 — [LOW] `seal` errors mislabeled as `Decryption`

- **Where:** `core/src/crypto/aead.rs` (`seal_with_aad`).
- **Defect:** an encryption failure was mapped to `CryptoError::Decryption`
  (cosmetic; not exploitable, not an oracle).
- **Fix:** added a distinct `CryptoError::Encryption` variant.

> Reviewers also **verified sound** (no change needed): the raw-key PRAGMA is
> injection-safe (`validate_raw_key`), `temp_store=MEMORY` holds and WAL doesn't
> spill plaintext, the persistence origin/feedback-loop handling, the X25519
> unknown-key-share binding, constant-time comparisons, DEK password-independence,
> recovery-code parsing, zeroization, nonce freshness/AAD handling, HKDF
> label separation, and the full ammonia XSS surface (script/style/svg/mathml/
> event-handler/js-url/srcset stripping, sandbox policy, and src escaping).

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

---

## 🖥️ Desktop build-out — second adversarial review (vault, commands, editor)

Turning the tested core into a running Tauri app added new code: the vault
lifecycle (`vault.rs`), the Tauri command surface (`commands.rs`/`state.rs`),
and the CRDT block editor (`crdt/blocks.ts`, `textdiff.ts`, `components/Editor.tsx`,
`App.tsx`). A fresh 4-dimension adversarial review (crypto/vault, Tauri surface,
editor/CRDT, persistence) with independent per-finding verification confirmed
8 defects. All were fixed and are covered by tests where testable.

### 🐞 [MED] create() wrote `notion.db` before `vault.json`, bricking the vault on a mid-create failure

- **Where:** `apps/desktop/src-tauri/src/vault.rs`, `create`.
- **Defect:** the DB (keyed by the fresh DEK) was created **before** `vault.json`
  — the only durable copy of the wrapped DEK — was written. If the metadata write
  failed (ENOSPC) or the process was killed in between, an encrypted `notion.db`
  was stranded under a DEK whose wraps never hit disk. Because `exists()` keys off
  `vault.json`, the app offered "create" again, and opening the leftover DB with a
  new DEK failed `file is not a database` forever.
- **Fix:** persist `vault.json` **first**, then open/create the DB. A crash after
  the meta write leaves `exists() == true`, and `unlock()` re-derives the same DEK
  from the password wrap and creates the DB with the correct key on next run.

### 🐞 [MED] `write_meta` had no fsync — power loss could truncate the sole copy of the wrapped DEK

- **Where:** `vault.rs`, `write_meta`.
- **Defect:** temp-file-then-rename gives atomicity against an app crash, but with
  no `fsync` the rename can reach disk before the temp file's bytes, so power loss
  could leave a truncated `vault.json`. Since it holds both DEK wraps, that means
  permanent, silent loss of the whole database.
- **Fix:** `File::sync_all()` the temp file before the rename, and fsync the
  containing directory after it, so the bytes are durable before the name flips.

### 🐞 [MED] Caret offset diverged from the model on multi-line blocks (wrong split/merge index)

- **Where:** `components/Editor.tsx`.
- **Defect:** the model was fed from `innerText` (which renders `<br>` as `\n`) but
  the caret was measured with `Range.toString()` (which drops `<br>`). A soft line
  break made the two disagree, so Enter split/merge cut at the wrong index and lost
  a character.
- **Fix:** the block is now a strict single text node — read/write via
  `textContent` (never `innerText`), and intercept every key that would insert a
  `<br>`. Offsets measured with `Range.toString()` and the `textContent` model now
  always line up.

### 🐞 [MED] Keydown ignored IME composition (Enter/Backspace hijacked from the IME)

- **Where:** `components/Editor.tsx`, `onKeyDown`.
- **Defect:** pressing Enter to confirm a CJK/IME candidate split the block; a
  mid-composition Backspace merged blocks.
- **Fix:** early-return while `e.nativeEvent.isComposing || e.keyCode === 229`, so
  the IME keeps those keys.

### 🐞 [MED] `delete_page` could leave orphaned encrypted rows (async flush race)

- **Where:** `core/src/db.rs` (schema) + persistence flush.
- **Defect:** a debounced update/snapshot flush landing **after** a page was
  deleted re-inserted encrypted rows for a now-dead `doc_id`, orphaning them.
- **Fix:** `sync_updates.doc_id` and `doc_snapshots.doc_id` now
  `REFERENCES pages(id) ON DELETE CASCADE` (foreign_keys is ON). A late flush fails
  the INSERT (surfaced via `onError`, harmlessly dropped since the provider is
  destroyed) and deleting a page cascades to its updates + snapshots. Regression
  tests: `append_for_missing_page_is_rejected`,
  `deleting_page_cascades_to_updates_and_snapshots`.

### 🐞 [MED] Shared rename timer dropped the previous page's title edit on fast switch

- **Where:** `apps/desktop/src/App.tsx`.
- **Defect:** one debounce timer + a closure over `activeId` meant switching pages
  within the debounce window fired the rename against the **new** page (or dropped
  it), silently losing the previous page's title edit.
- **Fix:** the pending rename is stored keyed by page id and flushed immediately on
  `activeId` change / unmount / lock.

### 🐞 [MED] Enter in a code block ended the block instead of inserting a newline

- **Where:** `components/Editor.tsx` + `crdt/blocks.ts`.
- **Defect:** Enter always split, and `codeBlock` was not a "continues" type, so a
  code block could not contain multiple lines via Enter.
- **Fix:** in a code block Enter inserts a `\n` (Shift+Enter is the escape hatch);
  Shift+Enter is a soft line break in other block types. Newlines go through the
  model as literal `\n` in the single text node.

### 🐞 [LOW] DEK-derived sync key was held/cleared as a non-zeroizing `[u8; 32]`

- **Where:** `apps/desktop/src-tauri/src/state.rs`, `vault.rs`.
- **Defect:** the sync-AEAD key (copied out of the zeroizing `ContentKeys`) sat in a
  plain array, contradicting the §2.6 zeroize-on-drop guarantee; locking merely
  dropped it without wiping.
- **Fix:** hold it end-to-end in `zeroize::Zeroizing<[u8; 32]>`, so it is wiped on
  lock/replace/drop.
