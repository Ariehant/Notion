import { useCallback, useEffect, useRef, useState } from "react";
import {
  createPage,
  deletePage,
  ingestText,
  listPages,
  lockVault,
  notebookEnabled,
  renamePage,
  type PageDto,
} from "./bridge";
import { genId } from "./crdt/blocks";
import { planDrop } from "./ai/actions";
import { usePageDoc } from "./crdt/usePageDoc";
import { AiDialog } from "./components/AiDialog";
import { AIStudio } from "./components/AIStudio";
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
  // Open Notebook AI (only when the backend reports the flag is on).
  const [aiEnabled, setAiEnabled] = useState(false);
  const [studioOpen, setStudioOpen] = useState(false);
  const [aiDialog, setAiDialog] = useState<{ open: boolean; prompt: string; blockId?: string }>({
    open: false,
    prompt: "",
  });
  const [dropMsg, setDropMsg] = useState<string | null>(null);
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

  // On unlock, load pages and select (or create) a first page, and learn whether
  // the Open Notebook AI features are enabled in the backend.
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
    void notebookEnabled()
      .then(setAiEnabled)
      .catch(() => setAiEnabled(false));
  }, [unlocked, refreshPages]);

  // Flash a transient drop message, then clear it.
  useEffect(() => {
    if (!dropMsg) return;
    const t = setTimeout(() => setDropMsg(null), 3500);
    return () => clearTimeout(t);
  }, [dropMsg]);

  const onAskAI = useCallback((blockId: string, blockText: string) => {
    setAiDialog({ open: true, prompt: blockText, blockId });
  }, []);

  // Drag-and-drop ingestion: text and .txt/.md files go into the knowledge base.
  const onDrop = useCallback(
    async (e: React.DragEvent) => {
      if (!aiEnabled) return;
      e.preventDefault();
      const text = e.dataTransfer.getData("text/plain");
      const files = Array.from(e.dataTransfer.files);
      const plan = planDrop(text, files);
      try {
        if (plan.action === "ingest-text") {
          await ingestText(plan.text);
          setDropMsg("Ingested dropped text ✨");
        } else if (plan.action === "ingest-files") {
          for (const file of plan.files) {
            await ingestText(await file.text());
          }
          setDropMsg(`Ingested ${plan.files.length} file(s) ✨`);
        } else {
          setDropMsg(plan.reason);
        }
      } catch (err) {
        console.error(err);
        setDropMsg("Ingestion failed.");
      }
    },
    [aiEnabled],
  );

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
    setStudioOpen(false);
    setAiDialog({ open: false, prompt: "" });
  }, [flushRename]);

  if (!unlocked) {
    return <VaultGate onUnlocked={() => setUnlocked(true)} />;
  }

  return (
    <div
      className="workspace"
      onDragOver={aiEnabled ? (e) => e.preventDefault() : undefined}
      onDrop={aiEnabled ? (e) => void onDrop(e) : undefined}
    >
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
            aiEnabled={aiEnabled}
            onAskAI={onAskAI}
          />
        ) : (
          <div className="empty-main">{activeId ? "Loading…" : "Select or create a page."}</div>
        )}
      </main>

      {aiEnabled && (
        <>
          {studioOpen && (
            <AIStudio
              open={studioOpen}
              onClose={() => setStudioOpen(false)}
              onOpenSource={(id) => {
                // If the hit is a known page, open it.
                if (pages.some((p) => p.id === id)) setActiveId(id);
              }}
            />
          )}
          <button
            type="button"
            className="ai-fab"
            title="Ask AI ✨"
            onClick={() => setAiDialog({ open: true, prompt: "" })}
          >
            ✨
          </button>
          <button
            type="button"
            className="ai-studio-toggle"
            title="Open AI Studio"
            onClick={() => setStudioOpen((v) => !v)}
          >
            AI
          </button>
          <AiDialog
            open={aiDialog.open}
            initialPrompt={aiDialog.prompt}
            blockId={aiDialog.blockId}
            onClose={() => setAiDialog({ open: false, prompt: "" })}
            onDone={() => void refreshPages()}
          />
          {dropMsg && <div className="ai-toast">{dropMsg}</div>}
        </>
      )}
    </div>
  );
}
