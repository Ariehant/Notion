# Blueprint Audit & Development-Ready Corrected Spec
### Offline-First, Notion-like Desktop Application

> Audit of the v1 "Full Technical Blueprint." Verified against primary sources (Yjs `INTERNALS.md`, Zetetic SQLCipher docs, SQLite.org, the `tauri-plugin-sql` issue tracker, vendor repos/pricing). Last updated 2026-07-06.

---

## Context

The v1 blueprint is directionally excellent — **Tauri + Rust + Yjs + SQLite/SQLCipher** is the right family of choices. But it contains **factual errors, internal contradictions, under-specified security, and feature gaps** that would cause rework or shipped-in bugs if built as-is. This document lists each with a concrete fix. Where the blueprint was actually *correct* and I initially doubted it, that's stated too (several "exotic" libraries it cites are real — just immature).

**How to use:** §1–§2 = corrections to apply before coding. §3 = feature-completeness checklist. §4 = corrected stack. §5 = realistic phase plan with the missing engineering tracks. §6 = de-risking spikes + sources.

Severity: **[CRITICAL]** breaks the design as drawn · **[HIGH]** will cause bugs/rework · **[MED]** should fix · **[LOW]** polish/accuracy.

---

## 0. Decision to make first — Collaboration model

The blueprint is **self-contradictory** about its users: "local device is the single source of truth," "privacy-first," single-user — yet it lists a **Person** property, **Comments/Discussion**, "collaborative editing," and awareness (multiple people). This decision determines whether identity/permissions/sharing/cross-user key exchange are "missing-critical" or "out of scope."

| Option | Meaning | Impact |
|---|---|---|
| **A. Single-user, multi-device** (recommended MVP) | One person, own devices. No sharing/guests/roles. | E2E = *device-pairing* (one key across your devices). Person property, comments, awareness become optional. |
| **B. Multi-user collaboration** | Teams, shared pages, guests, roles. | Requires identity, per-page ACLs, per-user keypairs, group-key distribution, presence. ~Doubles Phase 2. |
| **C. Phased** (recommended) | Ship A, architect for B. | Keep CRDT + relay design collaboration-ready; defer ACL/identity. |

**Recommendation: C.** Build A's product surface without painting yourself into a corner on the sync/key design (§2.1). This document assumes **C**.

---

## 1. Development anomalies & corrections

