import { describe, expect, it } from "vitest";
import { computeTextDelta, isNoopDelta } from "./textdiff";

/** Apply a delta to a string the same way Y.Text.delete/insert would. */
function applyDelta(prev: string, d: { index: number; remove: number; insert: string }): string {
  return prev.slice(0, d.index) + d.insert + prev.slice(d.index + d.remove);
}

describe("computeTextDelta", () => {
  const cases: Array<[string, string]> = [
    ["", "hello"], // insert into empty
    ["hello", ""], // delete everything
    ["hello", "hello world"], // append
    ["hello world", "hello"], // truncate
    ["cat", "cart"], // insert in middle
    ["cart", "cat"], // delete in middle
    ["abc", "aXc"], // replace middle char
    ["the quick fox", "the slow fox"], // replace a word
    ["aaa", "aa"], // remove a repeated char (prefix/suffix overlap guard)
    ["", ""], // no-op
    ["same", "same"], // no-op
    ["🙂ab", "🙂Xb"], // surrogate-pair aware indexing (UTF-16 units)
  ];

  it.each(cases)("reconstructs %j -> %j", (prev, next) => {
    const d = computeTextDelta(prev, next);
    expect(applyDelta(prev, d)).toBe(next);
  });

  it("produces a minimal edit (only the changed span)", () => {
    const d = computeTextDelta("hello world", "hello brave world");
    expect(d.index).toBe(6); // after "hello "
    expect(d.remove).toBe(0);
    expect(d.insert).toBe("brave ");
  });

  it("flags no-op deltas for equal strings", () => {
    expect(isNoopDelta(computeTextDelta("x", "x"))).toBe(true);
    expect(isNoopDelta(computeTextDelta("x", "y"))).toBe(false);
  });
});
