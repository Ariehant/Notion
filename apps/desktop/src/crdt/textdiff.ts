/**
 * Minimal single-range text diff — the primitive behind the editor's Yjs binding.
 *
 * A `contentEditable` block reports its whole new string on every input. Naively
 * clearing and re-inserting the Y.Text on each keystroke would spray tombstones
 * across the CRDT (bloating updates + snapshots). Instead we compute the single
 * changed span (common prefix + common suffix) and emit one delete+insert, which
 * is what a human edit actually is. This keeps CRDT updates tiny and merges clean.
 *
 * Pure and framework-free so it can be unit-tested in isolation.
 */

export interface TextDelta {
  /** Offset at which the change begins. */
  index: number;
  /** Number of code units to delete starting at `index`. */
  remove: number;
  /** String to insert at `index` (after the deletion). */
  insert: string;
}

/**
 * Compute the minimal delta turning `prev` into `next`.
 *
 * Returns a no-op delta (`remove: 0, insert: ""`) when the strings are equal.
 * Operates on UTF-16 code units (JS string indexing), which is exactly what
 * `Y.Text.insert`/`delete` expect.
 */
export function computeTextDelta(prev: string, next: string): TextDelta {
  if (prev === next) return { index: prev.length, remove: 0, insert: "" };

  const maxPrefix = Math.min(prev.length, next.length);
  let prefix = 0;
  while (prefix < maxPrefix && prev[prefix] === next[prefix]) prefix++;

  // Longest common suffix, not overlapping the shared prefix on either side.
  let suffix = 0;
  const maxSuffix = Math.min(prev.length - prefix, next.length - prefix);
  while (suffix < maxSuffix && prev[prev.length - 1 - suffix] === next[next.length - 1 - suffix]) {
    suffix++;
  }

  return {
    index: prefix,
    remove: prev.length - prefix - suffix,
    insert: next.slice(prefix, next.length - suffix),
  };
}

/** True if the delta changes nothing. */
export function isNoopDelta(d: TextDelta): boolean {
  return d.remove === 0 && d.insert.length === 0;
}
