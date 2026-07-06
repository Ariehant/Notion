import { useEffect, useMemo, useState } from "react";
import type { PageDto } from "../bridge";
import { searchPages } from "../bridge";

interface SidebarProps {
  pages: PageDto[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onCreate: () => void;
  onDelete: (id: string) => void;
  onLock: () => void;
}

export function Sidebar({ pages, activeId, onSelect, onCreate, onDelete, onLock }: SidebarProps) {
  const [query, setQuery] = useState("");
  const [matchIds, setMatchIds] = useState<string[] | null>(null);

  // Debounced full-text search over the encrypted FTS index (§1.8).
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setMatchIds(null);
      return;
    }
    const timer = setTimeout(() => {
      void searchPages(q)
        .then(setMatchIds)
        .catch((err) => console.error("search failed", err));
    }, 200);
    return () => clearTimeout(timer);
  }, [query]);

  const visible = useMemo(() => {
    if (!matchIds) return pages;
    const rank = new Map(matchIds.map((id, i) => [id, i]));
    return pages.filter((p) => rank.has(p.id)).sort((a, b) => rank.get(a.id)! - rank.get(b.id)!);
  }, [pages, matchIds]);

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <span className="brand">Notion</span>
        <button type="button" className="lock" title="Lock vault" onClick={onLock}>
          Lock
        </button>
      </div>

      <input
        className="search"
        placeholder="Search…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />

      <button type="button" className="new-page" onClick={onCreate}>
        + New page
      </button>

      <nav className="page-list">
        {visible.length === 0 && <p className="empty">No pages.</p>}
        {visible.map((p) => (
          <div
            key={p.id}
            className={`page-item${p.id === activeId ? " active" : ""}`}
            onClick={() => onSelect(p.id)}
          >
            <span className="page-title-text">{p.title || "Untitled"}</span>
            <button
              type="button"
              className="delete"
              title="Delete page"
              onClick={(e) => {
                e.stopPropagation();
                onDelete(p.id);
              }}
            >
              ×
            </button>
          </div>
        ))}
      </nav>
    </aside>
  );
}
