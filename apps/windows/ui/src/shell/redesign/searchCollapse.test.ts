// At a large text size the header row wraps onto two rows because the search field sits on its own. In a middle width (a container
// of 40 to 50 rem: the 734 x 481 window at Windows text 150%) the field collapses to an icon button beside the title and the Memory
// state, so one row holds all three, and opens on activation or Ctrl+K. These are the rules and the text checks that keep it so.

import { describe, expect, it } from "vitest";
import header from "./HeaderRow.tsx?raw";
import shell from "./RedesignedShell.tsx?raw";
import css from "./shell.css?raw";
import { searchShown } from "./searchCollapse";

describe("when the collapsed search field is drawn open", () => {
  it("is closed until it is asked for", () => {
    expect(searchShown({ asked: false, text: "", findShown: false })).toBe(false);
  });
  it("opens when asked for (the icon, Ctrl+K)", () => {
    expect(searchShown({ asked: true, text: "", findShown: false })).toBe(true);
  });
  it("stays open while it holds words, so a query is never hidden", () => {
    expect(searchShown({ asked: false, text: "Northwind", findShown: false })).toBe(true);
    expect(searchShown({ asked: false, text: "   ", findShown: false })).toBe(false);
  });
  it("is open on the Find page, which is where its words go", () => {
    expect(searchShown({ asked: false, text: "", findShown: true })).toBe(true);
  });
});

describe("the header at a middle width", () => {
  it("keeps the field's name on an icon button that says whether the field is open", () => {
    expect(header).toContain("w-shell-search-toggle");
    expect(header).toMatch(/<WIconButton[^>]*icon="search"[^>]*label=\{t\("Search your work memory"\)\}/);
    expect(header).toContain("aria-expanded={shown}");
  });
  it("collapses only in the middle band: wider than the stacked layout (40rem) and not wider than 50rem; the 900 px window keeps today's rows", () => {
    expect(css).toMatch(/@container shell \(min-width: 40\.01rem\) and \(max-width: 50rem\) \{[\s\S]*?\.w-shell-search-toggle \{[^}]*display:\s*inline-grid/);
    expect(css).toMatch(/\.w-shell-search-toggle \{[^}]*display:\s*none/);
    expect(css).toMatch(/\.w-shell-header-row\[data-search="closed"\] \.w-shell-search \{[^}]*display:\s*none/);
  });
  it("puts the title, the icon and the Memory state on one row", () => {
    expect(css).toMatch(/@container shell \(min-width: 40\.01rem\) and \(max-width: 50rem\) \{[\s\S]*?\.w-shell-header-row \{[^}]*grid-template-columns:\s*minmax\(0, 1fr\) auto auto/);
  });
  it("Ctrl+K opens the field before it focuses it", () => {
    expect(shell).toContain("focusTick");
    expect(header).toContain("p.focusTick");
  });
});
