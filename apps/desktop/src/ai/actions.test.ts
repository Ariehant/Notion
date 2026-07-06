import { describe, expect, it } from "vitest";
import type { AgentOutcome, IngestedSource, SearchHit } from "../bridge";
import {
  dedupeHits,
  describeSource,
  formatOutcome,
  isIngestibleTextFile,
  normalizePrompt,
  planDrop,
  scorePercent,
  truncate,
} from "./actions";

describe("normalizePrompt", () => {
  it("trims and rejects blank input", () => {
    expect(normalizePrompt("  hello  ")).toBe("hello");
    expect(normalizePrompt("   ")).toBeNull();
    expect(normalizePrompt("")).toBeNull();
  });
});

describe("formatOutcome", () => {
  const mk = (kind: string, message: string): AgentOutcome => ({
    kind,
    message,
    createdId: null,
  });
  it("prefixes an emoji by action kind", () => {
    expect(formatOutcome(mk("add_event", "Added event"))).toBe("📅 Added event");
    expect(formatOutcome(mk("create_page", "Created page"))).toBe("📄 Created page");
    expect(formatOutcome(mk("answer", "42"))).toBe("✨ 42");
  });
});

describe("truncate", () => {
  it("leaves short text untouched", () => {
    expect(truncate("short", 10)).toBe("short");
  });
  it("cuts long text on a word boundary with an ellipsis", () => {
    const out = truncate("the quick brown fox jumps over", 18);
    expect(out.endsWith("…")).toBe(true);
    expect(out.length).toBeLessThanOrEqual(19);
    expect(out).not.toContain("jumps");
  });
});

describe("describeSource", () => {
  const base: IngestedSource = {
    id: "s1",
    sourceType: "pdf",
    sourcePath: "/tmp/a.pdf",
    title: "Report",
    summary: null,
    processedAt: 0,
  };
  it("uppercases the kind and appends a summary when present", () => {
    expect(describeSource(base)).toBe("PDF · Report");
    expect(describeSource({ ...base, summary: "A short summary" })).toBe(
      "PDF · Report · A short summary",
    );
  });
});

describe("dedupeHits", () => {
  it("keeps the best score per source and sorts descending", () => {
    const hits: SearchHit[] = [
      { sourceId: "a", score: 0.2, title: "A" },
      { sourceId: "b", score: 0.9, title: "B" },
      { sourceId: "a", score: 0.5, title: "A" },
    ];
    const out = dedupeHits(hits);
    expect(out.map((h) => h.sourceId)).toEqual(["b", "a"]);
    expect(out.find((h) => h.sourceId === "a")?.score).toBe(0.5);
  });
});

describe("scorePercent", () => {
  it("clamps to 0–100 integers", () => {
    expect(scorePercent(0.734)).toBe(73);
    expect(scorePercent(-1)).toBe(0);
    expect(scorePercent(2)).toBe(100);
  });
});

describe("isIngestibleTextFile", () => {
  it("accepts text mime types and known extensions", () => {
    expect(isIngestibleTextFile("notes.txt", "")).toBe(true);
    expect(isIngestibleTextFile("README.md", "")).toBe(true);
    expect(isIngestibleTextFile("data", "text/plain")).toBe(true);
  });
  it("rejects binary docs handled by the native extractor", () => {
    expect(isIngestibleTextFile("paper.pdf", "application/pdf")).toBe(false);
    expect(isIngestibleTextFile("clip.mp3", "audio/mpeg")).toBe(false);
  });
});

describe("planDrop", () => {
  const file = (name: string, type: string): File => new File(["x"], name, { type });

  it("prefers ingestible text files", () => {
    const plan = planDrop("", [file("a.md", "")]);
    expect(plan.action).toBe("ingest-files");
  });
  it("rejects non-text files with a helpful reason", () => {
    const plan = planDrop("", [file("a.pdf", "application/pdf")]);
    expect(plan).toEqual({
      action: "ignore",
      reason: expect.stringContaining("CLI"),
    });
  });
  it("falls back to dropped text", () => {
    expect(planDrop("some pasted text", [])).toEqual({
      action: "ingest-text",
      text: "some pasted text",
    });
  });
  it("ignores an empty drop", () => {
    expect(planDrop("   ", []).action).toBe("ignore");
  });
});
