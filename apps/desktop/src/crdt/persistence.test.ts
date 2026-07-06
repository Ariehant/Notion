import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as Y from "yjs";
import { BatchedPersistence, type PersistSink } from "./persistence";

class RecordingSink implements PersistSink {
  flushes: { docId: string; updates: Uint8Array[] }[] = [];
  flush(docId: string, updates: Uint8Array[]): Promise<void> {
    this.flushes.push({ docId, updates });
    return Promise.resolve();
  }
  get totalUpdates(): number {
    return this.flushes.reduce((n, f) => n + f.updates.length, 0);
  }
}

describe("BatchedPersistence (§1.6 async batched persistence)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("does NOT flush synchronously on edit — the edit path never blocks", () => {
    const doc = new Y.Doc();
    const sink = new RecordingSink();
    new BatchedPersistence("d1", doc, sink, { debounceMs: 250 });

    doc.getText("t").insert(0, "hello");
    // Immediately after the edit, nothing has been written to disk yet.
    expect(sink.flushes.length).toBe(0);
  });

  it("flushes once after the debounce window", async () => {
    const doc = new Y.Doc();
    const sink = new RecordingSink();
    new BatchedPersistence("d1", doc, sink, { debounceMs: 250 });

    doc.getText("t").insert(0, "a");
    doc.getText("t").insert(1, "b");
    expect(sink.flushes.length).toBe(0);

    await vi.advanceTimersByTimeAsync(250);
    expect(sink.flushes.length).toBe(1);
    expect(sink.flushes[0].docId).toBe("d1");
    expect(sink.totalUpdates).toBe(2); // both edits batched into one flush
  });

  it("flushes immediately when the batch-size threshold is hit", async () => {
    const doc = new Y.Doc();
    const sink = new RecordingSink();
    new BatchedPersistence("d1", doc, sink, { debounceMs: 10_000, maxBatch: 3 });

    const t = doc.getText("t");
    t.insert(0, "1");
    t.insert(1, "2");
    expect(sink.flushes.length).toBe(0);
    t.insert(2, "3"); // 3rd pending update → immediate flush
    await vi.advanceTimersByTimeAsync(0);
    expect(sink.flushes.length).toBe(1);
    expect(sink.totalUpdates).toBe(3);
  });

  it("does not re-persist updates replayed from storage", async () => {
    // Produce a stored update from a source doc.
    const source = new Y.Doc();
    source.getText("t").insert(0, "restored");
    const stored = Y.encodeStateAsUpdate(source);

    const doc = new Y.Doc();
    const sink = new RecordingSink();
    const p = new BatchedPersistence("d1", doc, sink, { debounceMs: 50 });

    p.applyStoredUpdates([stored]);
    await vi.advanceTimersByTimeAsync(50);

    // Replayed content is in the doc but was NOT written back (no loop).
    expect(doc.getText("t").toString()).toBe("restored");
    expect(sink.flushes.length).toBe(0);
  });

  it("flushes remaining updates on destroy", async () => {
    const doc = new Y.Doc();
    const sink = new RecordingSink();
    const p = new BatchedPersistence("d1", doc, sink, { debounceMs: 10_000 });

    doc.getText("t").insert(0, "x");
    await p.destroy();
    expect(sink.totalUpdates).toBe(1);
  });

  it("stored updates round-trip the document state", async () => {
    // Persist edits, then rebuild a fresh doc from the captured updates.
    const doc = new Y.Doc();
    const sink = new RecordingSink();
    const p = new BatchedPersistence("d1", doc, sink, { debounceMs: 5 });
    doc.getText("t").insert(0, "round trip");
    await vi.advanceTimersByTimeAsync(5);

    const rebuilt = new Y.Doc();
    for (const f of sink.flushes) for (const u of f.updates) Y.applyUpdate(rebuilt, u);
    expect(rebuilt.getText("t").toString()).toBe("round trip");
    await p.destroy();
  });

  it("re-queues and retries after a failed flush — no data loss, no poisoning", async () => {
    // Reviewer HIGH finding: a single rejected flush must not silently drop the
    // batch or kill all future writes.
    const doc = new Y.Doc();
    let failuresLeft = 1;
    const errors: unknown[] = [];
    const good: Uint8Array[][] = [];
    const sink: PersistSink = {
      flush(_id, updates) {
        if (failuresLeft > 0) {
          failuresLeft--;
          return Promise.reject(new Error("transient disk error"));
        }
        good.push(updates);
        return Promise.resolve();
      },
    };
    const p = new BatchedPersistence("d1", doc, sink, {
      debounceMs: 50,
      retryBaseMs: 500,
      onError: (e) => errors.push(e),
    });

    doc.getText("t").insert(0, "important");
    await vi.advanceTimersByTimeAsync(50); // first attempt fails
    expect(errors.length).toBe(1);
    expect(good.length).toBe(0);

    await vi.advanceTimersByTimeAsync(500); // scheduled retry succeeds
    expect(good.length).toBe(1);

    // The retried batch still carries the original edit — nothing lost.
    const rebuilt = new Y.Doc();
    for (const u of good[0]) Y.applyUpdate(rebuilt, u);
    expect(rebuilt.getText("t").toString()).toBe("important");
    await p.destroy();
  });

  it("bounds memory by coalescing when the sink stalls", async () => {
    // Reviewer MEDIUM finding: a slow/stalled sink must not let buffered bytes
    // grow without limit. With maxBytes tiny, every edit coalesces the buffer.
    const doc = new Y.Doc();
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    let calls = 0;
    const sink: PersistSink = {
      flush() {
        calls++;
        return gate; // never resolves until released → flush is "in flight"
      },
    };
    const p = new BatchedPersistence("d1", doc, sink, {
      debounceMs: 1000,
      maxBytes: 1,
      maxBatch: 1_000_000,
    });

    const t = doc.getText("t");
    for (let i = 0; i < 200; i++) t.insert(i, "x");
    await vi.advanceTimersByTimeAsync(0);

    // 200 edits, but the pending buffer stays coalesced to a single update and
    // only one flush was ever started (the rest returned the in-flight promise).
    expect(p.pendingCount).toBeLessThanOrEqual(1);
    expect(calls).toBe(1);

    release();
    await vi.advanceTimersByTimeAsync(1000);
    await p.destroy();
  });
});
