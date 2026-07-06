import { describe, it, expect } from "vitest";
import { BLOCK_SPECS, getBlockSpec, customBlocks, starredGapBlocks } from "./schema";

describe("block schema registry (§3 editor-block gaps)", () => {
  it("includes the ★ blocks the audit says users notice immediately", () => {
    const types = new Set(BLOCK_SPECS.map((b) => b.type));
    for (const required of ["toggle", "toggleHeading", "callout", "columns", "simpleTable"]) {
      expect(types.has(required)).toBe(true);
    }
  });

  it("marks the gap-filling blocks as custom (need implementation)", () => {
    expect(getBlockSpec("callout")?.custom).toBe(true);
    expect(getBlockSpec("columns")?.custom).toBe(true);
    // A standard block stays built-in.
    expect(getBlockSpec("paragraph")?.custom).toBe(false);
  });

  it("has no duplicate block types", () => {
    const types = BLOCK_SPECS.map((b) => b.type);
    expect(new Set(types).size).toBe(types.length);
  });

  it("starred blocks are a subset of custom blocks", () => {
    const customTypes = new Set(customBlocks().map((b) => b.type));
    for (const b of starredGapBlocks()) {
      expect(customTypes.has(b.type)).toBe(true);
    }
  });
});
