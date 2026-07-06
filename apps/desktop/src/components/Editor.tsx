import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import * as Y from "yjs";
import {
  BlockType,
  LOCAL_ORIGIN,
  blockChecked,
  blockText,
  blockType,
  blockId,
  detectMarkdownShortcut,
  docPlainText,
  getBlocks,
  mergeWithPrevious,
  setBlockType,
  splitBlock,
  stripLeading,
  toggleChecked,
} from "../crdt/blocks";
import { computeTextDelta, isNoopDelta } from "../crdt/textdiff";
import { indexPage } from "../bridge";

// --- Caret helpers for plain-text contentEditable blocks -------------------
//
// The block is a single text node (we intercept every key that would make the
// browser insert a <br>/<div>, and we render via textContent), so offsets
// measured with Range.toString() line up exactly with the Y.Text string.

function caretOffset(el: HTMLElement): number {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) return el.textContent?.length ?? 0;
  const range = sel.getRangeAt(0);
  const pre = range.cloneRange();
  pre.selectNodeContents(el);
  pre.setEnd(range.endContainer, range.endOffset);
  return pre.toString().length;
}

/** The current selection's [start, end) offsets within `el`. */
function selectionOffsets(el: HTMLElement): { start: number; end: number } {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) {
    const o = el.textContent?.length ?? 0;
    return { start: o, end: o };
  }
  const range = sel.getRangeAt(0);
  const pre = range.cloneRange();
  pre.selectNodeContents(el);
  pre.setEnd(range.startContainer, range.startOffset);
  const start = pre.toString().length;
  return { start, end: start + range.toString().length };
}

function setCaret(el: HTMLElement, offset: number): void {
  el.focus();
  const sel = window.getSelection();
  if (!sel) return;
  const node = el.firstChild;
  const range = document.createRange();
  if (node && node.nodeType === Node.TEXT_NODE) {
    const max = node.textContent?.length ?? 0;
    range.setStart(node, Math.min(offset, max));
  } else {
    range.setStart(el, 0);
  }
  range.collapse(true);
  sel.removeAllRanges();
  sel.addRange(range);
}

// --- Block list derivation --------------------------------------------------

interface Item {
  block: Y.Map<unknown>;
  id: string;
  type: BlockType;
  checked: boolean;
  ordinal: number | null;
}

function deriveItems(doc: Y.Doc): Item[] {
  const blocks = getBlocks(doc);
  const items: Item[] = [];
  let counter = 0;
  for (let i = 0; i < blocks.length; i++) {
    const block = blocks.get(i);
    const type = blockType(block);
    if (type === "numberedListItem") counter += 1;
    else counter = 0;
    items.push({
      block,
      id: blockId(block),
      type,
      checked: blockChecked(block),
      ordinal: type === "numberedListItem" ? counter : null,
    });
  }
  return items;
}

function useBlocks(doc: Y.Doc): Item[] {
  const [items, setItems] = useState<Item[]>(() => deriveItems(doc));
  useEffect(() => {
    const blocks = getBlocks(doc);
    const onDeep = (events: Array<Y.YEvent<Y.AbstractType<unknown>>>) => {
      // Text-only edits don't change the list's shape; skip re-render (the
      // contentEditable already reflects them). Structural / type / checked
      // changes target the array or a Y.Map, so re-derive then.
      const structural = events.some((e) => !(e.target instanceof Y.Text));
      if (structural) setItems(deriveItems(doc));
    };
    setItems(deriveItems(doc));
    blocks.observeDeep(onDeep);
    return () => blocks.unobserveDeep(onDeep);
  }, [doc]);
  return items;
}

// --- Slash command menu -----------------------------------------------------

const SLASH_COMMANDS: Array<{ label: string; type: BlockType }> = [
  { label: "Text", type: "paragraph" },
  { label: "Heading 1", type: "heading1" },
  { label: "Heading 2", type: "heading2" },
  { label: "Heading 3", type: "heading3" },
  { label: "Bulleted list", type: "bulletListItem" },
  { label: "Numbered list", type: "numberedListItem" },
  { label: "To-do list", type: "checkListItem" },
  { label: "Quote", type: "quote" },
  { label: "Code", type: "codeBlock" },
];

interface SlashState {
  blockId: string;
  top: number;
  left: number;
}

// --- Individual block row ---------------------------------------------------

interface BlockRowProps {
  doc: Y.Doc;
  item: Item;
  registerRef: (id: string, el: HTMLDivElement | null) => void;
  onEnter: (block: Y.Map<unknown>, offset: number) => void;
  onMergeBackspace: (block: Y.Map<unknown>) => void;
  onOpenSlash: (block: Y.Map<unknown>, el: HTMLDivElement) => void;
  onCloseSlash: () => void;
}

