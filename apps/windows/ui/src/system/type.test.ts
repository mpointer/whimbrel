import { describe, expect, it } from "vitest";
import { THREAD, skinVariables, type SkinManifest } from "../skin";

// The type roles of Thread (docs/superpowers/plans/2026-11-25-richer-skins.md, section 1): every text role is drawn
// in one of four faces (display, body, numerals, label), and each face takes the skin's family, weight and, for
// display and label, italic. These tests read the CSS itself, resolve its var()s the way a browser does, and check
// (1) which face each role maps to, (2) that a skin changes what the plan says it changes, and (3) that the Thread
// skin resolves to exactly what these rules drew before they read the skin.

const FILES = {
  system: (await import("./system.css?raw")).default,
  settings: (await import("./settings.css?raw")).default,
  restyle: (await import("./restyle.css?raw")).default,
  home: (await import("../today/redesign/home.css?raw")).default,
  shell: (await import("../shell/redesign/shell.css?raw")).default,
  liveMeeting: (await import("../meetings/redesign/liveMeeting.css?raw")).default,
  find: (await import("../library/redesign/find.css?raw")).default,
  conversations: (await import("../conversations/conversations.css?raw")).default,
};
const generated = (await import("./tokens.generated.css?raw")).default;

type Decls = Record<string, string>;

/** Every `selector { declarations }` in a file, last declaration wins, comments dropped. */
function rules(css: string): Map<string, Decls> {
  const out = new Map<string, Decls>();
  for (const m of css.replace(/\/\*[\s\S]*?\*\//g, "").replace(/@import[^;]*;/g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const selector = m[1].trim().replace(/\s+/g, " ");
    if (selector.startsWith("@")) continue;
    const decls: Decls = out.get(selector) ?? {};
    for (const d of m[2].split(";")) {
      const at = d.indexOf(":");
      if (at > 0) decls[d.slice(0, at).trim()] = d.slice(at + 1).trim();
    }
    out.set(selector, decls);
  }
  return out;
}

const parsed = Object.fromEntries(Object.entries(FILES).map(([name, css]) => [name, rules(css)])) as Record<keyof typeof FILES, Map<string, Decls>>;

function decl(file: keyof typeof FILES, selector: string, prop: string): string {
  const value = parsed[file].get(selector)?.[prop];
  if (value === undefined) throw new Error(`${file}: "${selector}" has no ${prop}`);
  return value;
}

/** The variables a window has: the design tokens, the app's `:root` aliases, the skin's, and what Thread pins. */
function variables(skin: SkinManifest, thread: boolean): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const m of generated.matchAll(/(--w-[a-z0-9-]+)\s*:\s*([^;]+);/g)) vars[m[1]] ??= m[2].trim();
  for (const [name, value] of Object.entries(parsed.system.get(":root") ?? {})) if (name.startsWith("--")) vars[name] = value;
  Object.assign(vars, skinVariables(skin));
  if (thread) {
    const pin = parsed.system.get(':root[data-skin="thread"], .w-thread') ?? parsed.system.get(':root[data-skin="thread"],.w-thread');
    Object.assign(vars, pin);
  }
  return vars;
}

/** `var(--a, fallback)` resolved against `vars`, innermost first. */
function resolve(value: string, vars: Record<string, string>): string {
  let out = value;
  for (let i = 0; i < 12 && out.includes("var("); i++) {
    out = out.replace(/var\(\s*(--[a-z0-9-]+)\s*(?:,\s*([^()]*(?:\([^()]*\)[^()]*)*))?\)/gi, (_all, name: string, fallback?: string) => vars[name] ?? fallback ?? "");
  }
  return out.trim();
}

const skin = (typography: Partial<SkinManifest["typography"]>): SkinManifest => ({ ...THREAD.skin, typography: { ...THREAD.skin.typography, ...typography } });

/** Display roles. */
const DISPLAY: [keyof typeof FILES, string][] = [
  ["system", ".w-flow-title"],
  ["system", ".w-dialog-title"],
  ["system", ".w-win-title"],
  ["settings", ".w-scard-title"],
  ["restyle", ".w-shell .rt .display"],
  ["home", ".th-title"],
  ["shell", ".w-shell-title h1"],
];
/** Section labels. */
const SECTION_LABELS: [keyof typeof FILES, string][] = [
  ["system", ".w-label"],
  ["restyle", ".w-shell .rt .section-label"],
  ["restyle", ".w-shell .rt .day"],
];
/** Numerals: the time line's times, the scoreboard digits, the marks' times. */
const NUMERALS: [keyof typeof FILES, string][] = [
  ["system", ".w-spine-time"],
  ["system", ".w-next-digits"],
  ["liveMeeting", ".th-marks-time"],
];
/** Strong text: card headings, counts, badges, the primary link. */
const STRONG: [keyof typeof FILES, string][] = [
  ["system", ".w-badge"],
  ["system", ".w-meeting-title"],
  ["system", ".w-source-name"],
  ["system", ".w-next-title"],
  ["system", ".w-button-link.w-button-primary"],
  ["restyle", ".w-shell .rt .topic-row.topic-row--active .row-title"],
  ["home", '.th-menu .w-menu-item[aria-checked="true"]'],
  ["liveMeeting", ".th-marks-head"],
  ["find", ".th-find-speaker"],
];
/** Captions and small meta text. */
const CAPTIONS: [keyof typeof FILES, string][] = [
  ["system", ".w-spine-sub"],
  ["restyle", ".w-shell .rt .small"],
];

