import { describe, expect, it } from "vitest";
import { iconNames, type IconName } from "../components/icons";
import {
  androidPillars,
  androidSecurityPoints,
  androidSecuritySteps,
  androidStores,
  androidTagline,
  androidTrustChips,
} from "./android";

const isValidIcon = (name: IconName) => iconNames.includes(name);

describe("android content data", () => {
  it("has a tagline and three feature pillars with valid icons", () => {
    expect(androidTagline.length).toBeGreaterThan(0);
    expect(androidPillars).toHaveLength(3);
    for (const p of androidPillars) {
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
    expect(androidTrustChips.length).toBeGreaterThan(0);
    for (const c of androidTrustChips) {
      expect(c.label.length).toBeGreaterThan(0);
      expect(isValidIcon(c.icon)).toBe(true);
    }
  });

  it("describes the key pipeline and hardening points", () => {
    expect(androidSecuritySteps.length).toBeGreaterThanOrEqual(3);
    for (const s of androidSecuritySteps) {
      expect(s.label.length).toBeGreaterThan(0);
      expect(s.detail.length).toBeGreaterThan(0);
    }
    expect(androidSecurityPoints.length).toBeGreaterThan(0);
  });

  it("lists Android distribution channels with valid icons", () => {
    const names = androidStores.map((s) => s.name);
    expect(names).toEqual(expect.arrayContaining(["Google Play", "F-Droid", "Direct APK"]));
    for (const s of androidStores) {
      expect(s.channel.length).toBeGreaterThan(0);
      expect(s.note.length).toBeGreaterThan(0);
      expect(isValidIcon(s.icon)).toBe(true);
    }
  });
});
