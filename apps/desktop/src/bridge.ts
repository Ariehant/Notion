/**
 * Typed Tauri IPC bridge to the Rust `notion_core` engine.
 *
 * This is the ONLY place the WebView talks to Rust. All key material and crypto
 * stay in Rust (audit §2.6); the WebView only ever sees decrypted *content*,
 * sanitized HTML, opaque update bytes, and — exactly once, at creation — the
 * recovery code. It never sees keys.
 */
import { invoke } from "@tauri-apps/api/core";
import type { PersistSink } from "./crdt/persistence";
import type { SanitizerBridge } from "./sanitize";

/** A page as returned by the Rust layer. */
export interface PageDto {
  id: string;
  title: string;
  createdAtMs: number;
  updatedAtMs: number;
}

// --- Vault lifecycle -------------------------------------------------------

export const vaultExists = (): Promise<boolean> => invoke<boolean>("vault_exists");
export const isUnlocked = (): Promise<boolean> => invoke<boolean>("is_unlocked");

/** Create a new vault; resolves with the one-time recovery code to show once. */
export const createVault = (password: string): Promise<string> =>
  invoke<string>("create_vault", { password });

export const unlockVault = (password: string): Promise<void> =>
  invoke("unlock_vault", { password });

export const recoverVault = (recoveryCode: string, newPassword: string): Promise<void> =>
  invoke("recover_vault", { recoveryCode, newPassword });

export const lockVault = (): Promise<void> => invoke("lock_vault");

// --- Pages -----------------------------------------------------------------

export const createPage = (id: string, title: string): Promise<PageDto> =>
  invoke<PageDto>("create_page", { id, title });

export const listPages = (): Promise<PageDto[]> => invoke<PageDto[]>("list_pages");

export const renamePage = (id: string, title: string): Promise<void> =>
  invoke("rename_page", { id, title });

export const deletePage = (id: string): Promise<void> => invoke("delete_page", { id });

export const indexPage = (pageId: string, title: string, body: string): Promise<void> =>
  invoke("index_page", { pageId, title, body });

export const searchPages = (query: string): Promise<string[]> =>
  invoke<string[]>("search_pages", { query });

// --- Document persistence --------------------------------------------------

/** Persist encoded Yjs updates through the Rust append-only log (§1.6). */
export const tauriPersistSink: PersistSink = {
  async flush(docId, updates) {
    // Updates are passed as arrays of bytes; Rust seals (AEAD) + stores them.
    await invoke("persist_updates", {
      docId,
      updates: updates.map((u) => Array.from(u)),
    });
  },
};

/** Load stored updates for a document (replayed into the Yjs doc on open). */
export async function loadUpdates(docId: string): Promise<Uint8Array[]> {
  const raw = await invoke<number[][]>("load_updates", { docId });
  return raw.map((u) => Uint8Array.from(u));
}

/** Take an explicit full-document restore point (§1.3); returns the snapshot id. */
export const takeSnapshot = (docId: string, label?: string): Promise<number> =>
  invoke<number>("take_snapshot", { docId, label: label ?? null });

// --- Untrusted HTML --------------------------------------------------------

/** Sanitizer + embed surface backed by Rust `ammonia` (§2.8). */
export const tauriSanitizer: SanitizerBridge = {
  sanitizeHtml: (dirty) => invoke<string>("sanitize_html", { dirty }),
  sandboxedEmbed: (src) => invoke<string>("sandboxed_embed", { src }),
};

// --- Open Notebook AI (gated by ENABLE_OPEN_NOTEBOOK) ----------------------
//
// These wrap the `open_notebook_core` engine running in the Tauri backend. Every
// call resolves against the SAME encrypted DB the editor uses; the WebView never
// sees the DB key. When the feature flag is unset, `notebookEnabled()` is false
// and the AI UI is hidden.

/** One semantic/keyword search hit. */
export interface SearchHit {
  sourceId: string;
  score: number;
  title: string;
}

/** An ingested source (PDF/URL/audio/text) as returned by Rust. */
export interface IngestedSource {
  id: string;
  sourceType: string;
  sourcePath: string | null;
  title: string;
  summary: string | null;
  processedAt: number;
}

/** The result of an agent action. */
export interface AgentOutcome {
  kind: string;
  message: string;
  createdId: string | null;
}

/** A transparency-log entry describing a past agent action. */
export interface AgentLog {
  id: string;
  agentType: string;
  prompt: string;
  actionTaken: string;
  blockAffected: string | null;
  timestamp: number;
}

export const notebookEnabled = (): Promise<boolean> => invoke<boolean>("notebook_enabled");

export const semanticSearch = (query: string, limit?: number): Promise<SearchHit[]> =>
  invoke<SearchHit[]>("semantic_search", { query, limit: limit ?? null });

/** Index a page's text into semantic memory (called alongside `indexPage`). */
export const reindexPage = (pageId: string, title: string, body: string): Promise<void> =>
  invoke("reindex_page", { pageId, title, body });

export const ingestText = (text: string): Promise<IngestedSource> =>
  invoke<IngestedSource>("ingest_text", { text });

export const listSources = (): Promise<IngestedSource[]> =>
  invoke<IngestedSource[]>("list_sources");

export const runAgent = (prompt: string, blockId?: string): Promise<AgentOutcome> =>
  invoke<AgentOutcome>("run_agent", { prompt, blockId: blockId ?? null });

export const studioSummarize = (text: string): Promise<string> =>
  invoke<string>("studio_summarize", { text });

export const studioTransform = (text: string, instruction: string): Promise<string> =>
  invoke<string>("studio_transform", { text, instruction });

export const listAgentLogs = (limit?: number): Promise<AgentLog[]> =>
  invoke<AgentLog[]>("list_agent_logs", { limit: limit ?? null });
