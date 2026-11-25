// The Find page's layout rules that a hand test on a 900x640 window caught, kept here as text checks: there is no layout engine in
// these tests.

import { describe, expect, it } from "vitest";
import find from "./find.css?raw";
import shell from "../../shell/redesign/shell.css?raw";
import home from "../../today/redesign/home.css?raw";
import filters from "../../today/redesign/HomeFilters.tsx?raw";

const rule = (css: string, selector: string): string => {
  const at = css.indexOf(`${selector} {`);
  return at < 0 ? "" : css.slice(at, css.indexOf("}", at));
};

describe("the Find page's layout", () => {
  it("keeps what is positioned inside the content area inside it, so the window never scrolls beside it", () => {
    expect(rule(shell, ".w-shell .w-content")).toMatch(/position:\s*relative/);
  });
  it("puts the conversations under the page when both do not fit, instead of squeezing the titles", () => {
    expect(rule(find, ".th-find-wrap")).toMatch(/flex-wrap:\s*wrap/);
    // A container query cannot change the container itself: the old rule that tried to never applied.
    expect(find).not.toMatch(/@container th-find[^{]*\{\s*\.th-find-wrap\s*\{/);
    expect(rule(find, ".th-find-side")).toMatch(/flex:\s*1 1 20rem/);
  });
  it("never cuts the Dates menu, so Apply is always in view", () => {
    expect(rule(home, ".th-menu.th-menu-dates")).toMatch(/max-height:\s*none/);
    expect(filters).toContain('className="th-menu th-menu-dates"');
  });
});
