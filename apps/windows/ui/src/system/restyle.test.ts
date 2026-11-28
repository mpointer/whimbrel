// Step 10, the restyle pass: Topics, Automations, the Meetings list and the capture and episode screens keep their layout and take the
// Thread look. These guard the two things CSS can get wrong without a layout engine: that the rules only reach the screens that
// carry the `rt` class inside the shell, and that every color pair the pass introduces meets its contrast rule (4.5:1 for text,
// 3:1 for large text, control edges, focus and state marks) in light, dark and high contrast.

import { describe, expect, it } from "vitest";
import tokensText from "../../../../../design/tokens/tokens.json?raw";
import css from "./restyle.css?raw";
import topics from "../topics/TopicsView.tsx?raw";
import automations from "../automations/AutomationsView.tsx?raw";
import meetings from "../meetings/MeetingsView.tsx?raw";
import recording from "../meetings/RecordingCard.tsx?raw";
import detail from "../library/Detail.tsx?raw";
import shell from "../shell/redesign/RedesignedShell.tsx?raw";
import tokensCss from "../tokens.css?raw";

const T = JSON.parse(tokensText) as {
  palette: Record<string, Record<string, string>>;
  threadOnly: { controlBorder: { $value: string[] } };
  fixed: Record<string, { $value: string[] }>;
};
const MODES = ["light", "dark", "highContrastLight", "highContrastDark"] as const;
type Mode = (typeof MODES)[number];

