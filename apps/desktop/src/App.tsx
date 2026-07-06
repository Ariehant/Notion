import { useCallback, useEffect, useRef, useState } from "react";
import { createPage, deletePage, listPages, lockVault, renamePage, type PageDto } from "./bridge";
import { genId } from "./crdt/blocks";
import { usePageDoc } from "./crdt/usePageDoc";
import { Editor } from "./components/Editor";
import { Sidebar } from "./components/Sidebar";
import { VaultGate } from "./components/VaultGate";

/**
 * App shell. Below the vault gate it wires the corrected architecture end to
 * end: pages in the encrypted DB, each page body a Yjs doc with async batched
 * persistence (§1.6), scheduled snapshots (§1.3), and one HTML sanitizer path.
 */
export function App() {
  const [unlocked, setUnlocked] = useState(false);
  const [pages, setPages] = useState<PageDto[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const { doc, ready } = usePageDoc(activeId);
  // A pending title rename is keyed by page id so a fast page switch flushes the
  // outgoing page's edit instead of firing it against the newly-selected page.
  const pendingRename = useRef<{ id: string; title: string } | null>(null);
  const renameTimer = useRef<ReturnType<typeof setTimeout>>();

  const flushRename = useCallback(() => {
    clearTimeout(renameTimer.current);
    const pending = pendingRename.current;
    if (!pending) return;
    pendingRename.current = null;
    void renamePage(pending.id, pending.title).catch((err) => console.error(err));
  }, []);

  const refreshPages = useCallback(async (): Promise<PageDto[]> => {
    const list = await listPages();
    setPages(list);
    return list;
  }, []);

  // On unlock, load pages and select (or create) a first page.
  useEffect(() => {
    if (!unlocked) return;
    void (async () => {
      let list = await refreshPages();
      if (list.length === 0) {
        const page = await createPage(genId(), "Untitled");
        list = [page];
        setPages(list);
      }
      setActiveId((cur) => cur ?? list[0].id);
    })().catch((err) => console.error(err));
  }, [unlocked, refreshPages]);

  const activePage = pages.find((p) => p.id === activeId) ?? null;

  const onCreate = useCallback(async () => {
    try {
      const page = await createPage(genId(), "Untitled");
      setPages((prev) => [page, ...prev]);
      setActiveId(page.id);
    } catch (err) {
      console.error(err);
    }
  }, []);

  const onDelete = useCallback(async (id: string) => {
    try {
      await deletePage(id);
      const list = await listPages();
      setPages(list);
      setActiveId((cur) => (cur === id ? (list[0]?.id ?? null) : cur));
    } catch (err) {
      console.error(err);
    }
  }, []);

  const onTitleChange = useCallback(
    (title: string) => {
      if (!activeId) return;
      setPages((prev) => prev.map((p) => (p.id === activeId ? { ...p, title } : p)));
      pendingRename.current = { id: activeId, title };
      clearTimeout(renameTimer.current);
      renameTimer.current = setTimeout(flushRename, 400);
    },
    [activeId, flushRename],
  );

  // Flush a pending rename before switching pages or unmounting.
  useEffect(() => {
    return () => flushRename();
  }, [activeId, flushRename]);

  const onLock = useCallback(async () => {
    flushRename();
    try {
      await lockVault();
    } catch (err) {
      console.error(err);
    }
    setUnlocked(false);
    setPages([]);
    setActiveId(null);
  }, [flushRename]);

  if (!unlocked) {
    return <VaultGate onUnlocked={() => setUnlocked(true)} />;
  }

  return (
    <div className="workspace">
      <Sidebar
        pages={pages}
        activeId={activeId}
        onSelect={setActiveId}
        onCreate={onCreate}
        onDelete={onDelete}
        onLock={onLock}
      />
      <main className="main-pane">
        {activePage && doc && ready ? (
          <Editor
            key={activePage.id}
            doc={doc}
            pageId={activePage.id}
            title={activePage.title}
            onTitleChange={onTitleChange}
          />
        ) : (
          <div className="empty-main">{activeId ? "Loading…" : "Select or create a page."}</div>
        )}
      </main>
    </div>
  );
}
