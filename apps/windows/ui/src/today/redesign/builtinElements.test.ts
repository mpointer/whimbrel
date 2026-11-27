import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { SkinManifest } from "../../skin";
import { DEFAULT_ELEMENTS, elementsFromOptions, type ElementChoices } from "./elements";

// Richer skins row 12: Classic, High Contrast and Hypermodern choose their Home elements in `today.options`
// (docs/superpowers/plans/2026-11-25-richer-skins.md, section 7). The Rust side tests the same values and the
// contrast pairs (core/skins/tests/builtin_elements.rs).

const bundledManifests = Object.values(import.meta.glob("../../../../../../skins/*.json", { import: "default", eager: true }) as Record<string, SkinManifest>).filter((m) => Array.isArray(m.palettes));
const skinFile = (name: string) => JSON.parse(readFileSync(fileURLToPath(new URL(`../../../../../../skins/${name}.json`, import.meta.url)), "utf8"));
const manifest = (id: string) => bundledManifests.find((m) => m.id === id)!;
const elements = (id: string) => elementsFromOptions(manifest(id).today.options, manifest(id).today.archetype);

const WANT: Record<string, ElementChoices> = {
  whimbrel: { nextMeeting: "infobox", timeline: "line", labels: "smallCaps", ask: "field", actions: "buttons" },
  "high-contrast": { nextMeeting: "infobox", timeline: "ruled", labels: "plain", ask: "field", actions: "buttons" },
  hypermodern: { nextMeeting: "scoreboard", timeline: "line", labels: "plain", ask: "prompt", actions: "buttons" },
};

describe("the built-in skins' Home elements", () => {
  for (const [id, want] of Object.entries(WANT)) {
    it(`${id} resolves to its own elements, not the default`, () => {
      expect(elements(id)).toEqual(want);
      expect(elements(id)).not.toEqual(DEFAULT_ELEMENTS);
    });
    it(`${id} writes all five keys and keeps its archetype`, () => {
      const file = skinFile(id);
      expect(Object.keys(file.today.options).sort()).toEqual(["actions", "ask", "labels", "nextMeeting", "timeline"]);
      expect(manifest(id).today.archetype).toBe(file.today.archetype);
    });
    it(`an old synced copy of ${id} (no element keys) still draws its old Home`, () => {
      expect(elementsFromOptions({}, manifest(id).today.archetype)).toEqual(DEFAULT_ELEMENTS);
      expect(elementsFromOptions(undefined, manifest(id).today.archetype)).toEqual(DEFAULT_ELEMENTS);
    });
  }

  it("leaves Thread as it was", () => {
    expect(elements("thread")).toEqual(DEFAULT_ELEMENTS);
    expect(manifest("thread").today.options).toEqual({ actions: "buttons", ask: "field", labels: "plain", nextMeeting: "card", timeline: "line" });
  });
});
