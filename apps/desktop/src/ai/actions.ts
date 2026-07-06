/**
 * Pure UI-logic for the Open Notebook AI features.
 *
 * All the fiddly, edge-case-prone decisions the AI panels make — is a dropped
 * file ingestible? what emoji/label represents this outcome? is a prompt empty?
 * how do we render a source line? — live here as pure functions so they are
 * unit-tested (the React components stay thin glue, like the rest of the app).
 */
import type { AgentOutcome, IngestedSource, SearchHit } from "../bridge";

/** Trim a user's agent/magic-wand prompt; `null` if there's nothing to send. */
export function normalizePrompt(raw: string): string | null {
  const trimmed = raw.trim();
  return trimmed.length === 0 ? null : trimmed;
}

/** A friendly one-line result string with an emoji cue per action kind. */
export function formatOutcome(outcome: AgentOutcome): string {
  const icon = outcome.kind === "add_event" ? "📅" : outcome.kind === "create_page" ? "📄" : "✨";
  return `${icon} ${outcome.message}`;
}

/** Truncate to `max` chars on a word-ish boundary, adding an ellipsis. */
export function truncate(text: string, max: number): string {
  if (text.length <= max) return text;
  const slice = text.slice(0, max);
  const lastSpace = slice.lastIndexOf(" ");
  const cut = lastSpace > max * 0.6 ? slice.slice(0, lastSpace) : slice;
  return `${cut.trimEnd()}…`;
}

/** A compact display line for an ingested source: "PDF · Title · summary…". */
export function describeSource(s: IngestedSource): string {
  const kind = s.sourceType.toUpperCase();
  const summary = s.summary ? ` · ${truncate(s.summary, 80)}` : "";
  return `${kind} · ${s.title}${summary}`;
}

/** Sort + de-duplicate search hits by source, keeping the highest score. */
export function dedupeHits(hits: SearchHit[]): SearchHit[] {
  const best = new Map<string, SearchHit>();
  for (const h of hits) {
    const prev = best.get(h.sourceId);
    if (!prev || h.score > prev.score) best.set(h.sourceId, h);
  }
  return [...best.values()].sort((a, b) => b.score - a.score);
}

/** A 0–100 integer relevance for display from a raw cosine-ish score. */
export function scorePercent(score: number): number {
  return Math.max(0, Math.min(100, Math.round(score * 100)));
}

const TEXT_EXTENSIONS = [".txt", ".md", ".markdown", ".text", ".log", ".csv"];

/**
 * Whether a dropped file can be ingested as plain text in the frontend (read
 * via `FileReader`). PDFs/audio need the native extractor and are handled by the
 * backend/CLI, so they are intentionally excluded here.
 */
export function isIngestibleTextFile(name: string, mime: string): boolean {
  if (mime.startsWith("text/")) return true;
  const lower = name.toLowerCase();
  return TEXT_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

/** Classify a drop payload into what the UI should do with it. */
export type DropPlan =
  | { action: "ingest-text"; text: string }
  | { action: "ingest-files"; files: File[] }
  | { action: "ignore"; reason: string };

/**
 * Decide how to handle a drop given its plain-text payload and file list.
 * Prefers real files (they carry a name/title); falls back to dropped text.
 */
export function planDrop(text: string, files: File[]): DropPlan {
  const ingestible = files.filter((f) => isIngestibleTextFile(f.name, f.type));
  if (ingestible.length > 0) return { action: "ingest-files", files: ingestible };
  if (files.length > 0)
    return {
      action: "ignore",
      reason: "Only text files (.txt, .md) can be dropped in — use the CLI for PDFs.",
    };
  const trimmed = text.trim();
  if (trimmed.length > 0) return { action: "ingest-text", text: trimmed };
  return { action: "ignore", reason: "Nothing to ingest." };
}
