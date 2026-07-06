import { useEffect, useState } from "react";
import * as Y from "yjs";
import { BatchedPersistence } from "./crdt/persistence";
import { SnapshotScheduler } from "./snapshots/scheduler";
import { starredGapBlocks } from "./blocks/schema";

/**
 * Minimal app shell. The real editor (BlockNote + custom blocks) lands in
 * Phase 1; this wires the corrected architecture end to end: a Yjs doc as the
 * fast path with async batched persistence (§1.6) and scheduled snapshots
 * (§1.3). The heavy WebKitGTK build is documented in docs/ARCHITECTURE.md.
 */
export function App() {
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const doc = new Y.Doc();
    // In the running app the sink is `tauriPersistSink` from ./bridge; here we
    // just prove the wiring compiles and initializes.
    const sink = { flush: async () => {} };
    const persistence = new BatchedPersistence("welcome", doc, sink);
    const scheduler = new SnapshotScheduler();
    scheduler.recordUpdates(0);
    setReady(true);
    return () => void persistence.destroy();
  }, []);

  return (
    <main style={{ fontFamily: "system-ui", padding: "2rem" }}>
      <h1>Offline-first Notion</h1>
      <p>Status: {ready ? "core initialized" : "loading…"}</p>
      <p>Custom blocks queued for Phase 1:</p>
      <ul>
        {starredGapBlocks().map((b) => (
          <li key={b.type}>{b.label}</li>
        ))}
      </ul>
    </main>
  );
}
