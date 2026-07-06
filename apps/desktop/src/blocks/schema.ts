/**
 * Block schema registry — audit §3 (missing editor blocks).
 *
 * The v1 blueprint omitted several signature Notion blocks. This registry is the
 * single declaration of the block set the editor supports, tagging which are
 * built into BlockNote vs which need custom implementations (the §3 gaps). It
 * lets the slash menu, paste mapper, and export all agree on one list, and lets
 * a test assert the gap-filling blocks are actually present.
 */

export type BlockCategory = "text" | "list" | "media" | "layout" | "database" | "embed";

export interface BlockSpec {
  /** Stable block type id used in the CRDT + storage. */
  type: string;
  /** Human label for the slash menu. */
  label: string;
  category: BlockCategory;
  /** True if it needs a custom BlockNote implementation (not built-in). */
  custom: boolean;
  /** Marked ★ in the audit as "users notice immediately". */
  audit_starred?: boolean;
}

export const BLOCK_SPECS: readonly BlockSpec[] = [
  // Built-in text blocks.
  { type: "paragraph", label: "Text", category: "text", custom: false },
  { type: "heading", label: "Heading", category: "text", custom: false },
  { type: "bulletListItem", label: "Bulleted List", category: "list", custom: false },
  { type: "numberedListItem", label: "Numbered List", category: "list", custom: false },
  { type: "checkListItem", label: "To-do List", category: "list", custom: false },
  { type: "quote", label: "Quote", category: "text", custom: false },
  { type: "codeBlock", label: "Code", category: "text", custom: false },
  { type: "image", label: "Image", category: "media", custom: false },
  { type: "divider", label: "Divider", category: "text", custom: false },

  // ★ §3 gap-filling custom blocks.
  { type: "toggle", label: "Toggle List", category: "list", custom: true, audit_starred: true },
  {
    type: "toggleHeading",
    label: "Toggle Heading",
    category: "text",
    custom: true,
    audit_starred: true,
  },
  { type: "callout", label: "Callout", category: "text", custom: true, audit_starred: true },
  { type: "columns", label: "Columns", category: "layout", custom: true, audit_starred: true },
  { type: "simpleTable", label: "Table", category: "layout", custom: true, audit_starred: true },

  // Other §3 gaps (non-starred).
  { type: "audio", label: "Audio", category: "media", custom: true },
  { type: "pdf", label: "PDF", category: "media", custom: true },
  { type: "embed", label: "Embed", category: "embed", custom: true },
] as const;

const BY_TYPE = new Map(BLOCK_SPECS.map((b) => [b.type, b]));

export function getBlockSpec(type: string): BlockSpec | undefined {
  return BY_TYPE.get(type);
}

export function customBlocks(): BlockSpec[] {
  return BLOCK_SPECS.filter((b) => b.custom);
}

/** The starred §3 blocks that must ship in Phase 1. */
export function starredGapBlocks(): BlockSpec[] {
  return BLOCK_SPECS.filter((b) => b.audit_starred);
}