### 1.1 [CRITICAL] `tauri-plugin-sql` does **not** support SQLCipher out of the box
- **Blueprint:** "`tauri-plugin-sql` provides full SQLite integration" + "SQLCipher transparent AES-256 encryption," as a drop-in combo.
- **Reality (verified):** The official plugin has **no built-in SQLCipher/encryption** — long-standing open issues (`tauri-apps/plugins-workspace` #7, #2528; `tauri-plugin-sql` #165). Users rely on forks or bypass the plugin.
- **Nuance:** The adjacent claim "sqlx can't load extensions / you must use rusqlite" is **false** — current `sqlx` has a `load-extension` feature and encrypts via `libsqlite3-sys` `bundled-sqlcipher`.
- **Fix:** Pick one encrypted path and prototype it week 1: (a) a SQLCipher plugin fork (`skorphil/tauri-plugin-sqlcipher`, `daraccrafter/tauri-plugin-sql-sqlcipher`), or (b) `sqlx`/`rusqlite` directly with `bundled-sqlcipher`. **Do not assume the stock plugin encrypts anything.**

### 1.2 [HIGH] Yjs does **not** use a Hybrid Logical Clock (HLC)
- **Blueprint:** "Yjs uses a Hybrid Logical Clock (HLC) for version ordering."
- **Reality (verified vs Yjs `INTERNALS.md`):** Each item has `ID(clientID, clock)` — a **Lamport timestamp**. `clock` increments only on inserts, has **no wall-clock component**; merge is the **YATA** algorithm + a state vector, with `clientID` as tiebreaker. Yjs deliberately avoids wall-clock time (that's why offline edits merge without synced clocks).
- **Impact:** "ties broken by larger nodeId" ≈ correct (it's `clientID`), but "HLC" is wrong and misleads anyone assuming last-writer-by-time semantics — Yjs provides none.
- **Fix:** Correct the text. For "who edited last by wall-clock," store an explicit `editedAt`; never derive it from Yjs's clock.

### 1.3 [HIGH] "Version History" contradicts Yjs garbage collection
- **Blueprint:** §1.3 "Yjs does not retain full history"; §6.1 "GC enabled by default"; yet §3.4/§6.4 promise "Version History — Yjs snapshots."
- **Reality:** Yjs's native `snapshot()` can only restore/diff a past state if **`gc: false`**. With GC on (default), deleted content is dropped and native snapshots can't reconstruct old versions. Both GC statements conflict with the version-history feature.
- **Fix (choose + state):** (a) disable GC to use native snapshots (larger docs), or (b) keep GC on and do version history via **periodic full-document binary copies** in `doc_snapshots`. **Recommend (b)** — make "restore points" explicit full snapshots on a schedule/threshold.

### 1.4 [HIGH] Two CRDT engines, no declared source of truth
- **Blueprint:** Yjs (JS) frontend **and** `y-crdt`/`yrs` (Rust) backend, with no authority stated.
- **Reality:** `yrs` and Yjs are wire-compatible (verified), so co-existing works — but both as authoritative mutators of the same in-process doc is redundant (double memory/GC) and ambiguous.
- **Fix:** Declare the split: **Yjs owns the live in-editor doc; Rust/`yrs` is an opaque byte store + sync relay, never mutating independently.** Pin exact Yjs and `yrs` versions, choose the update-encoding variant (v1 or v2), tag persisted updates with it, add a round-trip conformance test in CI.

### 1.5 [HIGH] `y-indexeddb` is redundant inside a Tauri app
- **Blueprint:** Phase 1.4 uses `y-indexeddb` for "browser-side persistence" **and** SQLite.
- **Reality:** Two persistence layers for the same updates (IndexedDB in WebView + SQLite via Rust) → redundant writes, divergence risk, opaque WebView2 storage.
- **Fix:** SQLite is the source of truth — drop `y-indexeddb` or use it only as a non-authoritative transient cache.

### 1.6 [HIGH] "Synchronously written to SQLite" per change will jank
- **Blueprint:** §1.2 "the change is **synchronously written** to SQLite" on every keystroke/drag.
- **Fix:** Keep the in-memory Yjs doc as the fast path; **batch/debounce** encoded updates and persist **asynchronously** (append to `sync_updates`, flush on idle/interval). Never block the editor on disk.

### 1.7 [HIGH] Vector search: `sqlite-vec` vs `libsql` are alternatives; extension loading fights encryption
- **Blueprint:** §2.8 "use `sqlite-vec` or `libsql` extensions."
- **Reality (verified):** `sqlite-vec` is a **loadable extension** (virtual tables); `libsql` (Turso) is a **SQLite fork with a native vector type** (no extension). They're **alternatives, not a combo.** Extension loading via the stock plugin is undocumented and must coexist with SQLCipher.
- **Fix:** Pick **one**. Most robust: statically compile `sqlite-vec` into your SQLCipher amalgamation (§1.1); else use `sqlx` `load-extension`. Prototype vector + encryption together (§6).

### 1.8 [HIGH] FTS5 over SQLCipher can leak plaintext via temp files
- **Blueprint:** §3.4 FTS5 search over the encrypted DB, assumed fully covered.
- **Reality (verified, Zetetic + SQLite.org):** FTS5 shadow tables inside the DB **are** encrypted (whole-file page encryption). But SQLCipher does **not** encrypt transient temp/sort-spill files by default and doesn't auto-set `SQLITE_TEMP_STORE` — index rebuilds/large sorts can spill **plaintext** to disk.
- **Fix:** Set `PRAGMA temp_store = MEMORY` (or build with `SQLITE_TEMP_STORE=2`). Never use external-content FTS tables pointing at an unencrypted store. Document the tokenizer.

### 1.9 [MED] The Yjs-vs-Automerge benchmark table is internally inconsistent
- **Blueprint:** Concludes "Yjs wins on speed everywhere," yet one row ("snapshot + 1000 updates (50k): Yjs 415 ms vs Automerge 204 ms") shows **Automerge faster**, and 415 ms vs 6–9 ms for the snapshot alone is a suspicious jump.
- **Fix:** Don't present as authoritative. Cite the exact source/version (resembles `dmonad/crdt-benchmarks`) or re-run on your data. The *conclusion* (Yjs is the right pick) still holds on maturity/memory grounds.

### 1.10 [MED] Scraper "disable JavaScript" contradicts "render JS-heavy pages"
- **Blueprint:** §2.7/§3.1 use the WebView **for** JS-rendered pages, but §6.3 says "restrict or disable JavaScript execution."
- **Fix:** Can't do both. Render JS in an **isolated** WebView context (no Tauri IPC, strict CSP), then sanitize output — rather than disabling JS.

### 1.11 [LOW] Optimistic perf numbers & a stray framework list
- **Tauri (verified):** binary single-digit MB ✓; but idle memory realistically **~30–40 MB** (not ~18 MB) and Electron cold start realistically **~1–2 s** (not 4–6 s). Directionally right, two figures inflated — don't quote as guarantees. Note the **WebView2 runtime dependency** on Windows (bundle the evergreen bootstrapper).
- Diagram lists "React / Svelte / Vue"; §2.2 picks React — clean up to React only.
- **The "exotic" libraries are real, not hallucinated (verified):** `Obscura` (Rust headless browser, V8 via `deno_core`, CDP, stealth), `Servo` embeddable `0.1.0` on crates.io (very new), `hocuspocus-rs` (`yrs`-based, v0.1.x). **Plate Plus €299** accurate. Caveats: all three tools are **immature** (2026, v0.1.x) — pin versions, keep a fallback. And `hocuspocus-rs` is a **server** → it belongs in the relay (Phase 2), not "embedded in the Tauri backend" for local editing.

---

## 2. Security & privacy corrections

### 2.1 [CRITICAL] Multi-device key distribution is undefined — the single biggest gap
- **Blueprint:** §5.1 keys derived from `password + salt` (Argon2id); §2.3 relay auth is a "signature-based challenge."
- **Problem:** E2E multi-device sync needs the **same data key on every device**, but there's **no device-pairing / authenticated key-exchange / identity keypair** — and the "signature-based challenge" implies an asymmetric keypair never defined in the key pipeline. How does device #2 get the key? Add/revoke a device? Unspecified. This is the hardest part of any E2E system.
- **Fix:** A per-user **identity keypair** (Ed25519) for relay auth + device enrollment; a random **data-encryption key (DEK)** *wrapped* per device (new device shows a QR/emoji SAS code; existing device wraps DEK to the new device's public key). Keep the DEK independent of the password (so password change ≠ full re-encryption). Reference: 1Password Secret Key / Signal device linking.

### 2.2 [HIGH] Derive subkeys with HKDF, not by slicing the master key
- **Blueprint:** §5.1 "Master key → split into SQLCipher / XChaCha20 / HMAC keys."
- **Fix:** Run the Argon2id output through **HKDF-SHA256** with distinct `info` labels (`"sqlcipher"`, `"sync-aead"`, …). Slicing 32 raw bytes invites key correlation/reuse.

### 2.3 [HIGH] XChaCha20-Poly1305 is already authenticated — the separate "HMAC key" is redundant
- **Blueprint:** §2.6/§5.2 add an "HMAC key (message auth)" and per-packet HMAC.
- **Reality:** XChaCha20-**Poly1305** is an AEAD (integrity + authenticity). A separate HMAC is redundant unless it covers *associated data / cleartext routing metadata*.
- **Fix:** Drop it, or state exactly what it covers (prefer AEAD associated-data for envelope headers).

### 2.4 [HIGH] Nonce management unspecified (reuse = catastrophic)
- **Fix:** Specify **random 24-byte nonces per message** (XChaCha20's extended nonce makes random nonces safe — say so). Each stored encrypted update in the append-only log needs a unique nonce.

### 2.5 [HIGH] No account recovery = permanent, total data loss
- **Reality:** Password-derived key, no recovery → forgotten password loses **all** local + synced data. Acceptable as a zero-knowledge stance only if **deliberate and disclosed**.
- **Fix:** A **recovery kit** (printable recovery key wrapping the DEK), or optional escrow, warned at setup. No silent irreversible loss.

### 2.6 [MED] Crypto parameter hygiene
- **Argon2id** 64 MB/t=3/p=1 is acceptable but low for disk unlock; tune to ~0.5–1 s (consider 128–256 MB).
- **SQLCipher:** "AES-256" is imprecise (SQLCipher 4 = AES-256-CBC + HMAC-SHA512 + PBKDF2). With an Argon2id-derived key, pass a **raw key** (`PRAGMA key` raw hex) to avoid a double KDF; set cipher params explicitly.
- **Keys in JS heap:** `zeroize` (Rust) is good, but never hand keys/plaintext-keys to the WebView/JS layer (no zeroize there; GC copies memory). Keep all crypto + key material in Rust; expose decrypted *content* only.

### 2.7 [HIGH] Scraper isolation overstated; SSRF unaddressed
- **Blueprint:** §5.3 "no filesystem access," "isolated sandbox."
- **Reality:** A Tauri WebView isn't automatically fs-isolated — it runs with app privileges unless you engineer isolation (separate WebView, no IPC/API injection, strict CSP, isolation pattern). An **out-of-process** engine (Obscura/Servo child process) is a stronger boundary.
- **SSRF (missing):** Scraping user URLs unguarded lets a malicious note hit `localhost`, `127.0.0.1`, `169.254.169.254` (cloud metadata), private ranges, `file://`. **Add SSRF guards:** allow only `http`/`https`, resolve + block loopback/link-local/private IPs, cap redirects. `robots.txt` ≠ SSRF protection.

### 2.8 [HIGH] Arbitrary iframe/embed + pasted rich text = XSS / remote-content risk
- **Blueprint:** §3.1 "Embed → Arbitrary iframe" + embeds; §3.3 rich-text paste; §5.3 sanitizes *scraped* content only.
- **Fix:** (a) Render embeds in **`sandbox`ed iframes** (never `allow-scripts` + `allow-same-origin` together) behind a strict CSP; never expose Tauri IPC to embedded content. (b) Run **all** untrusted HTML — scraped **and pasted** — through one sanitizer (DOMPurify-equivalent) before it becomes a block. Paste is an equal, currently-unguarded XSS vector.

### 2.9 [MED] Relay abuse controls & scraping legality
- **Relay:** Even zero-knowledge, needs storage quotas, message TTL/GC for offline queues, rate limiting, abuse controls — undefined in §2.3.
- **Scraping legality:** Obscura ships **stealth/fingerprint-randomization**; UA rotation is itself mild evasion. Add a legal/ToS posture (CFAA/contract/copyright exceed `robots.txt`); default to respectful scraping, evasion opt-out-by-default.

---

## 3. Missing features (so you don't ship a half-clone)

Signature Notion features omitted from v1. **★ = users notice immediately.**

**Editor blocks**
- ★ **Toggle lists / toggle headings** (collapsible) — absent
- ★ **Multi-column / column layouts** (side-by-side blocks) — absent
- ★ **Callout blocks** — absent
- ★ **Simple inline table** (non-database) — absent (only DB "Table View" listed)
- Audio block, PDF-embed block — absent

**Linking & knowledge graph**
- ★ **Backlinks / linked references** (bidirectional links) — absent; core to Notion's value
- ★ **@-mentions** (pages/people/dates) + **inline page-link autocomplete** — absent
- **Reminders & date notifications** — absent
- **Linked database** (live view of a DB living elsewhere) — absent (only inline DB block)

**Database**
- **Formula engine** — listed as a property but **no design** for the language/evaluator (a large sub-project); scope explicitly
- **View-level aggregations** (sum/avg/count footers, per-group rollups) — absent
- **Bidirectional relation sync**, **sub-items/dependencies** (timeline), **recurring entries**, **AND/OR filter groups + multi-level sort saved per view** — specify
- **Import from Notion / Evernote / Word / HTML** (whole pages) — only CSV/Markdown-into-DB listed

**Workspace, collaboration & data ownership**
- ★ **Sharing / permissions / roles / guests** — entirely absent (tie to §0)
- **Presence / awareness / live cursors** — collab editing claimed, no awareness protocol (`y-protocols/awareness`)
- **Public "share to web" publishing** — absent
- ★ **Full-workspace export + backup/restore** — absent (only per-DB CSV/JSON). Essential for "total user ownership" and disaster recovery
- **Duplicate page**, **"Move to"**, **Trash retention/auto-purge** — partial/absent
- **Print / export page to PDF**, **word count** — absent
- **Page & database Templates gallery + template buttons** — only DB "Templates" mentioned

**Cross-cutting engineering tracks absent from all 5 phases**
- ★ **Testing/QA strategy** (unit/integration/e2e) — none
- ★ **CI/CD pipeline** — none
- **Undo/redo** as first-class Phase-1 (Yjs `UndoManager`) — mentioned only late in §6.4
- **Accessibility (a11y/ARIA/keyboard nav)** and **i18n/localization/RTL** — none
- **Backup/restore & CRDT-corruption recovery** — mentioned once as "automatic repair," undefined
- **App-data schema migration / CRDT-format versioning** across releases — under-specified
- **Security audit / threat-model review gate** before shipping crypto (Phase 2) — none
- **Dependency licensing/legal review** (BlockNote, Tiptap/ProseMirror, WebKitGTK LGPL, crates) + **code-signing cert + notarization cost** — none
- **Onboarding / first-run / docs** — none

---

## 4. Corrected technology stack

| Layer | Choice | Correction vs v1 |
|---|---|---|
| Desktop shell | **Tauri 2.0** | keep; bundle WebView2 bootstrapper; realistic perf numbers |
| Frontend | **React + TypeScript** | keep; remove "Svelte/Vue" from diagram |
| Block editor | **BlockNote** (ProseMirror) | keep; confirm Yjs collab binding; budget custom blocks (toggle, columns, callout, DB views) |
| CRDT | **Yjs** (frontend, authoritative) + **`yrs`** (Rust, opaque store/relay only) | declare source of truth (§1.4); pin versions + encoding |
| Local DB | **SQLite + SQLCipher** | **not** via stock `tauri-plugin-sql` — SQLCipher fork or `sqlx`/`rusqlite` + `bundled-sqlcipher` (§1.1); `PRAGMA temp_store=MEMORY` (§1.8) |
| Vectors (Phase 4) | **`sqlite-vec` statically linked** (pick one, not "or libsql") | §1.7 |
| At-rest keys | Argon2id → **HKDF** subkeys → OS keychain | §2.2, §2.6 |
| Transport | TLS 1.3 + XChaCha20-Poly1305 AEAD | drop redundant HMAC; specify nonces (§2.3–2.4) |
| Identity/sync keys | **Ed25519 identity + wrapped DEK + device pairing** | new — closes §2.1 |
| Scraping | out-of-process **Obscura/Servo** (JS) or `reqwest`+`scraper` (static) | immature deps → pin + fallback; SSRF guards (§2.7) |
| Local AI | **Ollama** (`/api/generate`, `/api/chat`) / LM Studio | accurate ✓ |

---

## 5. Revised, executable phase plan

v1 timelines are **optimistic by a large factor** — "Phase 1 in 4–6 weeks = full offline Notion clone with ALL core features, usable as a product" is unrealistic (Notion's editor+DB+views alone is many months for a small team). Ranges below are **MVP-scoped** with the missing tracks added.

- **Phase 0 — De-risking spikes (1 wk, NEW):** prove the risky combos (§6). Stand up **CI + test harness + linting** now.
- **Phase 1 — Offline core (realistically 8–12+ wks):** BlockNote + custom blocks (**toggle, columns, callout, simple table**), Yjs↔SQLite (async/batched) persistence, **undo/redo (UndoManager)**, page tree, nested pages, trash, FTS5 search, slash menu, drag-drop, **5 DB view types**, formula-engine spike. **a11y + i18n scaffolding from day one.**
- **Phase 2 — Sync + E2E (4–6 wks):** identity keypair + **device pairing + wrapped DEK** (§2.1), HKDF subkeys, AEAD sync, relay with quotas/TTL/auth, **recovery kit** (§2.5). **Gate: external security review before release.**
- **Phase 3 — Web capture (2–3 wks):** out-of-process scraper, **SSRF guards + sanitizer for scraped *and* pasted HTML**, sandboxed embeds, robots.txt + legal posture.
- **Phase 4 — Local AI (3–4 wks):** Ollama/LM Studio client, streaming, context builder, RAG via the one chosen vector engine (spiked in Phase 0).
- **Phase 5 — Backup, packaging, release (3–4 wks):** **full-workspace export + backup/restore (NEW, first-class)**, migrations/versioning, signing + notarization (budget the cert), auto-update, perf pass, crash reporting.

---

## 6. De-risking spikes (do first) + verification

**Prototype-first checklist — each answers "does the stack even work as drawn?"**
1. **SQLCipher + chosen SQL layer** actually encrypts/opens (fork vs `sqlx`/`rusqlite` + `bundled-sqlcipher`). → §1.1
2. **`sqlite-vec` compiled into the SQLCipher build** loads + queries. → §1.7
3. **Yjs ⇄ `yrs` round-trip** on pinned versions/encoding, in CI. → §1.4
4. **Isolated WebView / out-of-process scraper** with no IPC + SSRF blocklist. → §2.7
5. **Device-pairing + wrapped-DEK** handshake between two machines. → §2.1
6. **FTS5 rebuild with `temp_store=MEMORY`** — confirm no plaintext spill. → §1.8

**Verified against primary sources**
- Yjs is Lamport, not HLC — `yjs/INTERNALS.md`.
- SQLCipher temp-file plaintext + config — `zetetic.net/sqlcipher/design`, `sqlite.org/fts5`.
- `tauri-plugin-sql` no SQLCipher — `plugins-workspace#7`, `#2528`, `tauri-plugin-sql#165`; `sqlx` `load-extension` + `bundled-sqlcipher` — sqlx #1460/#3147/#4010/#4093.
- BLOB vs filesystem thresholds — `sqlite.org/intern-v-extern-blob`, `/fasterthanfs`.
- Tauri vs Electron real numbers — gethopp / levminer benchmarks.
- Libraries real but immature; Plate Plus €299 — Obscura repo, Servo crates.io 0.1.0, `hocuspocus-rs` crates.io, `pro.platejs.org/pricing`.
