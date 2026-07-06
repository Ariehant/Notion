import { useEffect, useRef, useState } from "react";
import { runAgent } from "../bridge";
import { formatOutcome, normalizePrompt } from "../ai/actions";

interface AiDialogProps {
  open: boolean;
  /** Seed text (e.g. the block the user invoked `/ai` from). */
  initialPrompt?: string;
  /** The editor block this action is associated with, for the activity log. */
  blockId?: string;
  onClose: () => void;
  /** Called after a successful action so the shell can refresh pages/events. */
  onDone: () => void;
}

/**
 * The "magic wand" / `/ai` prompt. Sends a natural-language instruction to the
 * local action agent ("add an event for tomorrow at 3pm", "make a page for Q3")
 * and shows the outcome. Thin glue over `runAgent`; the prompt/outcome logic is
 * the tested `ai/actions` module.
 */
export function AiDialog({ open, initialPrompt, blockId, onClose, onDone }: AiDialogProps) {
  const [prompt, setPrompt] = useState(initialPrompt ?? "");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (open) {
      setPrompt(initialPrompt ?? "");
      setResult(null);
      setError(null);
      // Focus after mount.
      queueMicrotask(() => inputRef.current?.focus());
    }
  }, [open, initialPrompt]);

  if (!open) return null;

  const submit = async () => {
    const clean = normalizePrompt(prompt);
    if (!clean) return;
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const outcome = await runAgent(clean, blockId);
      setResult(formatOutcome(outcome));
      onDone();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="ai-modal-backdrop" onClick={onClose}>
      <div className="ai-modal" onClick={(e) => e.stopPropagation()}>
        <h3 className="ai-modal-title">Ask AI ✨</h3>
        <p className="ai-modal-hint">
          Describe what you want. Try “add a calendar event for tomorrow at 3pm” or “make a page
          called Q3 planning”.
        </p>
        <textarea
          ref={inputRef}
          className="ai-modal-input"
          rows={3}
          value={prompt}
          placeholder="What do you want to do?"
          disabled={busy}
          onChange={(e) => setPrompt(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void submit();
            if (e.key === "Escape") onClose();
          }}
        />
        {result && <div className="ai-result">{result}</div>}
        {error && <div className="ai-error">{error}</div>}
        <div className="ai-modal-actions">
          <button type="button" className="ghost" onClick={onClose} disabled={busy}>
            Close
          </button>
          <button type="button" className="primary" onClick={() => void submit()} disabled={busy}>
            {busy ? "Thinking…" : "Ask"}
          </button>
        </div>
      </div>
    </div>
  );
}
