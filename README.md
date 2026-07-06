# Notion — offline-first, encrypted desktop app

An offline-first, end-to-end-encryptable, Notion-like desktop application built
on the **corrected** technology blueprint (see `BLUEPRINTAUDIT.md`). This
repository implements the audit's fixes from day one rather than shipping the v1
blueprint's factual errors and security gaps.

- **Shell:** Tauri 2.0 · **Frontend:** React + TypeScript (Vite)
- **Editor doc:** Yjs (authoritative) · **Store/relay:** `yrs` (opaque)
- **At rest:** SQLite + **SQLCipher** (linked directly, not via the stock plugin)
- **Crypto:** Argon2id → HKDF subkeys → XChaCha20-Poly1305 AEAD; per-device
  wrapped DEK + Ed25519 identity; printable recovery kit

## What's here

| Path                      | What                                                                                    | Status                        |
| ------------------------- | --------------------------------------------------------------------------------------- | ----------------------------- |
| `core/`                   | Rust engine: crypto, CRDT store, SQLCipher DB, SSRF guard, sanitizer                    | **built + tested (58 tests)** |
| `apps/desktop/src/`       | Frontend logic: batched persistence, snapshot scheduler, sanitize routing, block schema | **built + tested (18 tests)** |
| `apps/desktop/src-tauri/` | Tauri command layer wiring `notion_core`                                                | scaffolded                    |
| `BUGFIXES.md`             | Every audit finding quoted → code that resolves it                                      | —                             |
| `docs/ARCHITECTURE.md`    | Decisions (collaboration model, source of truth, key pipeline)                          | —                             |

## Getting started

Prerequisites: Rust (stable), Node 20+, pnpm, and a C toolchain (for the bundled
SQLCipher/OpenSSL build).

```bash
# Rust core — compiles SQLCipher (bundled, vendored OpenSSL) and runs all tests
cargo test

# fast crypto/CRDT/SSRF/sanitizer tests without the heavy SQLCipher build
cargo test --no-default-features

# Frontend logic tests + checks
pnpm install
pnpm -r test
pnpm -r typecheck
pnpm -r lint
```

Building the actual desktop bundle requires the platform WebView dev libraries
(e.g. `libwebkit2gtk-4.1-dev` on Linux) and the Tauri CLI; see
`docs/ARCHITECTURE.md` for why `src-tauri` is kept out of the default CI build.

## Security posture (implemented)

- Encrypted at rest (SQLCipher, raw key, `temp_store=MEMORY`).
- E2E multi-device key distribution: random DEK wrapped per device, Ed25519
  identity, device-pairing SAS, printable recovery kit — no silent data loss.
- SSRF-guarded web capture; one HTML sanitizer for pasted **and** scraped
  content; embeds only in sandboxed iframes.

A formal external security review is a **required gate before the E2E sync
release** (audit §5, Phase 2) and has not yet happened.

## Status

Phase 0 (de-risking spikes) and the security-critical Phase 1 foundations are
implemented and tested. The full editor (BlockNote + custom blocks), databases,
sync relay, web capture runtime, and local AI are scoped in `BLUEPRINTAUDIT.md`
§5 and tracked from there.
