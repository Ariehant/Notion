import { describe, it, expect } from "vitest";
import { SnapshotScheduler } from "./scheduler";

describe("SnapshotScheduler (§1.3 explicit restore points)", () => {
  const policy = { everyNUpdates: 5, everyMs: 1000 };

  it("is not due when the doc is idle (no updates)", () => {
    const s = new SnapshotScheduler(policy, 0);
    expect(s.isDue(1_000_000)).toBe(false); // lots of time, but zero edits
  });

  it("is due after the update threshold", () => {
    const s = new SnapshotScheduler(policy, 0);
    s.recordUpdates(4);
    expect(s.isDue(10)).toBe(false);
    s.recordUpdates(1); // now 5
    expect(s.isDue(10)).toBe(true);
  });

  it("is due after the time threshold if any edit happened", () => {
    const s = new SnapshotScheduler(policy, 0);
    s.recordUpdates(1);
    expect(s.isDue(999)).toBe(false);
    expect(s.isDue(1000)).toBe(true);
  });

  it("resets counters after a snapshot is taken", () => {
    const s = new SnapshotScheduler(policy, 0);
    s.recordUpdates(5);
    expect(s.isDue(10)).toBe(true);
    s.markTaken(10);
    expect(s.pendingUpdates).toBe(0);
    expect(s.isDue(500)).toBe(false);
    // Clock threshold now measured from the last snapshot time.
    s.recordUpdates(1);
    expect(s.isDue(1010)).toBe(true);
  });
});
