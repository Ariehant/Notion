import { describe, expect, it } from "vitest";
import * as Y from "yjs";
import {
  blockText,
  blockType,
  detectMarkdownShortcut,
  ensureNonEmpty,
  getBlocks,
  mergeWithPrevious,
  newBlock,
  splitBlock,
} from "./blocks";

describe("detectMarkdownShortcut", () => {
  it.each([
    ["# ", "heading1", 2],
    ["## ", "heading2", 3],
    ["### ", "heading3", 4],
    ["- ", "bulletListItem", 2],
    ["* ", "bulletListItem", 2],
    ["[] ", "checkListItem", 3],
    ["[ ] ", "checkListItem", 4],
    ["> ", "quote", 2],
    ["1. ", "numberedListItem", 3],
    ["42. ", "numberedListItem", 4],
  ])("maps %j to %s", (input, type, strip) => {
    expect(detectMarkdownShortcut(input as string)).toEqual({ type, strip });
  });

  it("prefers the longest marker (## over #)", () => {
    expect(detectMarkdownShortcut("## ")?.type).toBe("heading2");
  });

  it("returns null for plain text", () => {
    expect(detectMarkdownShortcut("hello")).toBeNull();
    expect(detectMarkdownShortcut("-no space")).toBeNull();
  });
});

describe("block structural ops (Yjs)", () => {
  it("ensureNonEmpty seeds exactly one paragraph", () => {
    const doc = new Y.Doc();
    ensureNonEmpty(doc);
    ensureNonEmpty(doc);
    expect(getBlocks(doc).length).toBe(1);
    expect(blockType(getBlocks(doc).get(0))).toBe("paragraph");
  });

  it("splitBlock moves the caret tail into a new block", () => {
    const doc = new Y.Doc();
    const blocks = getBlocks(doc);
    blocks.insert(0, [newBlock("paragraph", "hello world")]);
    const created = splitBlock(doc, blocks.get(0), 5);
    expect(blocks.length).toBe(2);
    expect(blockText(blocks.get(0)).toString()).toBe("hello");
    expect(blockText(created).toString()).toBe(" world");
  });

  it("splitBlock keeps list types going", () => {
    const doc = new Y.Doc();
    const blocks = getBlocks(doc);
    blocks.insert(0, [newBlock("bulletListItem", "item")]);
    const created = splitBlock(doc, blocks.get(0), 4);
    expect(blockType(created)).toBe("bulletListItem");
  });

  it("mergeWithPrevious concatenates and reports the join caret", () => {
    const doc = new Y.Doc();
    const blocks = getBlocks(doc);
    blocks.insert(0, [newBlock("paragraph", "foo"), newBlock("paragraph", "bar")]);
    const res = mergeWithPrevious(doc, blocks.get(1));
    expect(res).not.toBeNull();
    expect(res?.caret).toBe(3);
    expect(blocks.length).toBe(1);
    expect(blockText(blocks.get(0)).toString()).toBe("foobar");
  });

  it("mergeWithPrevious is a no-op on the first block", () => {
    const doc = new Y.Doc();
    const blocks = getBlocks(doc);
    blocks.insert(0, [newBlock("paragraph", "only")]);
    expect(mergeWithPrevious(doc, blocks.get(0))).toBeNull();
  });
});
