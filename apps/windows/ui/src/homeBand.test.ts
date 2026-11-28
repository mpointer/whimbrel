// The skin's header band on Home (richer skins row 7). Windows already draws it: `SkinHeader` (the header image and the
// `.skin-band` element) sits in the `.stage`, above the tab's content, for every tab, Today included. The Mac
// had to be taught to do the same. These tests pin that Home is not an exception, and that a skin with no band takes no room.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { skinAttributes, WHIMBREL } from "./skin";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");

describe("the header band on Home", () => {
  for (const [name, path] of [["the shell", "./shell/redesign/RedesignedShell.tsx"]] as const) {
    it(`is drawn by ${name} once, above the tab's content, whatever the tab`, () => {
      const source = read(path);
      expect(source.match(/<SkinHeader \/>/g)).toHaveLength(1);
      const stage = source.indexOf('<div className="stage">');
      const header = source.indexOf("<SkinHeader />");
      expect(stage).toBeGreaterThan(-1);
      expect(header).toBeGreaterThan(stage);
      // Not inside a condition on the tab: the line before it is the stage's opening tag, and nothing between them tests a tab id.
      expect(source.slice(stage, header)).not.toMatch(/tab\.id|=== "today"|!== "today"/);
    });
  }

  it("is the same element the other tabs draw, aria-hidden and decorative", () => {
    const source = read("./SkinIcon.tsx");
    expect(source).toContain('<div className="skin-band" aria-hidden="true" />');
  });

  it("sets data-header-band from the skin, so Home's band follows the skin", () => {
    expect(skinAttributes({ ...WHIMBREL.skin, decoration: { ...WHIMBREL.skin.decoration, headerBand: "block" } })["data-header-band"]).toBe("block");
    expect(skinAttributes({ ...WHIMBREL.skin, decoration: { ...WHIMBREL.skin.decoration, headerBand: "rule" } })["data-header-band"]).toBe("rule");
  });

  it("takes no room when the skin has no band (Thread, High Contrast), and its stated height otherwise", () => {
    const css = read("./decoration.css");
    expect(css).toMatch(/\.skin-band\s*\{\s*display:\s*none;\s*\}/);
    expect(css).toMatch(/\[data-header-band="rule"\] \.skin-band \{\s*height: 2px;/);
    expect(css).toMatch(/\[data-header-band="block"\] \.skin-band \{\s*height: 12px;/);
    expect(css).toMatch(/\[data-header-band="stripes-diagonal"\] \.skin-band \{\s*height: 8px;/);
    expect(WHIMBREL.skin.decoration.headerBand).toBe("none");
  });
});
