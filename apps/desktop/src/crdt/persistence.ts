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
 *
 * Hardened after review:
 *   - A failed flush no longer poisons persistence: the batch is re-queued (in
 *     order) and retried with backoff, instead of being silently dropped and
 *     killing all future writes (was a silent data-loss bug).
 *   - Memory is genuinely bounded: when the buffer grows past `maxBytes` the
 *     pending updates are **coalesced** with `Y.mergeUpdates` into a single
 *     update, so a slow/stalled sink cannot grow memory without limit.
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
  /** Coalesce + flush once pending bytes reach this. Default 256 KiB. */
  maxBytes?: number;
  /** Base delay (ms) before retrying after a failed flush. Default 500. */
  retryBaseMs?: number;
  /** Called when a flush fails (after the batch has been safely re-queued). */
  onError?: (err: unknown) => void;
}

const DEFAULTS: Required<Omit<BatchedPersistenceOptions, "onError">> = {
  debounceMs: 250,
  maxBatch: 64,
  maxBytes: 256 * 1024,
  retryBaseMs: 500,
};

/**
 * Subscribes to a Y.Doc and persists its updates in the background.
 *
 * Updates that originate from this provider (i.e. replayed from storage) are
 * not re-persisted, preventing feedback loops.
 */
export class BatchedPersistence {
  private readonly opts: Required<Omit<BatchedPersistenceOptions, "onError">>;
  private readonly onError?: (err: unknown) => void;
  private pending: Uint8Array[] = [];
  private pendingBytes = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private draining: Promise<void> | null = null;
  private destroyed = false;
  private readonly onUpdate: (update: Uint8Array, origin: unknown) => void;

  constructor(
    private readonly docId: string,
    private readonly doc: Y.Doc,
    private readonly sink: PersistSink,
    options: BatchedPersistenceOptions = {},
  ) {
    const { onError, ...rest } = options;
    this.opts = { ...DEFAULTS, ...rest };
    this.onError = onError;
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

  /** Number of updates currently buffered (not yet handed to the sink). */
  get pendingCount(): number {
    return this.pending.length;
  }

  private byteLen(list: Uint8Array[]): number {
    return list.reduce((n, u) => n + u.byteLength, 0);
  }

  /** Merge the pending buffer into a single update to bound memory. */
  private coalesce(): void {
    if (this.pending.length <= 1) return;
    const merged = Y.mergeUpdates(this.pending);
    this.pending = [merged];
    this.pendingBytes = merged.byteLength;
  }

  private enqueue(update: Uint8Array): void {
    if (this.destroyed) return;
    this.pending.push(update);
    this.pendingBytes += update.byteLength;

    if (this.pendingBytes >= this.opts.maxBytes) {
      // Bound memory before draining (real backpressure, not just relocation).
      this.coalesce();
      void this.drain();
      return;
    }
    if (this.pending.length >= this.opts.maxBatch) {
      void this.drain();
      return;
    }
    // Otherwise debounce: reset the idle timer.
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.drain(), this.opts.debounceMs);
  }

  /** Force an immediate flush of everything buffered. Resolves when written. */
  flushNow(): Promise<void> {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    return this.drain();
  }

  /**
   * Single-flight drain loop. Sends batches in order; on failure the batch is
   * put back at the FRONT of `pending` (preserving order) and a retry is
   * scheduled — the failure never poisons future flushes.
   */
  private drain(): Promise<void> {
    if (!this.draining) {
      this.draining = this.runDrain().finally(() => {
        this.draining = null;
      });
    }
    return this.draining;
  }

  private async runDrain(): Promise<void> {
    while (this.pending.length > 0) {
      if (this.timer !== null) {
        clearTimeout(this.timer);
        this.timer = null;
      }
      const batch = this.pending;
      this.pending = [];
      this.pendingBytes = 0;
      try {
        await this.sink.flush(this.docId, batch);
      } catch (err) {
        // Re-queue at the front so nothing is lost and ordering is preserved.
        this.pending = batch.concat(this.pending);
        this.pendingBytes = this.byteLen(this.pending);
        this.onError?.(err);
        this.scheduleRetry();
        return;
      }
    }
  }

  private scheduleRetry(): void {
    if (this.destroyed || this.retryTimer !== null) return;
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      void this.drain();
    }, this.opts.retryBaseMs);
  }

  /** Unsubscribe and flush any remaining updates (best-effort). */
  async destroy(): Promise<void> {
    if (this.destroyed) return;
    this.destroyed = true;
    this.doc.off("update", this.onUpdate);
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
    await this.drain();
  }
}
