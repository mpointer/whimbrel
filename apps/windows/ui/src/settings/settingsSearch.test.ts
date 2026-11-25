import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { fold, SETTINGS_SEARCH_ENTRIES, searchSettings } from "./settingsSearch";
import { sectionTitle } from "./settingsSections";

const search = (q: string) => searchSettings(q, sectionTitle).map((e) => e.id);

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

  it("has unique ids, each tagged on a Settings row in the code", () => {
    const ids = SETTINGS_SEARCH_ENTRIES.map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
    const dir = join(__dirname);
    const source = readdirSync(dir)
      .filter((f) => f.endsWith(".tsx") && !f.includes(".test."))
      .map((f) => readFileSync(join(dir, f), "utf8"))
      .join("\n");
    const tagged = [...source.matchAll(/data-settings-anchor="([a-zA-Z.]+)"/g)].map((m) => m[1]);
    for (const id of ids) expect(tagged, id).toContain(id);
    // Every tag is in the index, so none is left behind by a renamed entry.
    for (const id of new Set(tagged)) expect(ids, id).toContain(id);
  });
});
