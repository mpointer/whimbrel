import { describe, expect, it } from "vitest";

// Every color in the UI comes from a token in tokens.css, so a theme can restyle all of it
// (docs/superpowers/specs/2026-10-06-themes-design.md). These guards keep that true.

const raw = (glob: Record<string, unknown>) => glob as Record<string, string>;

const tokensCss = (await import("./tokens.css?raw")).default;
const appCss = (await import("./app.css?raw")).default;
const sources = {
  "app.css": appCss,
  ...Object.fromEntries(
    Object.entries(raw(import.meta.glob("./**/*.tsx", { query: "?raw", import: "default", eager: true }))).map(
      ([path, text]) => [path.replace("./", ""), text],
    ),
  ),
};
const defaultTheme = Object.values(
  import.meta.glob("../../../../themes/default.json", { eager: true, import: "default" }),
)[0] as { light: Record<string, string>; dark: Record<string, string> };

/** `--color-text-on-accent` -> `textOnAccent`, the theme files' token names. */
const tokenName = (cssName: string) => cssName.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase());

/** The `--color-*: #rrggbb` declarations in a stretch of CSS. */
function palette(css: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of css.matchAll(/--color-([a-z-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)) out[tokenName(m[1])] = m[2].toLowerCase();
  return out;
}

const darkStart = tokensCss.indexOf("@media (prefers-color-scheme: dark)");
const lightTokens = palette(tokensCss.slice(0, darkStart));
const darkTokens = palette(tokensCss.slice(darkStart));

describe("colors come from tokens.css", () => {
  const RAW_COLOR = /#(?:[0-9a-f]{8}|[0-9a-f]{6}|[0-9a-f]{4}|[0-9a-f]{3})\b|\b(?:rgba?|hsla?)\(|\bcolor-mix\(/i;

  it("finds the stylesheets and components it is guarding", () => {
    expect(appCss.length).toBeGreaterThan(1000);
    expect(Object.keys(sources).filter((f) => f.endsWith(".tsx")).length).toBeGreaterThan(5);
  });

  it("uses no hex, rgb(), hsl() or color-mix() outside tokens.css", () => {
    const offenders: string[] = [];
    for (const [file, text] of Object.entries(sources)) {
      text.split(/\r?\n/).forEach((line, i) => {
        if (RAW_COLOR.test(line)) offenders.push(`${file}:${i + 1}: ${line.trim()}`);
      });
    }
    expect(offenders, "use a var(--color-…) token, or add one to tokens.css and themes/*.json").toEqual([]);
  });

  it("uses no var(--color-…) that tokens.css does not define", () => {
    const defined = new Set([...tokensCss.matchAll(/(--color-[a-z-]+)\s*:/g)].map((m) => m[1]));
    const undefinedUses: string[] = [];
    for (const [file, text] of Object.entries(sources)) {
      for (const m of text.matchAll(/var\((--color-[a-z-]+)/g)) {
        if (!defined.has(m[1])) undefinedUses.push(`${file}: ${m[1]}`);
      }
    }
    expect(undefinedUses).toEqual([]);
  });
});

describe("Default theme and tokens.css agree", () => {
  const tokens = Object.keys(defaultTheme.light);

  it("themes/default.json has the 15 tokens in both variants", () => {
    expect(tokens).toHaveLength(15);
    expect(Object.keys(defaultTheme.dark).sort()).toEqual([...tokens].sort());
  });

  it("matches the light values in :root", () => {
    const expected = Object.fromEntries(tokens.map((t) => [t, defaultTheme.light[t].toLowerCase()]));
    expect(lightTokens).toEqual(expected);
  });

  it("matches the dark values in the prefers-color-scheme block", () => {
    const expected = Object.fromEntries(tokens.map((t) => [t, defaultTheme.dark[t].toLowerCase()]));
    expect(darkTokens).toEqual(expected);
  });
});
