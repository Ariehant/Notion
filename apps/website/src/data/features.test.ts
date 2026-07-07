import { describe, expect, it } from "vitest";
import { iconNames, type IconName } from "../components/icons";
import { downloads, pillars, securityPoints, securitySteps, trustChips } from "./features";

const isValidIcon = (name: IconName) => iconNames.includes(name);

describe("marketing content data", () => {
  it("has exactly three feature pillars, each with content and valid icons", () => {
    expect(pillars).toHaveLength(3);
    for (const p of pillars) {
      expect(p.title.length).toBeGreaterThan(0);
      expect(p.summary.length).toBeGreaterThan(0);
      expect(isValidIcon(p.icon)).toBe(true);
      expect(p.features.length).toBeGreaterThanOrEqual(3);
      for (const f of p.features) {
        expect(f.title.length).toBeGreaterThan(0);
        expect(f.body.length).toBeGreaterThan(0);
      }
    }
  });

  it("has non-empty trust chips with valid icons", () => {
    expect(trustChips.length).toBeGreaterThan(0);
    for (const c of trustChips) {
      expect(c.label.length).toBeGreaterThan(0);
      expect(isValidIcon(c.icon)).toBe(true);
    }
  });

  it("describes the key pipeline as an ordered set of steps", () => {
    expect(securitySteps.length).toBeGreaterThanOrEqual(3);
    for (const s of securitySteps) {
      expect(s.label.length).toBeGreaterThan(0);
      expect(s.detail.length).toBeGreaterThan(0);
    }
    expect(securityPoints.length).toBeGreaterThan(0);
  });

  it("offers a download for each major OS with a valid icon", () => {
    const oses = downloads.map((d) => d.os);
    expect(oses).toEqual(expect.arrayContaining(["Windows", "macOS", "Linux"]));
    for (const d of downloads) {
      expect(d.file.length).toBeGreaterThan(0);
      expect(d.note.length).toBeGreaterThan(0);
      expect(isValidIcon(d.icon)).toBe(true);
    }
  });
});
