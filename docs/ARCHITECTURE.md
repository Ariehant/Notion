# Architecture & Key Decisions

This document records the architectural decisions taken from the Blueprint Audit
(`BLUEPRINTAUDIT.md`) and how the repository is laid out. Every decision here
resolves a specific audit finding; see `BUGFIXES.md` for the finding-by-finding
map to code.

## §0 — Collaboration model: **Option C**

Per the audit's recommendation we ship **single-user / multi-device (A)** but
**architect for multi-user (B)**:

- The product surface is single-user.
- The sync + key design is already collaboration-ready: a per-user Ed25519
  identity, a random data-encryption key (DEK) wrapped per device, and opaque
  CRDT sync updates. Per-page ACLs / group-key distribution are deferred, not
  designed out.

## Layering

```
apps/desktop/                 React + TypeScript (Vite) frontend
  src/                        UI + pure logic (tested with vitest)
  src-tauri/                  Tauri command layer (thin; delegates to core)
core/                         Rust engine — ALL business + security logic
```

**Why the split:** the `tauri` crate needs GUI system libraries (WebKitGTK on
Linux) to build, which headless CI lacks. Keeping every non-UI concern in
`notion_core` means the security-critical code is compiled and unit-tested on
every push, while the desktop bundle is assembled in a GUI-capable job/release
runner.

## Source of truth (§1.4)

- **Yjs (in the WebView) owns the live, in-editor document.** It is the only
  authoritative mutator.
- **`yrs` (Rust) is an opaque byte store + merge/sync surface.** It never mutates
  document content on its own. It persists encoded updates, merges them to
  produce compact state, and takes full-document snapshots.
- Updates are wire-encoded **v1** and tagged with `UpdateEncoding` so a future
  move to v2 stays unambiguous. A round-trip conformance test runs in CI.

## Persistence (§1.5, §1.6)

- **SQLite (SQLCipher) is the single source of truth on disk.** No `y-indexeddb`.
- Edits never block on disk. `BatchedPersistence` (frontend) debounces and
  flushes encoded updates asynchronously to the Rust append-only log
  (`sync_updates`). Flush also fires on a size threshold to bound memory.

## Version history (§1.2, §1.3)

- Yjs runs with GC **on**, so native `snapshot()` cannot rebuild deleted content.
- Restore points are therefore **explicit full-document binary copies**
  (`doc_snapshots`), taken on a schedule/threshold decided by `SnapshotScheduler`.
- "When was this edited" is an explicit, caller-supplied wall-clock value
  (`created_at_ms`). It is **never** derived from the Lamport CRDT clock.

## Encryption at rest (§1.1, §1.8, §2.6)

- SQLCipher is linked directly via `rusqlite` (`bundled-sqlcipher-vendored-openssl`).
  The stock `tauri-plugin-sql` does **not** encrypt and is not used.
- The DB key is the HKDF `sqlcipher` subkey, passed as a **raw** key
  (`PRAGMA key = "x'…'"`) to avoid a second KDF.
- `PRAGMA temp_store = MEMORY` + `secure_delete = ON` so FTS5 rebuilds / large
  sorts cannot spill plaintext to temp files.
- FTS5 lives inside the encrypted DB (no external-content pointing at plaintext).

## Key pipeline (§2.1–§2.6)

```
password ──Argon2id(128MiB,t=3)──▶ master key
                                     │  HKDF-SHA256 (distinct info labels)
                                     ├──▶ sqlcipher subkey  (raw DB key)
                                     ├──▶ sync-aead subkey   (seal updates)
                                     └──▶ dek-wrap subkey     (wrap the DEK)

DEK (random 256-bit, password-independent)
  ├─ wrapped by dek-wrap subkey            → unlock with password
  ├─ wrapped to each device's X25519 key   → multi-device (PairingGrant + SAS)
  └─ wrapped by a recovery key             → RecoveryKit (printable)

Identity: per-user Ed25519 (relay challenge auth + signs device enrollment)
Transport/at-rest updates: XChaCha20-Poly1305 AEAD, random 24-byte nonce,
  envelope metadata bound as associated data. No separate HMAC.
```

All key material is `Zeroize`/`ZeroizeOnDrop` and never crosses into JS.

## Web capture (§2.7, §2.8)

- SSRF guard: http/https only; block loopback/link-local (incl.
  `169.254.169.254`)/private/CGNAT/unique-local/etc.; IPv4-mapped normalization;
  redirect cap. DNS-resolved addresses must also pass `is_blocked_ip`.
- One sanitizer (`ammonia`) for **both** pasted and scraped HTML.
- Embeds render only as **sandboxed** iframes (never `allow-scripts` +
  `allow-same-origin` together), https-only + SSRF-guarded src, no Tauri IPC.

## Deferred (scoped, not built here)

- **Vector search (§1.7):** one engine, `sqlite-vec` statically linked — Phase 4.
- **Formula engine, backlinks, linked DB, aggregations, relations** — Phase 1/3.
- **Relay** (quotas/TTL/rate-limit) + external **security review gate** — Phase 2.
- Full-workspace **export/backup/restore**, migrations, signing/notarization —
  Phase 5.

See `BLUEPRINTAUDIT.md` §5 for the phase plan.
