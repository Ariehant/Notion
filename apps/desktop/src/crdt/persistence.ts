import * as Y from "yjs";

/**
 * Async, batched Yjs → durable-store persistence — audit §1.6.
 *
 * The blueprint said each change is "synchronously written to SQLite", which
 * would jank the editor. Instead the in-memory Yjs doc is the fast path; encoded
 * updates are **buffered and flushed asynchronously** (on idle, or when a size
 * threshold is hit) to the Rust/SQLite layer. The editor edit path never awaits
 * disk.
 *
 * SQLite is the single source of truth (audit §1.5) — there is deliberately no
 * `y-indexeddb` layer duplicating these writes.
 */

/** Durable sink for encoded Yjs updates (implemented over Tauri IPC → Rust). */
export interface PersistSink {
  /** Append a batch of encoded v1 updates for `docId`. Must be idempotent-safe. */
  flush(docId: string, updates: Uint8Array[]): Promise<void>;
}

export interface BatchedPersistenceOptions {
  /** Idle window (ms) with no edits before a flush fires. Default 250. */
  debounceMs?: number;
  /** Flush immediately once this many updates are pending. Default 64. */
  maxBatch?: number;
  /** Flush immediately once pending bytes exceed this. Default 256 KiB. */
  maxBytes?: number;
}

const DEFAULTS: Required<BatchedPersistenceOptions> = {
  debounceMs: 250,
  maxBatch: 64,
  maxBytes: 256 * 1024,
};

/**
 * Subscribes to a Y.Doc and persists its updates in the background.
 *
 * Updates that originate from this provider (i.e. replayed from storage) are
 * not re-persisted, preventing feedback loops.
 */
export class BatchedPersistence {
  private readonly opts: Required<BatchedPersistenceOptions>;
  private pending: Uint8Array[] = [];
  private pendingBytes = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private flushing: Promise<void> = Promise.resolve();
  private destroyed = false;
  private readonly onUpdate: (update: Uint8Array, origin: unknown) => void;

  constructor(
    private readonly docId: string,
    private readonly doc: Y.Doc,
    private readonly sink: PersistSink,
    options: BatchedPersistenceOptions = {},
  ) {
    this.opts = { ...DEFAULTS, ...options };
    this.onUpdate = (update, origin) => {
      // Don't re-persist updates we ourselves replayed from storage.
      if (origin === this) return;
      this.enqueue(update);
    };
    this.doc.on("update", this.onUpdate);
  }

  /** Replay stored updates into the doc without triggering re-persistence. */
  applyStoredUpdates(updates: Uint8Array[]): void {
    for (const u of updates) {
      Y.applyUpdate(this.doc, u, this); // origin === this ⇒ skipped by onUpdate
    }
  }

  /** Number of updates currently buffered (not yet flushed). */
  get pendingCount(): number {
    return this.pending.length;
  }

  private enqueue(update: Uint8Array): void {
    if (this.destroyed) return;
    this.pending.push(update);
    this.pendingBytes += update.byteLength;

    // Size-based immediate flush keeps memory bounded under heavy edits.
    if (this.pending.length >= this.opts.maxBatch || this.pendingBytes >= this.opts.maxBytes) {
      void this.flushNow();
      return;
    }
    // Otherwise debounce: reset the idle timer.
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.flushNow(), this.opts.debounceMs);
  }

  /** Force an immediate flush of everything buffered. Returns when written. */
  async flushNow(): Promise<void> {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    if (this.pending.length === 0) return this.flushing;

    const batch = this.pending;
    this.pending = [];
    this.pendingBytes = 0;

    // Serialize flushes so appends stay ordered even if called concurrently.
    this.flushing = this.flushing.then(() => this.sink.flush(this.docId, batch));
    return this.flushing;
  }

  /** Unsubscribe and flush any remaining updates. */
  async destroy(): Promise<void> {
    if (this.destroyed) return;
    this.destroyed = true;
    this.doc.off("update", this.onUpdate);
    await this.flushNow();
  }
}