const BlockRow = function BlockRow({
  doc,
  item,
  registerRef,
  onEnter,
  onMergeBackspace,
  onOpenSlash,
  onCloseSlash,
}: BlockRowProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const { block, type } = item;
  const ytext = blockText(block);

  // Keep the DOM text in sync with the Y.Text via textContent (never innerText:
  // its setter turns "\n" into <br>, which would desync caret offsets). We write
  // ONLY when the strings differ, so our own keystrokes never reset the caret. A
  // layout effect so a freshly mounted block's text is in the DOM *before* the
  // editor's focus layout effect runs (correct caret after a split).
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const sync = () => {
      const s = ytext.toString();
      if (el.textContent !== s) el.textContent = s;
    };
    sync();
    ytext.observe(sync);
    return () => ytext.unobserve(sync);
  }, [ytext]);

  // Replace the current selection with `str` at the model level and place the
  // caret after it — used for paste and soft/code newlines, so clipboard HTML
  // and browser-inserted <br>s never enter the block (§2.8) and the DOM stays a
  // single text node.
  const replaceSelection = useCallback(
    (el: HTMLDivElement, str: string) => {
      const { start, end } = selectionOffsets(el);
      doc.transact(() => {
        if (end > start) ytext.delete(start, end - start);
        if (str) ytext.insert(start, str);
      }, LOCAL_ORIGIN);
      setCaret(el, start + str.length);
    },
    [doc, ytext],
  );

  const onInput = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const next = el.textContent ?? "";
    const prev = ytext.toString();
    if (next !== prev) {
      const delta = computeTextDelta(prev, next);
      if (!isNoopDelta(delta)) {
        doc.transact(() => {
          if (delta.remove > 0) ytext.delete(delta.index, delta.remove);
          if (delta.insert) ytext.insert(delta.index, delta.insert);
        }, LOCAL_ORIGIN);
      }
    }

    // Slash menu: a lone "/" in an empty paragraph opens the command palette.
    if (type === "paragraph" && next === "/") {
      onOpenSlash(block, el);
      return;
    }
    onCloseSlash();

    // Markdown shortcuts convert a leading marker into a block type.
    if (type === "paragraph") {
      const hit = detectMarkdownShortcut(next);
      if (hit) {
        stripLeading(doc, block, hit.strip);
        setBlockType(doc, block, hit.type);
      }
    }
  }, [block, doc, onCloseSlash, onOpenSlash, type, ytext]);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      const el = ref.current;
      if (!el) return;
      // Never hijack keys from an in-progress IME composition (Enter confirms a
      // candidate; Backspace edits it) — let the IME handle them.
      if (e.nativeEvent.isComposing || e.keyCode === 229) return;

      if (e.key === "Enter" && !e.shiftKey) {
        // In a code block, Enter is a newline; Shift+Enter is the escape hatch.
        if (type === "codeBlock") {
          e.preventDefault();
          replaceSelection(el, "\n");
          return;
        }
        e.preventDefault();
        onCloseSlash();
        onEnter(block, caretOffset(el));
      } else if (e.key === "Enter" && e.shiftKey) {
        // Soft line break within the block (stays one block, one text node).
        e.preventDefault();
        replaceSelection(el, "\n");
      } else if (e.key === "Backspace") {
        if (caretOffset(el) === 0 && window.getSelection()?.isCollapsed) {
          if (type !== "paragraph") {
            e.preventDefault();
            setBlockType(doc, block, "paragraph");
          } else {
            e.preventDefault();
            onMergeBackspace(block);
          }
        }
      } else if (e.key === "Escape") {
        onCloseSlash();
      }
    },
    [block, doc, onCloseSlash, onEnter, onMergeBackspace, replaceSelection, type],
  );

  // Plain-text paste only: never inject clipboard HTML into a block (§2.8). The
  // text goes through the model, so no browser HTML/<br> reaches the DOM.
  const onPaste = useCallback(
    (e: React.ClipboardEvent<HTMLDivElement>) => {
      e.preventDefault();
      const el = ref.current;
      const text = e.clipboardData.getData("text/plain");
      if (el && text) replaceSelection(el, text);
    },
    [replaceSelection],
  );

  const setRef = useCallback(
    (el: HTMLDivElement | null) => {
      ref.current = el;
      registerRef(item.id, el);
    },
    [item.id, registerRef],
  );

  const editable = (
    <div
      ref={setRef}
      className="block-content"
      contentEditable
      suppressContentEditableWarning
      role="textbox"
      aria-multiline="false"
      data-placeholder={type === "paragraph" ? "Type '/' for commands…" : ""}
      onInput={onInput}
      onKeyDown={onKeyDown}
      onPaste={onPaste}
    />
  );

  return (
    <div className={`block block-${type}`} data-block-id={item.id}>
      {type === "bulletListItem" && <span className="marker">•</span>}
      {type === "numberedListItem" && <span className="marker">{item.ordinal}.</span>}
      {type === "checkListItem" && (
        <input
          type="checkbox"
          className="marker check"
          checked={item.checked}
          onChange={() => toggleChecked(doc, block)}
        />
      )}
      {editable}
    </div>
  );
};