describe("each Thread text role maps to a face", () => {
  it("draws display roles in the display face, weight and italic", () => {
    for (const [file, selector] of DISPLAY) {
      expect(decl(file, selector, "font-family"), selector).toMatch(/--sx-font-title/);
      expect(decl(file, selector, "font-weight"), selector).toMatch(/var\(--weight-display/);
      expect(decl(file, selector, "font-style"), selector).toMatch(/var\(--style-display/);
    }
    expect(parsed.system.get(":root")?.["--sx-font-title"]).toBe("var(--font-display, var(--w-font-ui))");
  });

  it("draws section labels in the label face, weight, italic and caps", () => {
    for (const [file, selector] of SECTION_LABELS) {
      expect(decl(file, selector, "font-family"), selector).toMatch(/--sx-font-label/);
      expect(decl(file, selector, "font-weight"), selector).toMatch(/var\(--weight-section-label, var\(--weight-label/);
      expect(decl(file, selector, "font-style"), selector).toMatch(/var\(--style-label/);
      expect(decl(file, selector, "text-transform"), selector).toMatch(/var\(--caps/);
      expect(decl(file, selector, "font-variant-caps"), selector).toMatch(/var\(--caps-variant/);
    }
    expect(parsed.system.get(":root")?.["--sx-font-label"]).toBe("var(--font-label, var(--sx-font-ui))");
  });

  it("draws captions and small meta text in the label face and its weight", () => {
    for (const [file, selector] of CAPTIONS) {
      expect(decl(file, selector, "font-family"), selector).toMatch(/--sx-font-label/);
      expect(decl(file, selector, "font-style"), selector).toMatch(/var\(--style-label/);
    }
    expect(decl("system", ".w-spine-sub", "font-weight")).toMatch(/var\(--weight-label/);
  });

  it("draws numerals in the numerals face and its weight, with tabular figures, and never italic", () => {
    for (const [file, selector] of NUMERALS) {
      expect(decl(file, selector, "font-weight"), selector).toMatch(/var\(--weight-numerals/);
    }
    for (const selector of [".w-spine-time", ".w-next-digits"]) {
      expect(decl("system", selector, "font-family"), selector).toMatch(/--sx-font-numerals/);
      expect(decl("system", selector, "font-variant-numeric"), selector).toBe("tabular-nums");
    }
    expect(decl("liveMeeting", ".th-marks-time", "font-family")).toMatch(/--sx-font-numerals/);
    expect(decl("liveMeeting", ".th-marks-time", "font-variant-numeric")).toBe("tabular-nums");
  });

  it("draws body emphasis from one variable, and the body root from the body weight", () => {
    expect(decl("system", ".w-button", "font-weight")).toBe("var(--weight-emphasis, 600)");
    expect(decl("settings", ".w-skin-name", "font-weight")).toBe("var(--weight-emphasis, 600)");
    expect(decl("restyle", ".w-shell .rt .row-title", "font-weight")).toBe("var(--weight-emphasis, 600)");
    expect(decl("system", ".w-sys", "font-weight")).toBe("var(--weight-body, 400)");
    expect(decl("restyle", ".w-shell .rt", "font-weight")).toBe("var(--weight-body, 400)");
  });

  it("draws strong text from one variable that follows emphasis, with Thread pinned at 700", () => {
    for (const [file, selector] of STRONG) expect(decl(file, selector, "font-weight"), selector).toBe("var(--weight-strong, var(--weight-emphasis, 700))");
    expect(variables(skin({ body: "SF Pro/regular" }), true)["--weight-strong"]).toBe("700");
    // A skin that is not Thread has no pin, so strong text takes its emphasis: body plus two steps, capped at 800.
    const strong = (body: string) => resolve(decl("system", ".w-badge", "font-weight"), variables(skin({ body }), false));
    expect(strong("SF Pro/regular")).toBe("600");
    expect(strong("SF Pro/medium")).toBe("700");
    expect(strong("SF Pro/semibold")).toBe("800");
  });

  it("ignores body and numerals italic in Thread: no rule reads --style-body or --style-numerals", () => {
    for (const [name, css] of Object.entries(FILES)) {
      expect(css, name).not.toMatch(/--style-body|--style-numerals/);
    }
  });

  it("leaves the recording indicator, the capture and consent controls and window chrome on their own type", () => {
    for (const selector of [".w-rec-word", ".w-rec-time", ".w-rec-reason", ".w-capture-button", ".w-consent-text", ".w-consent-copied"]) {
      const weight = parsed.system.get(selector)?.["font-weight"];
      expect(weight, selector).toMatch(/^\d+$/);
    }
    for (const selector of [".w-win-head", ".w-win-strong"]) expect(parsed.system.get(selector)?.["font-weight"], selector).toMatch(/^\d+$/);
  });
});

describe("a skin's type reaches those roles", () => {
  const heavySerif = skin({ display: "Georgia/black/italic", numerals: "Courier New/bold", label: "Georgia/heavy/italic", body: "Georgia/semibold/italic", caps: "upper" });
  const vars = variables(heavySerif, false);
  const css = (file: keyof typeof FILES, selector: string, prop: string) => resolve(decl(file, selector, prop), vars);

  it("gives a display role the skin's family, weight and italic", () => {
    for (const [file, selector] of DISPLAY) {
      expect(css(file, selector, "font-weight"), selector).toBe("900");
      expect(css(file, selector, "font-style"), selector).toBe("italic");
      expect(css(file, selector, "font-family"), selector).toBe("Georgia, serif");
    }
  });

  it("gives a section label the label weight as set, italic and caps", () => {
    for (const [file, selector] of SECTION_LABELS) {
      expect(css(file, selector, "font-weight"), selector).toBe("800");
      expect(css(file, selector, "font-style"), selector).toBe("italic");
      expect(css(file, selector, "text-transform"), selector).toBe("uppercase");
      expect(css(file, selector, "font-variant-caps"), selector).toBe("normal");
      expect(css(file, selector, "font-family"), selector).toBe("Georgia, serif");
    }
  });

  it("gives numerals the numerals weight as set and keeps the figures tabular, not italic", () => {
    for (const [file, selector] of NUMERALS) expect(css(file, selector, "font-weight"), selector).toBe("700");
    expect(css("system", ".w-spine-time", "font-family")).toBe('"Courier New", monospace');
    expect(css("system", ".w-spine-time", "font-variant-numeric")).toBe("tabular-nums");
    expect(parsed.system.get(".w-spine-time")?.["font-style"]).toBeUndefined();
  });

  it("gives emphasis two steps over the body weight, never past 800", () => {
    expect(css("system", ".w-button", "font-weight")).toBe("800");
    const regular = variables(skin({ body: "SF Pro/regular" }), false);
    expect(resolve(decl("system", ".w-button", "font-weight"), regular)).toBe("600");
    const black = variables(skin({ body: "SF Pro/black" }), false);
    expect(resolve(decl("system", ".w-button", "font-weight"), black)).toBe("900");
    expect(resolve(decl("system", ".w-sys", "font-weight"), regular)).toBe("400");
  });

  it("does not italicize body text or numerals, whatever the skin says", () => {
    expect(vars["--style-body"]).toBe("italic");
    expect(parsed.system.get(".w-sys")?.["font-style"]).toBeUndefined();
    expect(parsed.system.get(".w-button")?.["font-style"]).toBeUndefined();
  });
});

describe("section label caps", () => {
  const label = (typography: Partial<SkinManifest["typography"]>, extra: string, thread = false) => {
    const vars = variables(skin(typography), thread);
    const rule = parsed.system.get(".w-label")!;
    const variant = parsed.system.get(extra) ?? {};
    const merged = { ...rule, ...variant };
    return { transform: resolve(merged["text-transform"]!, vars), variant: resolve(merged["font-variant-caps"]!, vars) };
  };

  it("draws a plain label upper case when the skin's caps is upper", () => {
    expect(label({ caps: "upper" }, ".w-label-plain")).toEqual({ transform: "uppercase", variant: "normal" });
  });
  it("draws a plain label in small caps when the skin's caps is small", () => {
    expect(label({ caps: "small" }, ".w-label-plain")).toEqual({ transform: "none", variant: "small-caps" });
  });
  it("draws a plain label as written when caps is none", () => {
    expect(label({ caps: "none" }, ".w-label-plain")).toEqual({ transform: "none", variant: "normal" });
  });
  it("lets the Labels element decide the shape: small caps beats the skin's upper, brackets beat both", () => {
    expect(label({ caps: "upper" }, ".w-label-smallCaps")).toEqual({ transform: "none", variant: "all-small-caps" });
    expect(label({ caps: "small" }, ".w-label-smallCaps")).toEqual({ transform: "none", variant: "all-small-caps" });
    expect(label({ caps: "upper" }, ".w-label-bracketed")).toEqual({ transform: "none", variant: "normal" });
    expect(label({ caps: "small" }, ".w-label-bracketed")).toEqual({ transform: "none", variant: "normal" });
  });
});

/**
 * What these rules drew before they read the skin, on origin/main (0.9.83). Thread is skins/thread.json: display and
 * numerals semibold, body and label regular, caps none, one family. Every row must resolve to the same thing.
 */
describe("the Thread skin draws as it did before", () => {
  const vars = variables(THREAD.skin, true);
  const SEGOE = vars["--font-body"];
  const draw = (file: keyof typeof FILES, selector: string, prop: string) => resolve(decl(file, selector, prop), vars);

  it("has the values the table below assumes", () => {
    expect(THREAD.skin.typography).toEqual({ display: "SF Pro/semibold", body: "SF Pro/regular", numerals: "SF Pro/semibold", label: "SF Pro/regular", caps: "none" });
  });

  const BEFORE: [keyof typeof FILES, string, Record<string, string>][] = [
    ["system", ".w-flow-title", { "font-weight": "600", "font-style": "normal" }],
    ["system", ".w-dialog-title", { "font-weight": "600", "font-style": "normal" }],
    ["system", ".w-win-title", { "font-weight": "600", "font-style": "normal" }],
    ["settings", ".w-scard-title", { "font-weight": "600", "font-style": "normal" }],
    ["restyle", ".w-shell .rt .display", { "font-weight": "600", "font-style": "normal" }],
    ["home", ".th-title", { "font-weight": "600" }],
    ["shell", ".w-shell-title h1", { "font-weight": "600" }],
    ["system", ".w-spine-time", { "font-weight": "600" }],
    ["liveMeeting", ".th-marks-time", { "font-weight": "600" }],
    ["system", ".w-label", { "font-weight": "600", "font-style": "normal", "text-transform": "none", "font-variant-caps": "normal" }],
    ["restyle", ".w-shell .rt .section-label", { "font-weight": "600", "font-style": "normal", "text-transform": "none", "font-variant-caps": "normal" }],
    ["restyle", ".w-shell .rt .day", { "font-weight": "600", "font-style": "normal", "text-transform": "none", "font-variant-caps": "normal" }],
    ["system", ".w-spine-sub", { "font-weight": "400", "font-style": "normal" }],
    ["restyle", ".w-shell .rt .small", { "font-style": "normal" }],
    ["system", ".w-sys", { "font-weight": "400" }],
    ["restyle", ".w-shell .rt", { "font-weight": "400" }],
    ["system", ".w-button", { "font-weight": "600" }],
    ["system", ".w-badge", { "font-weight": "700" }],
    ["system", ".w-meeting-title", { "font-weight": "700" }],
    ["system", ".w-source-name", { "font-weight": "700" }],
    ["system", ".w-next-title", { "font-weight": "700" }],
    ["system", ".w-button-link.w-button-primary", { "font-weight": "700" }],
    ["restyle", ".w-shell .rt .topic-row.topic-row--active .row-title", { "font-weight": "700" }],
    ["home", '.th-menu .w-menu-item[aria-checked="true"]', { "font-weight": "700" }],
    ["liveMeeting", ".th-marks-head", { "font-weight": "700" }],
    ["find", ".th-find-speaker", { "font-weight": "700" }],
    ["system", ".w-field-label", { "font-weight": "600" }],
    ["settings", ".w-status", { "font-weight": "600" }],
    ["settings", ".w-skin-name", { "font-weight": "600" }],
    ["restyle", ".w-shell .rt .row-title", { "font-weight": "600" }],
    ["restyle", ".w-shell .rt .meeting-row__title", { "font-weight": "600" }],
    ["home", ".th-list-title", { "font-weight": "600" }],
    ["conversations", ".th-cv-title", { "font-weight": "600" }],
  ];
  for (const [file, selector, expected] of BEFORE) {
    it(`${selector}`, () => {
      for (const [prop, value] of Object.entries(expected)) expect(draw(file, selector, prop), `${selector} ${prop}`).toBe(value);
    });
  }

  it("keeps every face on the system family", () => {
    for (const [file, selector] of [...DISPLAY, ...SECTION_LABELS, ...CAPTIONS]) expect(draw(file, selector, "font-family"), selector).toBe(SEGOE);
    for (const [file, selector] of NUMERALS) expect(draw(file, selector, "font-family"), selector).toBe(SEGOE);
  });

  it("does not use the scoreboard, the one rule whose literal weight (700) is not Thread's numerals weight (600)", () => {
    expect(THREAD.skin.today.options.nextMeeting).toBe("card");
    expect(draw("system", ".w-next-digits", "font-weight")).toBe("600");
  });
});
