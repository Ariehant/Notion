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
  const renameTimer = useRef<ReturnType<typeof setTimeout>>();

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
      clearTimeout(renameTimer.current);
      renameTimer.current = setTimeout(() => {
        void renamePage(activeId, title).catch((err) => console.error(err));
      }, 400);
    },
    [activeId],
  );

  const onLock = useCallback(async () => {
    try {
      await lockVault();
    } catch (err) {
      console.error(err);
    }
    setUnlocked(false);
    setPages([]);
    setActiveId(null);
  }, []);

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
