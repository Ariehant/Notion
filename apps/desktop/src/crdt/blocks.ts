/**
 * Block-document model over Yjs — audit §1.4 (Yjs is the authoritative editor doc).
 *
 * A page body is a `Y.Array` named "blocks"; each element is a `Y.Map` holding
 * `{ id, type, checked?, text: Y.Text }`. Keeping per-block text in a `Y.Text`
 * means edits merge at character granularity and the document round-trips through
 * the encrypted update log / snapshots exactly like any other Yjs structure.
 *
 * Everything here is DOM/React-free so the model can be reasoned about (and
 * partly unit-tested) on its own.
 */
import * as Y from "yjs";

/** Block kinds the editor supports today (a subset of blocks/schema.ts). */
export type BlockType =
  | "paragraph"
  | "heading1"
  | "heading2"
  | "heading3"
  | "bulletListItem"
  | "numberedListItem"
  | "checkListItem"
  | "quote"
  | "codeBlock";

/** Transaction origin marking edits made locally by this editor instance. */
export const LOCAL_ORIGIN = Symbol("notion.editor.local");

const BLOCKS_KEY = "blocks";

/** A random id; prefers the platform CSPRNG, with a safe non-crypto fallback. */
export function genId(): string {
  const c = (globalThis as { crypto?: Crypto }).crypto;
  if (c && typeof c.randomUUID === "function") return c.randomUUID();
  // Fallback (e.g. non-secure test env): random enough for a DOM key.
  return "b-" + Math.abs(hashString(String(performance.now()) + ":" + Math.random())).toString(36);
}

function hashString(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (Math.imul(31, h) + s.charCodeAt(i)) | 0;
  return h;
}

export function getBlocks(doc: Y.Doc): Y.Array<Y.Map<unknown>> {
  return doc.getArray<Y.Map<unknown>>(BLOCKS_KEY);
}

export function blockText(block: Y.Map<unknown>): Y.Text {
  return block.get("text") as Y.Text;
}

export function blockType(block: Y.Map<unknown>): BlockType {
  return (block.get("type") as BlockType) ?? "paragraph";
}

export function blockId(block: Y.Map<unknown>): string {
  return (block.get("id") as string) ?? "";
}

export function blockChecked(block: Y.Map<unknown>): boolean {
  return Boolean(block.get("checked"));
}

/** Build a detached block map ready to insert into the blocks array. */
export function newBlock(type: BlockType, text = "", checked?: boolean): Y.Map<unknown> {
  const m = new Y.Map<unknown>();
  m.set("id", genId());
  m.set("type", type);
  if (type === "checkListItem") m.set("checked", checked ?? false);
  const t = new Y.Text();
  if (text) t.insert(0, text);
  m.set("text", t);
  return m;
}

/** Ensure a document always has at least one (empty paragraph) block to edit. */
export function ensureNonEmpty(doc: Y.Doc): void {
  const blocks = getBlocks(doc);
  if (blocks.length === 0) {
    doc.transact(() => blocks.insert(0, [newBlock("paragraph")]), LOCAL_ORIGIN);
  }
}

/** Index of a block (by identity) within the array, or -1. */
export function indexOfBlock(doc: Y.Doc, block: Y.Map<unknown>): number {
  const blocks = getBlocks(doc);
  for (let i = 0; i < blocks.length; i++) if (blocks.get(i) === block) return i;
  return -1;
}

/**
 * Split `block` at `offset`: text after the caret moves into a new block placed
 * right below. List-item types continue as the same type; everything else
 * becomes a paragraph. Returns the new block (so the caller can focus it).
 */
export function splitBlock(doc: Y.Doc, block: Y.Map<unknown>, offset: number): Y.Map<unknown> {
  const blocks = getBlocks(doc);
  const text = blockText(block);
  const type = blockType(block);
  const tail = text.toString().slice(offset);
  const continues =
    type === "bulletListItem" || type === "numberedListItem" || type === "checkListItem";
  const newType: BlockType = continues ? type : "paragraph";
  let created!: Y.Map<unknown>;
  doc.transact(() => {
    if (offset < text.length) text.delete(offset, text.length - offset);
    const at = indexOfBlock(doc, block) + 1;
    created = newBlock(newType, tail);
    blocks.insert(at, [created]);
  }, LOCAL_ORIGIN);
  return created;
}

