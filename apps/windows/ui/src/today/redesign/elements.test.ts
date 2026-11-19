import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { DEFAULT_ELEMENTS, ELEMENT_KEYS, elementsFromArchetype, elementsFromOptions } from "./elements";

const doc = (name: string) => readFileSync(fileURLToPath(new URL(`../../../../../../${name}`, import.meta.url)), "utf8");

/** The rows of the conversion table in skins.md, by old layout. */
function tableRows(): Record<string, string[]> {
  const text = doc("docs/ui-redesign/skins.md");
  const section = text.slice(text.indexOf("## Converting old skins"));
  const rows: Record<string, string[]> = {};
  for (const line of section.split("\n")) {
    const cells = line.split("|").map((c) => c.trim()).filter(Boolean);
    if (cells.length === 6 && !/^-+$/.test(cells[0]) && cells[0] !== "Old layout") rows[cells[0]] = cells.slice(1);
  }
  return rows;
}

const WORDS: Record<string, string> = { card: "card", scoreboard: "scoreboard", infobox: "infobox", line: "line", ruled: "ruled", plain: "plain", bracketed: "bracketed", "small caps": "smallCaps", field: "field", prompt: "prompt", buttons: "buttons", "text links": "links" };

describe("elementsFromArchetype", () => {
  const rows = tableRows();
  it("reads the whole table from skins.md", () => expect(Object.keys(rows).sort()).toEqual(["almanac", "field", "glass", "reference", "standard", "terminal"]));
  for (const [layout, cells] of Object.entries(tableRows())) {
    it(`matches the ${layout} row`, () => {
      const [nextMeeting, timeline, labels, ask, actions] = cells.map((c) => WORDS[c]);
      expect(elementsFromArchetype(layout)).toEqual({ nextMeeting, timeline, labels, ask, actions });
    });
  }
  it("treats an unknown archetype as Standard", () => expect(elementsFromArchetype("hologram")).toEqual(DEFAULT_ELEMENTS));
});

describe("elementsFromOptions", () => {
  it("lets options win over the archetype", () => {
    const o = { nextMeeting: "scoreboard", timeline: "ruled", labels: "smallCaps", ask: "prompt", actions: "links" };
    expect(elementsFromOptions(o, "standard")).toEqual({ nextMeeting: "scoreboard", timeline: "ruled", labels: "smallCaps", ask: "prompt", actions: "links" });
  });
  it("takes a missing key from the archetype", () => {
    expect(elementsFromOptions({ ask: "field" }, "terminal")).toEqual({ ...elementsFromArchetype("terminal"), ask: "field" });
    expect(elementsFromOptions(undefined, "reference")).toEqual(elementsFromArchetype("reference"));
  });
  it("takes the default for an unknown value (not the archetype's), and ignores unknown keys", () => {
    const c = elementsFromOptions({ nextMeeting: "hologram", labels: "Bracketed", colour: "red", ask: "" }, "terminal");
    expect(c.nextMeeting).toBe("card");
    expect(c.labels).toBe("plain");
    expect(c.ask).toBe("field");
  });
  it("lists the allowed values the catalogue lists", () => {
    const rust = doc("core/skins/src/elements.rs");
    for (const [key, values] of ELEMENT_KEYS) {
      const line = rust.split("\n").find((l) => l.includes(`("${key}", &[`))!;
      expect(line, key).toBeTruthy();
      expect([...line.matchAll(/"(\w+)"/g)].map((m) => m[1]).slice(1)).toEqual(values);
    }
  });
});
