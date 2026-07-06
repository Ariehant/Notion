import { useEffect, useState } from "react";
import * as Y from "yjs";
import { BatchedPersistence } from "./persistence";
import { ensureNonEmpty } from "./blocks";
import { SnapshotScheduler, DEFAULT_SNAPSHOT_POLICY } from "../snapshots/scheduler";
import { loadUpdates, tauriPersistSink, takeSnapshot } from "../bridge";

/**
 * Own the Yjs document lifecycle for one page: replay its encrypted update log,
 * wire async batched persistence (§1.6), and drive version-history snapshots
 * (§1.3). Swapping `pageId` tears the previous doc down (flushing first).
 */
export function usePageDoc(pageId: string | null): { doc: Y.Doc | null; ready: boolean } {
  const [doc, setDoc] = useState<Y.Doc | null>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    if (!pageId) {
      setDoc(null);
      setReady(false);
      return;
    }

    let disposed = false;
    const nextDoc = new Y.Doc();
    const persistence = new BatchedPersistence(pageId, nextDoc, tauriPersistSink, {
      onError: (err) => console.error("persist failed", err),
    });
    const scheduler = new SnapshotScheduler(DEFAULT_SNAPSHOT_POLICY, Date.now());

    setReady(false);
    setDoc(null);

    void (async () => {
      try {
        const updates = await loadUpdates(pageId);
        if (disposed) return;
        persistence.applyStoredUpdates(updates);
      } catch (err) {
        console.error("load failed", err);
      }
      if (disposed) return;
      ensureNonEmpty(nextDoc);
      setDoc(nextDoc);
      setReady(true);
    })();

    // Count only genuine edits (replayed updates carry origin === persistence).
    const onUpdate = (_u: Uint8Array, origin: unknown) => {
      if (origin !== persistence) scheduler.recordUpdates(1);
    };
    nextDoc.on("update", onUpdate);

    const timer = setInterval(() => {
      const now = Date.now();
      if (scheduler.isDue(now)) {
        void takeSnapshot(pageId)
          .then(() => scheduler.markTaken(now))
          .catch((err) => console.error("snapshot failed", err));
      }
    }, 60_000);

    return () => {
      disposed = true;
      clearInterval(timer);
      nextDoc.off("update", onUpdate);
      void persistence.destroy();
    };
  }, [pageId]);

  return { doc, ready };
}