/**
 * Merge `block` into the one above it: its text is appended to the previous
 * block and it is removed. Returns `{ prev, caret }` (caret = join offset) or
 * null if it is already the first block.
 */
export function mergeWithPrevious(
  doc: Y.Doc,
  block: Y.Map<unknown>,
): { prev: Y.Map<unknown>; caret: number } | null {
  const blocks = getBlocks(doc);
  const idx = indexOfBlock(doc, block);
  if (idx <= 0) return null;
  const prev = blocks.get(idx - 1);
  const prevText = blockText(prev);
  const caret = prevText.length;
  const tail = blockText(block).toString();
  doc.transact(() => {
    if (tail) prevText.insert(prevText.length, tail);
    blocks.delete(idx, 1);
  }, LOCAL_ORIGIN);
  return { prev, caret };
}

export function insertBlockAfter(
  doc: Y.Doc,
  block: Y.Map<unknown>,
  type: BlockType = "paragraph",
): Y.Map<unknown> {
  const blocks = getBlocks(doc);
  let created!: Y.Map<unknown>;
  doc.transact(() => {
    const at = indexOfBlock(doc, block) + 1;
    created = newBlock(type);
    blocks.insert(at, [created]);
  }, LOCAL_ORIGIN);
  return created;
}

export function setBlockType(doc: Y.Doc, block: Y.Map<unknown>, type: BlockType): void {
  doc.transact(() => {
    block.set("type", type);
    if (type === "checkListItem") {
      if (block.get("checked") === undefined) block.set("checked", false);
    } else if (block.get("checked") !== undefined) {
      block.delete("checked");
    }
  }, LOCAL_ORIGIN);
}

export function toggleChecked(doc: Y.Doc, block: Y.Map<unknown>): void {
  doc.transact(() => block.set("checked", !blockChecked(block)), LOCAL_ORIGIN);
}

export function deleteBlock(doc: Y.Doc, block: Y.Map<unknown>): void {
  const idx = indexOfBlock(doc, block);
  if (idx < 0) return;
  doc.transact(() => getBlocks(doc).delete(idx, 1), LOCAL_ORIGIN);
}

/** Strip a leading `count` characters from a block's text (markdown shortcut). */
export function stripLeading(doc: Y.Doc, block: Y.Map<unknown>, count: number): void {
  doc.transact(() => {
    const t = blockText(block);
    if (count > 0) t.delete(0, Math.min(count, t.length));
  }, LOCAL_ORIGIN);
}

/** Extract the document's plaintext (for full-text search indexing). */
export function docPlainText(doc: Y.Doc): string {
  const blocks = getBlocks(doc);
  const lines: string[] = [];
  for (let i = 0; i < blocks.length; i++) lines.push(blockText(blocks.get(i)).toString());
  return lines.join("\n");
}

/**
 * Detect a Notion-style markdown shortcut at the very start of a paragraph.
 * Returns the target type and how many leading characters to strip, or null.
 * Pure — unit-tested directly.
 */
export function detectMarkdownShortcut(text: string): { type: BlockType; strip: number } | null {
  const patterns: Array<[string, BlockType]> = [
    ["# ", "heading1"],
    ["## ", "heading2"],
    ["### ", "heading3"],
    ["- ", "bulletListItem"],
    ["* ", "bulletListItem"],
    ["[] ", "checkListItem"],
    ["[ ] ", "checkListItem"],
    ["> ", "quote"],
    ["``` ", "codeBlock"],
    ["```", "codeBlock"],
  ];
  // Longer markers first so "## " wins over "# ".
  for (const [marker, type] of [...patterns].sort((a, b) => b[0].length - a[0].length)) {
    if (text.startsWith(marker)) return { type, strip: marker.length };
  }
  // Numbered list: one-or-more digits, a dot, then a space.
  const num = /^(\d+)\.\s/.exec(text);
  if (num) return { type: "numberedListItem", strip: num[0].length };
  return null;
}
