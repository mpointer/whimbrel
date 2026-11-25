// A finished note with nothing for the right margin (no transcript, no summary, so no sources and no Whimbrel line) folds the
// column away on a wide window: the note takes the width, centred under the usual maximum. Text checks: there is no layout engine here.

import { describe, expect, it } from "vitest";
import css from "./finishedNote.css?raw";
import source from "./FinishedNote.tsx?raw";

const block = (selector: string): string => {
  const at = css.indexOf(`${selector} {`);
  return at < 0 ? "" : css.slice(at, css.indexOf("}", at));
};

describe("the note with an empty margin", () => {
  it("is told by the page: has-margin follows marginShown, not only the sources", () => {
    expect(source).toContain("marginShown(model, aiWrote)");
    expect(source).toContain("holdMargin(marginHeld.current, editing, marginShown(model, aiWrote))");
    expect(source).toMatch(/data-has-margin=\{margin\.shown \? "true" : "false"\}/);
  });

  it("keeps one column and no rule between columns at the wide size", () => {
    expect(css).toMatch(/@container th-fin \(min-width: 56rem\) \{[\s\S]*\.th-fin-page\[data-has-margin="false"\] \.th-fin-row \{[^}]*grid-template-columns:\s*minmax\(0, 1fr\)/);
    expect(block('.th-fin-page[data-has-margin="false"]::after')).toMatch(/display:\s*none/);
    expect(block('.th-fin-page[data-has-margin="false"] .th-fin-notices')).toMatch(/padding-right:\s*var\(--w-space-xl\)/);
  });

  it("centres the note under the width the note column had (the screen's maximum less the margin and one gap)", () => {
    const rule = block('.th-fin-page[data-has-margin="false"]');
    expect(rule).toMatch(/max-width:\s*calc\(78rem - var\(--th-fin-margin\) - var\(--w-space-xl\)\)/);
    expect(rule).toMatch(/margin-inline:\s*auto/);
  });
});
