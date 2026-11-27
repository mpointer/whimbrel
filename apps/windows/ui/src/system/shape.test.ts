// Richer skins, row 5: a double border on Windows. `border-style: double` needs three times the line width, so a padding next to a
// border of the skin's line gives back --border-trim, and the box keeps its size. CSS can get that wrong without a layout engine,
// so this reads the style sheets.

import { describe, expect, it } from "vitest";
import tokensCss from "../tokens.css?raw";
import systemCss from "./system.css?raw";

const sheets = import.meta.glob("../**/*.css", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const strip = (text: string) => text.replace(/\/\*[\s\S]*?\*\//g, "");

/** Every `{ ... }` block with no block inside it. */
function blocks(text: string): { selector: string; body: string }[] {
  return [...strip(text).matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((m) => ({ selector: m[1].trim(), body: m[2] }));
}
/** A value split at the spaces outside parentheses. */
function topLevel(value: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = "";
  for (const ch of value.trim()) {
    if (ch === "(") depth += 1;
    if (ch === ")") depth -= 1;
    if (/\s/.test(ch) && depth === 0) {
      if (cur) out.push(cur);
      cur = "";
    } else cur += ch;
  }
  if (cur) out.push(cur);
  return out;
}
const ALL_SIDES = /(?<![-\w])border:\s*var\(--(?:sx|rt)-(?:control-)?line\)/;
const PADDING = /(?<![-\w])padding:\s*([^;]+);/;

describe("a double border on Windows", () => {
  it("has a surface radius that follows the skin and a trim that defaults to nothing", () => {
    expect(tokensCss).toMatch(/--radius-surface:\s*16px/);
    expect(tokensCss).not.toMatch(/--radius-surface:\s*18px/);
    expect(tokensCss).toMatch(/--border-trim:\s*0px/);
    expect(systemCss).toMatch(/--sx-border-trim:\s*var\(--border-trim,\s*0px\)/);
  });

  it("gives every padding next to a full border of the skin's line back the trim", () => {
    const missing: string[] = [];
    let checked = 0;
    for (const [file, text] of Object.entries(sheets)) {
      if (file.includes("gallery") || file.includes("tokens.generated")) continue;
      for (const { selector, body } of blocks(text)) {
        const pad = PADDING.exec(body);
        if (!ALL_SIDES.test(body) || !pad) continue;
        checked += 1;
        // each non-zero side reads the trim
        const sides = topLevel(pad[1]);
        for (const side of sides) {
          if (side === "0" || side === "0px") continue;
          if (!side.includes("--sx-border-trim")) missing.push(`${file} ${selector}: ${side}`);
        }
      }
    }
    expect(checked).toBeGreaterThan(40);
    expect(missing).toEqual([]);
  });

  it("changes no padding while the trim is zero", () => {
    // every trimmed side is `max(0px, calc(X - var(--sx-border-trim)))`, which is X while the trim is 0px and X is not negative
    const sample = /max\(0px, calc\(.+ - var\(--sx-border-trim\)\)\)/;
    const body = blocks(systemCss).find((b) => b.selector === ".w-rec-bar")?.body ?? "";
    expect(PADDING.exec(body)?.[1]).toMatch(sample);
  });
});
