# GNOME Companion Calendar & Dynamic Island

A native, battery-friendly companion for the Notion desktop app on Ubuntu
24.04+ (GNOME 45+). Instead of booting a second WebView to check your schedule,
three small native processes share the main app's **already-encrypted** SQLite
file and surface your calendar in the GNOME top bar in real time.

```
┌──────────────────────────────────────────────────────────────────────┐
│  MAIN APP (Tauri) — writes calendar_events to the shared SQLCipher DB  │
│         ~/.local/share/co.merai.notion/notion.db                       │
└───────────────────────────────┬──────────────────────────────────────┘
                                 │ inotify (kernel, no polling)
        ┌────────────────────────┴─────────────────────────┐
        ▼                                                    ▼
┌───────────────────────┐   DBus: com.notion.Calendar   ┌───────────────────────┐
│ A. notion-watcher      │  GetTodayEvents / GetUpcoming │ C. notion-quickview    │
│    (Rust daemon)       │  EventsUpdated(json) ────────▶│    (GTK4/libadwaita)   │
│  read-only reader      │                               │  week view · quick-add │
│  broadcasts changes    │            ▲                  │  · Ask AI ✨ (Ollama)  │
└───────────────────────┘            │ signal            └───────────────────────┘
                              ┌───────┴────────────┐
                              │ B. GNOME extension  │
                              │   "Dynamic Island"  │  top-bar agenda, live
                              └─────────────────────┘
```

## Why it saves memory

No data duplication. Every component reads the **same** SQLCipher file the main
app writes — there is exactly one database and one copy of your data. The
watcher holds a read-only connection (`PRAGMA query_only`) and uses `inotify`
instead of polling, so idle cost is a few MB of RAM and ~0% CPU. Checking your
week no longer means spinning up the full React/WebView stack.

## Components

| Path                                        | Component | Build in CI? |
| ------------------------------------------- | --------- | ------------ |
| `notion-companion/`                         | Shared logic: event model, AI parsing, time math, XDG paths, keyring | **yes** — pure logic unit-tested (`cargo test`) |
| `notion-watcher/`                           | **A.** DBus watcher daemon (tokio · inotify · zbus) | yes — dedicated `companion-daemon` job |
| `gnome-extension/notion-island@notion.app/` | **B.** GNOME Shell extension (GJS) | yes — schema + JS syntax job |
| `notion-quickview/`                         | **C.** GTK4/libadwaita quick-view + AI add | yes — `companion-quickview` job (GTK deps) |
| `packaging/`                                | `install.sh` / `uninstall.sh` (per-user) | — |

The **security-critical and logic-heavy** parts live in `notion-companion` and
are unit-tested headless (calendar range/overlap math, natural-language AI
parsing + validation + conflict detection, day-boundary/timezone math, keyring
key validation). The daemon and GTK app are thin native shells on top.

## How the shared key works (security)

The companion needs the SQLCipher key to open the DB. It never runs Argon2id or
sees the password:

1. On unlock, the **main app** derives the raw SQLCipher key from the DEK
   (`DataKey::content_keys().sqlcipher_hex()`) and publishes **only that key**
   to the GNOME Keyring (Secret Service), under `co.merai.notion / sqlcipher-key`.
2. The daemon and quick-view read it back and open the shared DB.
3. On lock, the main app deletes the keyring entry, so the companion locks too.

Publishing the *derived DB key* rather than the DEK root is deliberate
least-privilege: the companion can read/write calendar rows but cannot unwrap
the CRDT sync log or any other DEK-derived secret. Keyring publishing is
best-effort — a headless main app still works; the companion simply has no key
until the next unlock in a graphical session. For development you can bypass the
keyring with `NOTION_SQLCIPHER_KEY_HEX=<64-hex>`.

## DBus interface — `com.notion.Calendar`

Session bus, object path `/com/notion/Calendar` (see
`notion-watcher/data/com.notion.Calendar.xml`):

- `GetTodayEvents() → s` — today + tomorrow as a JSON array string.
- `GetUpcoming(count: i) → s` — the next `count` not-yet-ended events.
- `EventsUpdated(json_data: s)` — broadcast on every DB change.

Payloads are JSON arrays of events with Unix-second timestamps:

```json
[{ "id": "…", "title": "Standup", "startTime": 1700049600, "endTime": 1700053200,
   "allDay": false, "location": "Zoom", "description": null,
   "blockId": null, "lastModified": 1700049600 }]
```

## Local AI "Ask" mode

In the quick-view, **Ask AI ✨** (or `notion-quickview --ask`, which the
extension launches) sends your text to a **local** Ollama instance
(`http://localhost:11434`, no data leaves the machine) with a strict JSON-only
system prompt stamped with the current time. The reply is parsed defensively
(first balanced JSON object only, even through ``` fences), validated (non-empty
title, sane/clamped duration, all-day normalization), and checked against the
next 30 days for conflicts before anything is written. See
`notion-companion/src/ai.rs`.

## Install (per-user, from source)

```bash
# Daemon builds anywhere; the GTK app needs GUI dev libraries:
sudo apt-get install -y libgtk-4-dev libadwaita-1-dev libdbus-1-dev pkg-config

companion/packaging/install.sh      # build + install binaries, service, extension
# then log out/in so GNOME Shell loads the extension, and unlock the main app once.

companion/packaging/uninstall.sh    # remove everything (your data is untouched)
```

The installer places binaries in `~/.local/bin`, a `notion-watcher` **systemd
user** service in `~/.config/systemd/user`, and the extension in
`~/.local/share/gnome-shell/extensions`, then enables both.

## Packaging a `.deb` (daemon)

```bash
cargo install cargo-deb
cargo deb --manifest-path companion/notion-watcher/Cargo.toml
```

Produces a Debian package that drops the daemon binary, its systemd user unit,
and the DBus activation file into place. Per-user `systemctl --user enable
notion-watcher` is still needed after install (a `.deb` cannot enable a *user*
service for every account) — `install.sh` does this for source installs.

## Build & test

```bash
# Shared logic (headless, unit-tested) — part of the root workspace:
cargo test -p notion-companion

# Daemon (standalone crate):
cargo build   --manifest-path companion/notion-watcher/Cargo.toml
cargo clippy  --manifest-path companion/notion-watcher/Cargo.toml --all-targets -- -D warnings

# GTK app (needs GUI dev libraries):
cargo build   --manifest-path companion/notion-quickview/Cargo.toml --release

# Extension:
glib-compile-schemas --strict "companion/gnome-extension/notion-island@notion.app/schemas/"
```
