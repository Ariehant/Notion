/**
 * Typed Tauri IPC bridge to the Rust `notion_core` engine.
 *
 * This is the ONLY place the WebView talks to Rust. All key material and crypto
 * stay in Rust (audit §2.6); the WebView only ever sees decrypted *content*,
 * sanitized HTML, and opaque update bytes — never keys.
 */
import { invoke } from "@tauri-apps/api/core";
import type { PersistSink } from "./crdt/persistence";
import type { SanitizerBridge } from "./sanitize";

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

/** Sanitizer + embed surface backed by Rust `ammonia` (§2.8). */
export const tauriSanitizer: SanitizerBridge = {
  sanitizeHtml: (dirty) => invoke<string>("sanitize_html", { dirty }),
  sandboxedEmbed: (src) => invoke<string>("sandboxed_embed", { src }),
};

/** Load stored updates for a document (replayed into the Yjs doc on open). */
export async function loadUpdates(docId: string): Promise<Uint8Array[]> {
  const raw = await invoke<number[][]>("load_updates", { docId });
  return raw.map((u) => Uint8Array.from(u));
}
