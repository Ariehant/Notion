/**
 * Version-history snapshot scheduling — audit §1.3.
 *
 * Because Yjs runs with GC on, we cannot rebuild old versions from the CRDT's
 * native `snapshot()`. Instead we take explicit **full-document restore points**
 * on a schedule/threshold. This module decides *when*; the actual full-doc copy
 * is produced by the Rust `crdt::snapshot` and stored in `doc_snapshots`.
 *
 * A snapshot is due when EITHER:
 *   - at least `everyNUpdates` updates have accrued since the last snapshot, OR
 *   - at least `everyMs` wall-clock has elapsed since the last snapshot
 *     (and at least one update happened — we never snapshot an idle doc).
 *
 * Wall-clock (audit §1.2) is injected, never derived from the CRDT clock.
 */

export interface SnapshotPolicy {
  everyNUpdates: number;
  everyMs: number;
}

export const DEFAULT_SNAPSHOT_POLICY: SnapshotPolicy = {
  everyNUpdates: 200,
  everyMs: 30 * 60 * 1000, // 30 minutes
};

export class SnapshotScheduler {
  private updatesSinceSnapshot = 0;
  private lastSnapshotAtMs: number;

  constructor(
    private readonly policy: SnapshotPolicy = DEFAULT_SNAPSHOT_POLICY,
    nowMs = 0,
  ) {
    this.lastSnapshotAtMs = nowMs;
  }

  /** Record that `count` updates were persisted. */
  recordUpdates(count = 1): void {
    this.updatesSinceSnapshot += count;
  }

  /** Whether a snapshot should be taken at wall-clock `nowMs`. */
  isDue(nowMs: number): boolean {
    if (this.updatesSinceSnapshot === 0) return false;
    if (this.updatesSinceSnapshot >= this.policy.everyNUpdates) return true;
    return nowMs - this.lastSnapshotAtMs >= this.policy.everyMs;
  }

  /** Call after a snapshot has actually been taken to reset counters. */
  markTaken(nowMs: number): void {
    this.updatesSinceSnapshot = 0;
    this.lastSnapshotAtMs = nowMs;
  }

  get pendingUpdates(): number {
    return this.updatesSinceSnapshot;
  }
}
