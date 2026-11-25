import { describe, expect, it } from "vitest";
import tokensText from "../../../../../design/tokens/tokens.json?raw";
import generatedCss from "./tokens.generated.css?raw";
import systemCss from "./system.css?raw";
import settingsCss from "./settings.css?raw";

// The default control border (#d5dbe3 on white, 1.39:1) is a hairline between cards, not a boundary a person can find
// an input by. WCAG 1.4.11 asks 3:1 for it. The control border is its own Thread token, so the card hairline stays.

const T = JSON.parse(tokensText) as {
  palette: Record<string, Record<string, string>>;
  threadOnly: { controlBorder: { $value: string[] } };
};
const MODES = ["light", "dark", "highContrastLight", "highContrastDark"] as const;
const lum = (hex: string) => {
  const n = parseInt(hex.slice(1), 16);
  const ch = (v: number) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return 0.2126 * ch((n >> 16) & 255) + 0.7152 * ch((n >> 8) & 255) + 0.0722 * ch(n & 255);
};
const ratio = (a: string, b: string) => (Math.max(lum(a), lum(b)) + 0.05) / (Math.min(lum(a), lum(b)) + 0.05);
const token = (mode: (typeof MODES)[number]) => T.threadOnly.controlBorder.$value[MODES.indexOf(mode)];

describe("the Thread control border token", () => {
  it.each(MODES)("%s: at least 3:1 on the card and on the page", (mode) => {
    for (const surface of ["backgroundCard", "backgroundPage"]) {
      const r = ratio(token(mode), T.palette[mode][surface]);
      expect(r, `${mode} ${token(mode)} on ${surface}`).toBeGreaterThanOrEqual(3);
    }
  });

  it("is a new token: the card hairline (border) keeps its values", () => {
    expect(T.palette.light.border.toLowerCase()).toBe("#d5dbe3");
    expect(T.palette.dark.border.toLowerCase()).toBe("#2b3745");
    expect(Object.keys(T.palette.light)).toHaveLength(15);
  });

  it("is written for Thread's palette only (the .w-thread class, or data-palette=thread on the root, which is how a skin applies it), never on :root alone", () => {
    const at = [...generatedCss.matchAll(/--w-control-border:/g)].map((m) => m.index!);
    // The class: 7 palette blocks. The attribute: light and dark, and high contrast light and dark.
    expect(at).toHaveLength(11);
    for (const i of at) {
      const selector = generatedCss.slice(generatedCss.lastIndexOf("}", i) + 1, i);
      expect(selector, selector).toMatch(/w-thread|data-palette="thread"/);
    }
    for (const mode of MODES) expect(generatedCss.toLowerCase()).toContain(`--w-control-border: ${token(mode).toLowerCase()};`);
    expect(generatedCss).toMatch(/:root\[data-palette="thread"\]\[data-theme="light"\]\s*\{[^}]*--w-control-border: #7e8ca0/);
    expect(generatedCss).toMatch(/:root\[data-palette="thread"\]\[data-theme="dark"\]\s*\{[^}]*--w-control-border: #5f6d7e/);
  });
});

describe("the control border in the components", () => {
  it("--sx-control-line falls back to the skin's own border, so a skin without the token is unchanged", () => {
    expect(systemCss).toMatch(/--sx-control-line:\s*var\(--sx-border-width\) var\(--sx-border-style\) var\(--w-control-border, var\(--color-border\)\)/);
  });

  const ruleFor = (css: string, selector: RegExp) => [...css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter((m) => selector.test(m[1].trim())).map((m) => m[2]).join("\n");

  it("inputs, selects and number boxes draw it everywhere (form controls, WCAG 1.4.11)", () => {
    expect(ruleFor(systemCss, /^\.w-input$/)).toMatch(/border:\s*var\(--sx-control-line\)/);
  });

  it("the segmented choice and a secondary button draw it only inside the redesigned Settings frame, so no other screen's picture changes", () => {
    expect(ruleFor(systemCss, /^\.w-segmented$/)).toMatch(/border:\s*var\(--sx-line\)/);
    expect(ruleFor(systemCss, /^\.w-button$/)).toMatch(/border:\s*var\(--sx-line\)/);
    expect(ruleFor(settingsCss, /^\.rs-frame \.w-segmented$/)).toMatch(/border:\s*var\(--sx-control-line\)/);
    const secondary = [...settingsCss.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter((m) => m[1].trim().startsWith(".rs-frame .w-button:not("));
    expect(secondary).toHaveLength(1);
    expect(secondary[0][2]).toMatch(/border:\s*var\(--sx-control-line\)/);
    for (const kind of ["primary", "quiet", "destructive"]) expect(secondary[0][1]).toContain(`.w-button-${kind}`);
  });

  it("the other button kinds keep their own border color", () => {
    for (const cls of ["primary", "destructive"]) expect(ruleFor(systemCss, new RegExp(`^\\.w-button-${cls}$`))).toMatch(/border-color:/);
    expect(ruleFor(systemCss, /^\.w-button-quiet$/)).toMatch(/border-color:\s*transparent/);
  });
});
void settingsCss;

describe("forced colours: a disabled control is clearly not an enabled one", () => {
  const forced = [...settingsCss.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/@media \(forced-colors: active\) \{([\s\S]*?)\n\}/g)].map((m) => m[1]).join("\n");
  const rule = (selector: string) => [...forced.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter((m) => m[1].split(",").map((x) => x.trim()).includes(selector)).map((m) => m[2]).join("\n");

  it.each([".w-button:disabled", ".w-input:disabled", "select:disabled", ".w-switch:disabled", ".w-segment:disabled"])(
    "%s has GrayText text and border",
    (selector) => {
      const body = rule(selector);
      expect(body, selector).toMatch(/color:\s*GrayText/);
      expect(body, selector).toMatch(/border-color:\s*GrayText/);
    },
  );

  it("enabled inputs, selects and buttons use ButtonText or CanvasText borders", () => {
    expect(rule(".w-input:enabled")).toMatch(/border-color:\s*(ButtonText|CanvasText)/);
    expect(rule(".w-button:enabled")).toMatch(/border-color:\s*(ButtonText|CanvasText)/);
  });
});