const strip = (text: string) => text.replace(/\/\*[\s\S]*?\*\//g, "");
/** A selector list split at its top-level commas (a comma inside :is( ) stays). */
function splitList(list: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let at = 0;
  for (let i = 0; i < list.length; i++) {
    if (list[i] === "(") depth++;
    else if (list[i] === ")") depth--;
    else if (list[i] === "," && depth === 0) {
      out.push(list.slice(at, i).trim());
      at = i + 1;
    }
  }
  out.push(list.slice(at).trim());
  return out.filter(Boolean);
}

/** Every rule's selector list and body, inside @media blocks too. */
function rules(text: string): { selectors: string[]; body: string }[] {
  const out: { selectors: string[]; body: string }[] = [];
  for (const m of strip(text).matchAll(/([^{}@]+)\{([^{}]*)\}/g)) out.push({ selectors: splitList(m[1]), body: m[2] });
  return out;
}

describe("the restyle stylesheet reaches only the screens that carry rt inside the shell", () => {
  it("has rules, and every selector starts with .w-shell .rt (the shell's class and the class the restyled screens carry)", () => {
    const all = rules(css);
    expect(all.length).toBeGreaterThan(60);
    const bad = all.flatMap((r) => r.selectors).filter((s) => !s.startsWith(".w-shell .rt"));
    expect(bad, bad.join(" | ")).toEqual([]);
  });

  it("uses no hex, rgb(), hsl() or color-mix(), and only color tokens that exist", () => {
    const text = strip(css);
    expect(text).not.toMatch(/#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?)\(|\bcolor-mix\(/);
    const defined = new Set([...tokensCss.matchAll(/(--color-[a-z-]+)\s*:/g)].map((m) => m[1]));
    const used = [...text.matchAll(/var\((--color-[a-z-]+)/g)].map((m) => m[1]);
    expect(used.length).toBeGreaterThan(20);
    expect(used.filter((name) => !defined.has(name))).toEqual([]);
  });

  it("uses none of the old palette's sizes or shapes (--text-*, --space-*, --radius*), no px type, and the Thread type roles (rem)", () => {
    const text = strip(css);
    expect(text).not.toMatch(/var\(--(?:text|space|radius)(?:-|\b)/);
    expect(text).not.toMatch(/font-size:\s*[\d.]+px/);
    for (const m of text.matchAll(/font-size:\s*([^;]+);/g)) expect(m[1], m[0]).toMatch(/^var\(--w-text-[a-z-]+-size\)$/);
  });

  it("draws the focus ring from the fixed focus token, and keeps it on forced colors", () => {
    expect(css).toMatch(/:focus-visible \{\s*outline: 2px solid var\(--w-color-focus\);/);
    expect(css).toMatch(/@media \(forced-colors: active\) \{[\s\S]*outline-color: Highlight/);
  });

  it("shows a chosen row by more than color: a border bar and heavier words, and forced colors keep the bar and add an outline", () => {
    expect(css).toMatch(/\.topic-row\.topic-row--active \{[^}]*border-inline-start-color/);
    expect(css).toMatch(/\.topic-row\.topic-row--active \.row-title \{[^}]*font-weight: var\(--weight-strong, var\(--weight-emphasis, 700\)\)/);
    expect(css).toMatch(/\.meeting-row\.meeting-item--active \{[^}]*border-inline-start-color/);
    expect(css).toMatch(/\.meeting-row\.meeting-item--active \.meeting-row__title \{[^}]*font-weight: var\(--weight-strong, var\(--weight-emphasis, 700\)\)/);
    expect(css).toMatch(/\.meeting-row--active,\s*\.w-shell \.rt \.meeting-item--active \{\s*box-shadow: none/);
    expect(css).toMatch(/forced-colors: active\) \{[\s\S]*meeting-item--active \{[^}]*outline: 1px solid Highlight/);
  });

  it("gives buttons and fields the minimum target and the 3:1 control edge", () => {
    expect(css).toMatch(/\.mode \{[^}]*min-height: var\(--w-target\)/);
    expect(css).toMatch(/\.mode \{[^}]*border: var\(--rt-control-line\)/);
    expect(css).toMatch(/\.input \{[^}]*min-height: var\(--w-target\)/);
    expect(css).toMatch(/\.input \{[^}]*border: var\(--rt-control-line\)/);
    expect(css).toMatch(/--rt-control-line: [^;]*var\(--w-control-border, var\(--color-border\)\)/);
  });
});

describe("no rule is dead: a class the screen puts on the same element as rt is written .rt.X, not .rt .X", () => {
  // `.rt .meeting-list summary` never matched because `.meeting-list` IS the .rt element (the descendant combinator needs a different
  // element), so the Import disclosure had no 44px target. The classes that sit beside rt come from the screens' own class strings.
  const sources = [topics, automations, meetings, recording, detail];
  const beside = new Set<string>();
  for (const src of sources) for (const m of src.matchAll(/"([^"]*\brt\b[^"]*)"/g)) for (const c of m[1].split(/\s+/)) if (c && c !== "rt") beside.add(c);

  it("finds the classes that sit beside rt (so the check below is not empty)", () => {
    for (const c of ["meeting-list", "empty-state", "topics", "stack", "card", "detail", "topics-empty"]) expect(beside.has(c), c).toBe(true);
  });

  it("every `.rt .X` where X can be the rt element itself has a `.rt.X` twin in the same rule", () => {
    const dead: string[] = [];
    for (const r of rules(css)) {
      for (const sel of r.selectors) {
        const m = /\.rt \.([a-z0-9_-]+)/i.exec(sel);
        if (!m || !beside.has(m[1])) continue;
        // A selector like `.rt .X` is fine only when the same rule also lists the same thing as `.rt.X`.
        const twin = sel.replace(`.rt .${m[1]}`, `.rt.${m[1]}`);
        if (!r.selectors.includes(twin)) dead.push(sel);
      }
    }
    expect(dead).toEqual([]);
  });

  it("the Import disclosure's summary and details are styled through .rt.meeting-list", () => {
    expect(css).toMatch(/\.w-shell \.rt\.meeting-list summary \{[^}]*min-height: var\(--w-target\)/);
    expect(css).toMatch(/\.w-shell \.rt\.meeting-list details \{/);
  });
});

describe("each restyled screen carries the rt class", () => {
  it("Topics: the list page, and the empty state", () => {
    expect(topics).toContain('className="topics rt"');
    expect(topics).toContain('className="empty-state topics-empty rt"');
    expect(topics.match(/\brt\b/g)?.length).toBe(2);
  });
  it("Automations", () => {
    expect(automations).toContain('className="stack rt"');
    expect(automations.match(/"[^"]*\brt\b[^"]*"/g)).toHaveLength(1);
  });
  it("Meetings: the list, the record card and the empty note slot, not the finished note or the live screen", () => {
    expect(meetings).toContain('className="card meeting-list rt"');
    expect(meetings).toContain('<section className="empty-state rt">');
    expect(recording).toContain('className="card rt"');
    expect(meetings).not.toContain('className="stack rt"');
  });
  it("the capture and the episode (not the meeting branch of Detail)", () => {
    expect(detail).toContain('className="detail rt"');
    expect(detail.match(/\brt\b/g)?.length).toBe(1);
  });
  it("the stylesheet is loaded by the redesigned shell", () => {
    expect(shell).toContain('import "../../system/restyle.css"');
  });
});

// ---- Contrast, from the token values --------------------------------------------------------------------------------------------------
const lum = (hex: string) => {
  const n = parseInt(hex.replace("#", "").slice(0, 6), 16);
  const ch = (v: number) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return 0.2126 * ch((n >> 16) & 255) + 0.7152 * ch((n >> 8) & 255) + 0.0722 * ch(n & 255);
};
const ratio = (a: string, b: string) => (Math.max(lum(a), lum(b)) + 0.05) / (Math.min(lum(a), lum(b)) + 0.05);
const color = (mode: Mode, name: string) => T.palette[mode][name];
const fixed = (mode: Mode, name: string) => T.fixed[name].$value[MODES.indexOf(mode)];
const control = (mode: Mode) => T.threadOnly.controlBorder.$value[MODES.indexOf(mode)];

/** Where the restyle draws text. [what, text color, the surfaces it sits on, minimum]. Text under 18.66px bold or 24px is body text: 4.5. */
const TEXT: [string, string, string[], number][] = [
  ["body text on every surface", "textPrimary", ["backgroundCard", "backgroundPage", "selection", "highlight"], 4.5],
  ["muted text (.muted, section labels, row sublines, the day line)", "textSecondary", ["backgroundCard", "backgroundPage", "selection", "highlight"], 4.5],
  ["links (.link-button, the recap's links)", "link", ["backgroundCard", "backgroundPage", "selection"], 4.5],
  ["warning text (a routine that cannot run)", "warning", ["backgroundCard", "backgroundPage"], 4.5],
  ["error text (an overdue date, the error line)", "error", ["backgroundCard", "backgroundPage", "highlight"], 4.5],
  ["the primary button's words", "textOnAccent", ["accentPrimary"], 4.5],
];
/** Marks that are not text: the chosen row's bar, the approval bar, the control edge, the focus ring. 3:1 (WCAG 1.4.11). */
const MARKS: [string, (mode: Mode) => string, string[], number][] = [
  ["the chosen row's bar and the approval bar (accent)", (m) => color(m, "accentPrimary"), ["backgroundCard", "backgroundPage", "selection"], 3],
  // Not on `selection`: a button's hover fill is the selection color inside its own edge, which is the Thread button's hover everywhere; the edge is
  // still found against the card or page around it.
  ["the control edge (buttons, fields)", control, ["backgroundCard", "backgroundPage"], 3],
  ["the focus ring", (m) => fixed(m, "focus"), ["backgroundCard", "backgroundPage", "selection"], 3],
];

describe("contrast of every pair the restyle introduces, in light, dark and both high contrast modes", () => {
  for (const mode of MODES) {
    it.each(TEXT)(`${mode}: %s`, (_what, name, surfaces, min) => {
      for (const surface of surfaces) expect(ratio(color(mode, name), color(mode, surface)), `${mode} ${name} ${color(mode, name)} on ${surface} ${color(mode, surface)}`).toBeGreaterThanOrEqual(min);
    });
    it.each(MARKS)(`${mode}: %s`, (_what, pick, surfaces, min) => {
      for (const surface of surfaces) expect(ratio(pick(mode), color(mode, surface)), `${mode} ${pick(mode)} on ${surface} ${color(mode, surface)}`).toBeGreaterThanOrEqual(min);
    });
  }

  // WCAG exempts a disabled control from the contrast rule, so this is a floor the pass sets for itself, not a requirement: a disabled button
  // is drawn at the stylesheet's opacity over what is behind it (fill and words fade together). The secondary button's words hold 3:1 in every
  // mode. The primary button's do not (about 2.4:1 in light at this opacity): it is exempt, so it is held to 2:1 only, to catch it getting
  // worse, and raising the opacity is the way to improve it.
  // (Body text on the card is 4.5:1 at full strength, tested above; that is not the disabled pair and is not repeated here.)
  const opacity = Number(/\.mode:disabled \{[^}]*opacity:\s*([\d.]+)/.exec(strip(css))?.[1]);
  const mix = (top: string, behind: string) => {
    const rgb = (h: string) => [16, 8, 0].map((sh) => (parseInt(h.slice(1, 7), 16) >> sh) & 255);
    const out = rgb(top).map((v, i) => Math.round(opacity * v + (1 - opacity) * rgb(behind)[i]));
    return `#${out.map((v) => v.toString(16).padStart(2, "0")).join("")}`;
  };
  it("reads the disabled opacity from the stylesheet", () => {
    expect(opacity).toBeGreaterThan(0.3);
    expect(opacity).toBeLessThan(1);
  });
  for (const mode of MODES) {
    it(`${mode}: a disabled button's words, drawn at the stylesheet's opacity, stay at 3:1 (secondary) and 2:1 (primary, exempt)`, () => {
      for (const behind of ["backgroundCard", "backgroundPage"]) {
        const under = color(mode, behind);
        const secondary = ratio(mix(color(mode, "textPrimary"), under), mix(color(mode, "backgroundCard"), under));
        const primary = ratio(mix(color(mode, "textOnAccent"), under), mix(color(mode, "accentPrimary"), under));
        expect(secondary, `${mode} secondary on ${behind}`).toBeGreaterThanOrEqual(3);
        expect(primary, `${mode} primary on ${behind}`).toBeGreaterThanOrEqual(2);
      }
    });
  }
});
