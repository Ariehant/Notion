import { describe, it, expect, vi } from "vitest";
import {
  createPasteHandler,
  ingestUntrustedHtml,
  ingestEmbed,
  type SanitizerBridge,
} from "./index";

// A stand-in for the Rust sanitizer: records calls and returns a marked string
// so tests can prove the sanitized (not the raw) value flows onward.
function fakeBridge(): SanitizerBridge & { calls: string[] } {
  const calls: string[] = [];
  return {
    calls,
    async sanitizeHtml(dirty) {
      calls.push(dirty);
      return `SANITIZED(${dirty.replace(/<script>.*<\/script>/, "")})`;
    },
    async sandboxedEmbed(src) {
      return `<iframe src="${src}" sandbox="allow-scripts"></iframe>`;
    },
  };
}

describe("untrusted HTML ingestion (§2.8 one sanitizer for paste + scrape)", () => {
  it("routes ingest through the Rust sanitizer", async () => {
    const bridge = fakeBridge();
    const out = await ingestUntrustedHtml("<b>hi</b>", bridge);
    expect(bridge.calls).toEqual(["<b>hi</b>"]);
    expect(out).toContain("SANITIZED");
  });

  it("paste handler never passes raw HTML to insertBlock", async () => {
    const bridge = fakeBridge();
    const inserted: string[] = [];
    const preventDefault = vi.fn();
    const onPaste = createPasteHandler(bridge, (safe) => inserted.push(safe));

    const dirty = "<p>ok</p><script>alert(1)</script>";
    await onPaste({
      clipboardData: { getData: (t) => (t === "text/html" ? dirty : "") },
      preventDefault,
    });

    expect(preventDefault).toHaveBeenCalled();
    // The inserted content is the sanitizer's output, and the raw dirty string
    // was handed to the sanitizer — never inserted verbatim.
    expect(bridge.calls).toEqual([dirty]);
    expect(inserted).toHaveLength(1);
    expect(inserted[0]).toContain("SANITIZED");
    expect(inserted[0]).not.toContain("<script>");
  });

  it("ignores empty/plain-text pastes without inserting", async () => {
    const bridge = fakeBridge();
    const inserted: string[] = [];
    const onPaste = createPasteHandler(bridge, (safe) => inserted.push(safe));
    await onPaste({
      clipboardData: { getData: () => "" },
      preventDefault: () => {},
    });
    expect(inserted).toHaveLength(0);
    expect(bridge.calls).toHaveLength(0);
  });

  it("embeds go through the sandboxed-embed bridge", async () => {
    const bridge = fakeBridge();
    const out = await ingestEmbed("https://ok.example/e", bridge);
    expect(out).toContain("sandbox=");
  });
});
