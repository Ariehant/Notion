# Your setup checklist — what to do on your side

Everything Claude built is code-complete, tested, and now lives on `main`. This
file lists the **manual steps only you can do** — installing OS packages, running
the app on real hardware, saving your recovery code, wiring external clients —
grouped by the feature each branch delivered.

> The three feature branches are stacked, so `main` now contains **all** of them:
> `claude/project-dev-bug-fixes-szshd7` (desktop) →
> `claude/gnome-companion-calendar` (companion) →
> `claude/open-notebook-ai-merge` (AI). You only need to work from `main`.

---

## 1. Desktop app — branch `claude/project-dev-bug-fixes-szshd7`

The offline-first, encrypted Notion desktop app (Tauri + React + SQLCipher).

Install prerequisites (Rust stable, Node 20+, pnpm). On Debian/Ubuntu also:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf build-essential
```

Then install JS deps and run or build:

```bash
pnpm install
pnpm --filter @notion/desktop exec tauri dev                    # run with hot-reload
pnpm --filter @notion/desktop exec tauri build --bundles deb    # or build an installer
```

- [ ] Run the prerequisite install for your OS.
- [ ] `pnpm install`, then `tauri dev` (or `tauri build`; artifact lands in
      `apps/desktop/src-tauri/target/release/bundle/`).
- [ ] **On first launch: create a vault and SAVE THE RECOVERY CODE.** It is shown
      exactly once and is the only way back in if you forget the password (the DEK is
      the root key, not the password).
- [ ] Sanity-check: write a page, lock, unlock — content should survive.

## 2. GNOME companion — branch `claude/gnome-companion-calendar`

Background DBus watcher + "Dynamic Island" top-bar agenda + GTK4 quick-view.
Ubuntu 24.04+/GNOME 45+ only.

```bash
sudo apt-get install -y libgtk-4-dev libadwaita-1-dev libdbus-1-dev pkg-config
bash companion/packaging/install.sh    # builds + installs daemon, service, extension
# ...later, to remove (data untouched):
bash companion/packaging/uninstall.sh
```

- [ ] Install the GUI dev libraries above.
- [ ] Run `companion/packaging/install.sh`.
- [ ] **Log out and back in** so GNOME Shell loads the extension.
- [ ] **Unlock the main desktop app once** in your graphical session — that publishes
      the derived SQLCipher key to the GNOME Keyring so the companion can open the
      shared DB. (Locking the app removes it again.)
- [ ] Confirm the agenda shows in the top bar; open the quick-view.
- [ ] _(Optional)_ Install [Ollama](https://ollama.com) for the quick-view's "Ask AI ✨".

## 3. Open Notebook AI — branch `claude/open-notebook-ai-merge`

Semantic search, ingestion, studio, and the action agent — plus a CLI and an MCP
server. **All AI is off by default** behind a flag.

Enable the features by launching the app with the flag set (unset = the app
behaves exactly as before and the AI UI stays hidden):

```bash
ENABLE_OPEN_NOTEBOOK=1 pnpm --filter @notion/desktop exec tauri dev
```

For generative features (summarize, agent, studio), install Ollama and pull
models. Semantic **search** works fully offline without this:

```bash
curl -fsSL https://ollama.com/install.sh | sh
ollama pull llama3.2           # chat / agent planning
ollama pull nomic-embed-text   # only if you switch on LLM embeddings
```

Build and install the CLI + MCP server (need `libdbus-1-dev pkg-config`):

```bash
cargo build --release --manifest-path open-notebook/notion-cli/Cargo.toml
cargo build --release --manifest-path open-notebook/notion-mcp/Cargo.toml
cp open-notebook/notion-cli/target/release/notion-cli ~/.local/bin/
cp open-notebook/notion-mcp/target/release/notion-mcp ~/.local/bin/
```

- [ ] Launch the app with `ENABLE_OPEN_NOTEBOOK=1`.
- [ ] _(Generative only)_ Install Ollama and pull `llama3.2`.
- [ ] Try the ✨ floating button / `/ai` in the editor ("add a calendar event for
      tomorrow at 3pm"), the AI Studio drawer (search / ingest / activity), and
      drag-and-drop a `.txt`/`.md` file onto the window.
- [ ] Build + install `notion-cli` and `notion-mcp` (commands above).
- [ ] Use the CLI (unlock the app once first so the key is in the keyring, or set
      `NOTION_SQLCIPHER_KEY_HEX`): `notion-cli search "..."`,
      `notion-cli ingest @notes.txt`, `notion-cli ask "..."`.
- [ ] Wire an MCP client (Claude Desktop / Cursor): run `notion-mcp` (listens on
      `http://127.0.0.1:8787`) and point the client there. It exposes `search_notes`,
      `create_page`, `add_event`, and binds loopback only — do not expose the port
      off-host.
- [ ] _(Optional, real PDFs)_ Text/`.md` ingest works today; a PDF/URL/audio extractor
      is the documented plug-in point (`SourceExtractor`) if you want it.

---

## 4. After the merge — verify `main`

- [ ] Watch CI on `main` go green (Rust core, frontend, desktop, 3 companion jobs,
      2 Open Notebook jobs).
- [ ] Reproduce the build once from a clean clone to confirm your machine has all the
      OS packages: `pnpm install && cargo test && pnpm -r test`.
- [ ] Decide whether to keep or delete the old feature branches — their history is now
      fully contained in `main`.
- [ ] Ask Claude to open a PR only if you want a formal review record; the code is
      already on `main`.