// --- The editor -------------------------------------------------------------

interface EditorProps {
  doc: Y.Doc;
  pageId: string;
  title: string;
  onTitleChange: (title: string) => void;
}

export function Editor({ doc, pageId, title, onTitleChange }: EditorProps) {
  const items = useBlocks(doc);
  const refs = useRef(new Map<string, HTMLDivElement>());
  const pendingFocus = useRef<{ id: string; offset: number } | null>(null);
  const [slash, setSlash] = useState<SlashState | null>(null);

  const registerRef = useCallback((id: string, el: HTMLDivElement | null) => {
    if (el) refs.current.set(id, el);
    else refs.current.delete(id);
  }, []);

  // Apply queued focus (after a split/merge changes the block list).
  useLayoutEffect(() => {
    const target = pendingFocus.current;
    if (!target) return;
    const el = refs.current.get(target.id);
    if (el) {
      setCaret(el, target.offset);
      pendingFocus.current = null;
    }
  }, [items]);

  const onEnter = useCallback(
    (block: Y.Map<unknown>, offset: number) => {
      const text = blockText(block).toString();
      const type = blockType(block);
      const isList =
        type === "bulletListItem" || type === "numberedListItem" || type === "checkListItem";
      // Enter on an empty list item exits the list (Notion behaviour).
      if (isList && text.length === 0) {
        setBlockType(doc, block, "paragraph");
        pendingFocus.current = { id: blockId(block), offset: 0 };
        return;
      }
      const created = splitBlock(doc, block, offset);
      pendingFocus.current = { id: blockId(created), offset: 0 };
    },
    [doc],
  );

  const onMergeBackspace = useCallback(
    (block: Y.Map<unknown>) => {
      const res = mergeWithPrevious(doc, block);
      if (res) pendingFocus.current = { id: blockId(res.prev), offset: res.caret };
    },
    [doc],
  );

  const onOpenSlash = useCallback((block: Y.Map<unknown>, el: HTMLDivElement) => {
    const rect = el.getBoundingClientRect();
    setSlash({ blockId: blockId(block), top: rect.bottom + 4, left: rect.left });
  }, []);

  const onCloseSlash = useCallback(() => setSlash((s) => (s ? null : s)), []);

  const applySlash = useCallback(
    (type: BlockType) => {
      if (!slash) return;
      const blocks = getBlocks(doc);
      for (let i = 0; i < blocks.length; i++) {
        const b = blocks.get(i);
        if (blockId(b) === slash.blockId) {
          stripLeading(doc, b, blockText(b).toString().length); // clear the "/"
          setBlockType(doc, b, type);
          pendingFocus.current = { id: slash.blockId, offset: 0 };
          break;
        }
      }
      setSlash(null);
    },
    [doc, slash],
  );

  // Debounced full-text indexing (title + body) for search (§1.8).
  const titleRef = useRef(title);
  const scheduleIndex = useRef<() => void>(() => {});
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = () => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        void indexPage(pageId, titleRef.current, docPlainText(doc)).catch((err) =>
          console.error("index failed", err),
        );
      }, 1200);
    };
    scheduleIndex.current = run;
    doc.on("update", run);
    run();
    return () => {
      clearTimeout(timer);
      doc.off("update", run);
    };
  }, [doc, pageId]);

  useEffect(() => {
    titleRef.current = title;
    scheduleIndex.current();
  }, [title]);

  const slashList = useMemo(() => SLASH_COMMANDS, []);

  return (
    <div className="editor" onClick={() => slash && onCloseSlash()}>
      <input
        className="page-title"
        value={title}
        placeholder="Untitled"
        onChange={(e) => onTitleChange(e.target.value)}
      />
      <div className="blocks">
        {items.map((item) => (
          <BlockRow
            key={item.id}
            doc={doc}
            item={item}
            registerRef={registerRef}
            onEnter={onEnter}
            onMergeBackspace={onMergeBackspace}
            onOpenSlash={onOpenSlash}
            onCloseSlash={onCloseSlash}
          />
        ))}
      </div>

      {slash && (
        <ul
          className="slash-menu"
          style={{ top: slash.top, left: slash.left }}
          onClick={(e) => e.stopPropagation()}
        >
          {slashList.map((cmd) => (
            <li key={cmd.type}>
              <button
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => applySlash(cmd.type)}
              >
                {cmd.label}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
