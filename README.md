# Notion — offline-first, encrypted desktop app

An offline-first, end-to-end-encryptable, Notion-like desktop application built
on the **corrected** technology blueprint (see `BLUEPRINTAUDIT.md`). This
repository implements the audit's fixes from day one rather than shipping the v1
blueprint's factual errors and security gaps.

- **Shell:** Tauri 2.0 · **Frontend:** React + TypeScript (Vite)
- **Editor doc:** Yjs (authoritative) · **Store/relay:** `yrs` (opaque)
- **At rest:** SQLite + **SQLCipher** (linked directly, not via the stock plugin)
- **Crypto:** Argon2id → HKDF subkeys → XChaCha20-Poly1305 AEAD; a random
  **DEK** that roots all content encryption, wrapped per password + per device +
  by a printable recovery kit

## What you can do today

It runs as a real desktop app:

- **Create / unlock a vault** with a password (Argon2id → the DEK that encrypts
  everything). Forgot it? Reset with the one-time **recovery code** — your data
  is preserved, because the DEK, not the password, is the root key.
- **Write** in a block editor backed by Yjs: paragraphs, H1–H3, bulleted /
  numbered / to-do lists, quotes, and code — via a slash menu (`/`) or markdown
  shortcuts (`# `, `- `, `[] `, `> `, ` ``` `). Edits merge as CRDT updates and
  are flushed asynchronously (the edit path never blocks on disk).
- **Manage pages** in a sidebar and **full-text search** them — all inside the
  encrypted database.
- Everything is encrypted at rest and never leaves the device.

## Layout

| Path                      | What                                                                              | Status                        |
| ------------------------- | --------------------------------------------------------------------------------- | ----------------------------- |
| `core/`                   | Rust engine: crypto, CRDT store, SQLCipher DB, SSRF guard, sanitizer              | **built + tested (69 tests)** |
| `apps/desktop/src/`       | React app: vault gate, sidebar, CRDT block editor, batched persistence            | **built + tested (51 tests)** |
| `apps/desktop/src-tauri/` | Tauri command layer: vault lifecycle, page/editor/search commands (`notion_core`) | **built + tested (4 tests)**  |
| `BUGFIXES.md`             | Every audit finding quoted → code that resolves it                                | —                             |
| `docs/ARCHITECTURE.md`    | Decisions (collaboration model, source of truth, key pipeline, vault)             | —                             |

## Build & run the desktop app

Prerequisites: Rust (stable), Node 20+, pnpm, and — for the Tauri shell — the
platform WebView + build libraries. On Debian/Ubuntu:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf build-essential
```

Then:

```bash
pnpm install

# Run it (dev): opens the window with hot-reload
pnpm --filter @notion/desktop exec tauri dev

# Build an installable bundle (.deb / .AppImage on Linux, .dmg on macOS, .msi on Windows)
pnpm --filter @notion/desktop exec tauri build
```

The installable artifacts land in `apps/desktop/src-tauri/target/release/bundle/`.
(AppImage bundling needs FUSE + network to fetch its tooling; `--bundles deb`
produces a self-contained `.deb` without either.)

## Tests & checks

```bash
# Rust core — compiles SQLCipher (bundled, vendored OpenSSL) and runs all tests
cargo test
cargo test --no-default-features   # fast crypto/CRDT/SSRF/sanitizer tests, no SQLCipher

# Frontend logic + checks
pnpm -r test
pnpm -r typecheck
pnpm -r lint
```

CI runs three jobs: the Rust core (fmt/clippy/test, incl. the SQLCipher build),
the frontend (typecheck/lint/test), and a **desktop** job that installs the
WebView libraries, runs the `src-tauri` clippy + vault tests, and produces an
installable `.deb` as an artifact. Security-critical logic lives in
`notion_core`, which every job compiles and tests.

## Security posture (implemented)

- **DEK-rooted encryption at rest.** A random 256-bit DEK is generated
  independently of the password; the SQLCipher key and the sync-update AEAD key
  are HKDF-derived **from the DEK**. The password (Argon2id → HKDF) only _wraps_
  the DEK, so a password change / recovery re-wraps it without re-encrypting the
  database. SQLCipher runs with `temp_store=MEMORY` + `secure_delete=ON`.
- **No silent data loss.** A printable recovery kit wraps the DEK independently
  of the password (audit §2.5). Multi-device key distribution (per-device
  wrapped DEK, Ed25519 identity, pairing SAS) is implemented in `core`.
- **SSRF-guarded web capture; one HTML sanitizer** for pasted **and** scraped
  content; embeds only in sandboxed iframes. The WebView gets a strict CSP and a
  minimal Tauri capability set (no shell/fs/http plugins); keys never cross into
  JS (audit §2.6).

A formal external security review is a **required gate before the E2E sync
release** (audit §5, Phase 2) and has not yet happened.

## Status

Phase 0/1 foundations are implemented, tested, and now wired into a running
desktop app. The sync relay, web-capture runtime, databases, and local AI are
scoped in `BLUEPRINTAUDIT.md` §5 and tracked from there.
