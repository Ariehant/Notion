/**
 * All marketing copy as typed data, so the sections stay presentational and the
 * content is unit-testable. Sourced from the repo's README / docs
 * (docs/OPEN_NOTEBOOK.md, companion/README.md, docs/ARCHITECTURE.md).
 */
import type { IconName } from "../components/icons";

export interface Feature {
  title: string;
  body: string;
}

export interface Pillar {
  icon: IconName;
  eyebrow: string;
  title: string;
  summary: string;
  features: Feature[];
}

export interface TrustChip {
  icon: IconName;
  label: string;
}

export interface SecurityStep {
  label: string;
  detail: string;
}

export interface Download {
  os: string;
  icon: IconName;
  file: string;
  note: string;
}

export const trustChips: TrustChip[] = [
  { icon: "wifiOff", label: "Offline-first" },
  { icon: "lock", label: "End-to-end encryptable" },
  { icon: "sparkle", label: "Local AI — no cloud" },
  { icon: "github", label: "Open source" },
];

export const pillars: Pillar[] = [
  {
    icon: "lock",
    eyebrow: "Write",
    title: "An encrypted vault & block editor",
    summary:
      "A fast block editor whose every page lives inside an encrypted database on your device.",
    features: [
      {
        title: "Block editor, CRDT-backed",
        body: "Headings, lists, to-dos, quotes and code via a slash menu (/) and markdown shortcuts. Edits merge as Yjs CRDT ops and flush asynchronously, so typing never blocks on disk.",
      },
      {
        title: "Encrypted at rest",
        body: "Everything is stored in SQLite + SQLCipher — pages, search index, and version-history snapshots. Nothing is written in the clear.",
      },
      {
        title: "Full-text search & history",
        body: "Search across your vault (the FTS index lives inside the encrypted file) and restore any page from full-document snapshots.",
      },
      {
        title: "Never locked out",
        body: "A one-time printable recovery kit resets a forgotten password without losing data — the encryption key, not the password, is the root.",
      },
    ],
  },
  {
    icon: "calendar",
    eyebrow: "See",
    title: "A native calendar companion",
    summary:
      "A tiny GNOME companion surfaces your agenda in the top bar — without spinning up the full app.",
    features: [
      {
        title: "Dynamic Island for GNOME",
        body: "Your next events, live in the top bar. A background daemon watches the shared database with the kernel (inotify) — no polling.",
      },
      {
        title: "~80–90% less memory",
        body: "Checking your week no longer means booting the whole WebView. Idle cost is a few MB of RAM and roughly 0% CPU.",
      },
      {
        title: "One encrypted database",
        body: "The companion reads the same encrypted file the app writes — no data duplication. It gets only the derived database key, never the root key.",
      },
      {
        title: "Quick-add with local AI",
        body: "A GTK quick-view lets you add events — including “Ask AI ✨” natural-language entry powered by a local model.",
      },
    ],
  },
  {
    icon: "sparkle",
    eyebrow: "Think",
    title: "Local AI, on your terms",
    summary:
      "Optional AI runs against a local model — semantic search, ingestion, and an action agent. Nothing leaves your machine.",
    features: [
      {
        title: "Semantic + keyword search",
        body: "Hybrid memory search finds notes by meaning and by exact keyword. It works fully offline with a built-in embedder — no model required.",
      },
      {
        title: "Ingest & summarize",
        body: "Drop in text or files to add them to your knowledge base, and summarize or rewrite with a local LLM (via Ollama).",
      },
      {
        title: "An action agent",
        body: "Say “add a calendar event for tomorrow at 3pm” and it becomes a real event the companion shows instantly. Every action is logged for transparency.",
      },
      {
        title: "CLI & MCP server",
        body: "A terminal client and a localhost MCP server let external tools like Claude Desktop and Cursor search and edit your notes — bound to loopback only.",
      },
    ],
  },
];

export const securitySteps: SecurityStep[] = [
  {
    label: "Password → Argon2id",
    detail: "Your password is stretched with Argon2id (128 MiB, t=3) — never stored.",
  },
  {
    label: "HKDF → DEK",
    detail: "HKDF-SHA256 derives subkeys that wrap a random 256-bit Data Encryption Key.",
  },
  {
    label: "DEK → database key",
    detail: "The DEK — not your password — is the root of all content encryption.",
  },
  {
    label: "XChaCha20-Poly1305",
    detail: "Content is sealed with authenticated encryption; keys never touch the WebView.",
  },
];

export const securityPoints: Feature[] = [
  {
    title: "The key, not the password, is the root",
    body: "Changing your password (or using the recovery kit) just re-wraps the key — the database is never re-encrypted, and you never lose data.",
  },
  {
    title: "Hardened by default",
    body: "SQLCipher is linked directly (temp store in memory, secure delete on). Pasted and scraped HTML goes through one sanitizer; web capture is SSRF-guarded.",
  },
  {
    title: "Keys stay in native code",
    body: "All key material is zeroized after use and never crosses into JavaScript. A strict Content-Security-Policy and a minimal capability set lock the shell down.",
  },
];

export const downloads: Download[] = [
  { os: "Windows", icon: "windows", file: "Notion_x64-setup.exe", note: "Double-click installer" },
  { os: "macOS", icon: "apple", file: "Notion.dmg", note: "Universal disk image" },
  {
    os: "Linux",
    icon: "linux",
    file: "Notion.AppImage",
    note: "Portable — chmod +x and run (or .deb)",
  },
];
