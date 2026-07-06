import { useCallback, useEffect, useState } from "react";
import {
  ingestText,
  listAgentLogs,
  listSources,
  semanticSearch,
  type AgentLog,
  type IngestedSource,
  type SearchHit,
} from "../bridge";
import { describeSource, scorePercent } from "../ai/actions";

type Tab = "search" | "ingest" | "activity";

interface AIStudioProps {
  open: boolean;
  onClose: () => void;
  /** Jump to a page id when a search hit that is a page is clicked. */
  onOpenSource: (sourceId: string) => void;
}

/**
 * The AI Studio drawer: semantic search, quick text ingestion, and the agent
 * activity log. Each tab is a thin wrapper over a bridge call; the display
 * shaping (score %, source lines) is the tested `ai/actions` module.
 */
export function AIStudio({ open, onClose, onOpenSource }: AIStudioProps) {
  const [tab, setTab] = useState<Tab>("search");

  // Search
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [searching, setSearching] = useState(false);

  // Ingest
  const [draft, setDraft] = useState("");
  const [sources, setSources] = useState<IngestedSource[]>([]);
  const [ingesting, setIngesting] = useState(false);

  // Activity
  const [logs, setLogs] = useState<AgentLog[]>([]);

  const refreshSources = useCallback(() => {
    void listSources()
      .then(setSources)
      .catch((err) => console.error(err));
  }, []);

  const refreshLogs = useCallback(() => {
    void listAgentLogs(50)
      .then(setLogs)
      .catch((err) => console.error(err));
  }, []);

  useEffect(() => {
    if (!open) return;
    if (tab === "ingest") refreshSources();
    if (tab === "activity") refreshLogs();
  }, [open, tab, refreshSources, refreshLogs]);

  const runSearch = useCallback(async () => {
    if (query.trim().length === 0) {
      setHits([]);
      return;
    }
    setSearching(true);
    try {
      setHits(await semanticSearch(query, 15));
    } catch (err) {
      console.error(err);
    } finally {
      setSearching(false);
    }
  }, [query]);

  const submitIngest = useCallback(async () => {
    if (draft.trim().length === 0) return;
    setIngesting(true);
    try {
      await ingestText(draft);
      setDraft("");
      refreshSources();
    } catch (err) {
      console.error(err);
    } finally {
      setIngesting(false);
    }
  }, [draft, refreshSources]);

  if (!open) return null;

  return (
    <aside className="ai-studio">
      <header className="ai-studio-head">
        <strong>AI Studio</strong>
        <button type="button" className="ghost" onClick={onClose} aria-label="Close AI Studio">
          ×
        </button>
      </header>

      <nav className="ai-tabs">
        {(["search", "ingest", "activity"] as Tab[]).map((t) => (
          <button
            key={t}
            type="button"
            className={t === tab ? "ai-tab active" : "ai-tab"}
            onClick={() => setTab(t)}
          >
            {t === "search" ? "Search" : t === "ingest" ? "Sources" : "Activity"}
          </button>
        ))}
      </nav>

      {tab === "search" && (
        <div className="ai-panel">
          <div className="ai-search-row">
            <input
              className="ai-input"
              value={query}
              placeholder="Search your notes semantically…"
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void runSearch()}
            />
            <button type="button" className="primary" onClick={() => void runSearch()}>
              Search
            </button>
          </div>
          {searching && <p className="ai-dim">Searching…</p>}
          <ul className="ai-hits">
            {hits.map((h) => (
              <li key={h.sourceId}>
                <button type="button" className="ai-hit" onClick={() => onOpenSource(h.sourceId)}>
                  <span className="ai-hit-title">{h.title}</span>
                  <span className="ai-hit-score">{scorePercent(h.score)}%</span>
                </button>
              </li>
            ))}
            {!searching && query.trim() && hits.length === 0 && (
              <li className="ai-dim">No matches.</li>
            )}
          </ul>
        </div>
      )}

      {tab === "ingest" && (
        <div className="ai-panel">
          <textarea
            className="ai-input ai-textarea"
            rows={4}
            value={draft}
            placeholder="Paste text to ingest into your knowledge base…"
            onChange={(e) => setDraft(e.target.value)}
          />
          <button
            type="button"
            className="primary"
            onClick={() => void submitIngest()}
            disabled={ingesting}
          >
            {ingesting ? "Ingesting…" : "Ingest text"}
          </button>
          <ul className="ai-sources">
            {sources.map((s) => (
              <li key={s.id} title={s.summary ?? ""}>
                {describeSource(s)}
              </li>
            ))}
            {sources.length === 0 && <li className="ai-dim">No sources yet.</li>}
          </ul>
        </div>
      )}

      {tab === "activity" && (
        <div className="ai-panel">
          <ul className="ai-logs">
            {logs.map((l) => (
              <li key={l.id}>
                <span className="ai-log-action">{l.actionTaken}</span>
                <span className="ai-dim ai-log-prompt">“{l.prompt}”</span>
              </li>
            ))}
            {logs.length === 0 && <li className="ai-dim">No AI actions yet.</li>}
          </ul>
        </div>
      )}
    </aside>
  );
}
