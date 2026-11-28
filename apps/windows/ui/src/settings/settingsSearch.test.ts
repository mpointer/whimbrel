import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { searchRedesign } from "./redesign/search";
import { fold, SETTINGS_SEARCH_ENTRIES } from "./settingsSearch";

const search = (q: string) => searchRedesign(q).map((e) => e.id);

describe("Settings search", () => {
  it("matches nothing for an empty or blank query", () => {
    expect(search("")).toEqual([]);
    expect(search("   \t")).toEqual([]);
  });

  it("finds Local-only mode by its title, both places it shows", () => {
    expect(search("local")).toEqual(expect.arrayContaining(["ai.localOnly", "capture.localOnly"]));
  });

  it("finds Plan & billing by a keyword", () => {
    expect(search("refund")).toContain("plan.summary");
  });

  it("puts title matches before keyword matches", () => {
    const ids = search("plan");
    expect(ids[0]).toBe("plan.summary");
  });

  it("ignores case and accents", () => {
    expect(search("LOCAL-ONLY")).toContain("ai.localOnly");
    expect(search("Lócal")).toContain("ai.localOnly");
    expect(fold("Résumé")).toBe("resume");
  });

  it("treats regular-expression characters as text", () => {
    for (const q of ["(", "[a", "\\", ".*", "+"]) expect(() => search(q)).not.toThrow();
    expect(search("(")).toEqual(["connectors.mcp"]);
    expect(search("[a")).toEqual([]);
  });

  it("finds nothing for a word no setting uses", () => {
    expect(search("zzzz")).toEqual([]);
    expect(search("sign out")).toEqual([]);
  });

  it("has unique ids, each a card or row id in the Settings sections or tagged on a card they reuse", () => {
    const ids = SETTINGS_SEARCH_ENTRIES.map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
    const walk = (dir: string): string[] =>
      readdirSync(dir, { withFileTypes: true }).flatMap((d) => (d.isDirectory() ? walk(join(dir, d.name)) : d.name.endsWith(".tsx") && !d.name.includes(".test.") ? [join(dir, d.name)] : []));
    const read = (files: string[]) => files.map((f) => readFileSync(f, "utf8")).join("\n");
    const reused = readdirSync(__dirname).filter((f) => f.endsWith(".tsx") && !f.includes(".test.")).map((f) => join(__dirname, f));
    const source = read([...walk(join(__dirname, "redesign", "sections")), ...reused]);
    const tagged = [...read(reused).matchAll(/data-settings-anchor="([a-zA-Z.]+)"/g)].map((m) => m[1]);
    for (const entry of SETTINGS_SEARCH_ENTRIES) {
      expect(source.includes(`id="${entry.id}"`) || source.includes(`"${entry.id}"`) || tagged.includes(entry.id), entry.id).toBe(true);
    }
    // Every tag is in the index, so none is left behind by a renamed entry.
    for (const id of new Set(tagged)) expect(ids, id).toContain(id);
  });
});
