/**
 * Untrusted-HTML ingestion — audit §2.8.
 *
 * The blueprint sanitized *scraped* HTML but left **pasted** rich text as an
 * equal, unguarded XSS vector. Here the WebView side does NOT reimplement
 * sanitization: it routes **all** untrusted HTML — pasted or scraped — through
 * the single audited Rust sanitizer (`sanitize_html`, ammonia) via Tauri IPC.
 * One sanitizer, one policy, both sources.
 *
 * Embeds never render as arbitrary iframes; they go through the Rust
 * `sandboxed_embed` command, which enforces the sandbox policy (never
 * `allow-scripts` + `allow-same-origin`) and an https-only, SSRF-guarded src.
 */

/** The Rust-backed sanitizer surface (implemented over Tauri `invoke`). */
export interface SanitizerBridge {
  sanitizeHtml(dirty: string): Promise<string>;
  sandboxedEmbed(src: string): Promise<string>;
}

/**
 * The ONLY sanctioned way to turn untrusted HTML into editor content.
 * Callers must never insert raw clipboard/scraped HTML directly.
 */
export async function ingestUntrustedHtml(dirty: string, bridge: SanitizerBridge): Promise<string> {
  return bridge.sanitizeHtml(dirty);
}

/**
 * Build a paste handler that always sanitizes before inserting a block.
 * `insertBlock` receives only sanitized HTML — raw HTML cannot reach it.
 */
export function createPasteHandler(
  bridge: SanitizerBridge,
  insertBlock: (safeHtml: string) => void,
) {
  return async function onPaste(event: {
    clipboardData: { getData(type: string): string } | null;
    preventDefault(): void;
  }): Promise<void> {
    const html = event.clipboardData?.getData("text/html") ?? "";
    if (!html) return; // plain-text paste handled elsewhere
    event.preventDefault();
    const safe = await ingestUntrustedHtml(html, bridge);
    insertBlock(safe);
  };
}

/** Turn a user-provided embed URL into a locked-down iframe (via Rust). */
export async function ingestEmbed(src: string, bridge: SanitizerBridge): Promise<string> {
  return bridge.sandboxedEmbed(src);
}
