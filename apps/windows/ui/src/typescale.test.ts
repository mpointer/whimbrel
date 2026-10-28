import { describe, expect, it } from "vitest";

// The type scale lives in tokens.css (the Mac's, docs/superpowers/specs/2026-10-16-mac-today-refresh-design.md).
// These guards keep app.css on it, so a size change is made once, in the token.

const tokensCss = (await import("./tokens.css?raw")).default;
const appCss = (await import("./app.css?raw")).default;

/** Rules whose size is deliberately off the body scale: the brand mark, the page title, the popup and alert windows. */
const OFF_SCALE = new Set([".brand", ".display", ".popup-brand", ".alert-title"]);

/** Every `selector { … font-size: Npx … }` in app.css, with the literal size. */
function literalSizes(css: string): { selector: string; px: number }[] {
  const out: { selector: string; px: number }[] = [];
  for (const m of css.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
    const size = /font-size:\s*(\d+(?:\.\d+)?)px/.exec(m[2]);
    if (size) out.push({ selector: m[1].trim(), px: Number(size[1]) });
  }
  return out;
}

describe("the type scale", () => {
  it("defines every size token app.css uses", () => {
    const defined = new Set([...tokensCss.matchAll(/(--text-[a-z-]+)\s*:/g)].map((m) => m[1]));
    const used = [...appCss.matchAll(/var\((--text-[a-z-]+)/g)].map((m) => m[1]);
    expect(used.length).toBeGreaterThan(5);
    expect(used.filter((name) => !defined.has(name))).toEqual([]);
  });

  it("keeps everything readable at 11px or more", () => {
    for (const m of tokensCss.matchAll(/(--text-[a-z-]+):\s*(\d+)px/g)) {
      expect(Number(m[2]), m[1]).toBeGreaterThanOrEqual(11);
    }
    expect(literalSizes(appCss).filter((r) => r.px < 11)).toEqual([]);
  });

  it("uses a size token, not a literal, outside the few off-scale rules", () => {
    const literal = literalSizes(appCss).filter((r) => !OFF_SCALE.has(r.selector));
    expect(literal, "use a var(--text-*) token, or add the rule to OFF_SCALE with a reason").toEqual([]);
  });

  it("gives the reading column one width token", () => {
    expect(tokensCss).toMatch(/--content-width:\s*760px/);
    expect(appCss).toMatch(/\.today\s*\{[^}]*max-width:\s*var\(--content-width\)/);
    expect(appCss).toMatch(/\.detail\s*\{[^}]*max-width:\s*var\(--content-width\)/);
  });

  it("draws cards flat: the skin's border (a hairline in Whimbrel) and no shadow", () => {
    const card = /\n\.card\s*\{([^}]*)\}/.exec(appCss)?.[1] ?? "";
    expect(card).toContain("border: var(--border-width) var(--border-style) var(--color-border)");
    expect(card).not.toContain("shadow");
    expect(tokensCss).toMatch(/--border-width:\s*1px;/);
    expect(tokensCss).toMatch(/--border-style:\s*solid;/);
  });
});
