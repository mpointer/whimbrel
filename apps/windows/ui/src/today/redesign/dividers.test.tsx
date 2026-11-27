// Richer skins, row 6 (docs/superpowers/plans/2026-11-25-richer-skins.md, section 3): a skin's divider style draws between
// Home's sections and in the ruled and line time lines, hidden from Narrator, and the ruled line skips the hero. These read the
// markup and the style sheets: how it looks on a real PC, and what Narrator says, are checked by hand.

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { WDivider, WSpine, WSpineStop } from "../../system";
import homeCss from "./home.css?raw";
import systemCss from "../../system/system.css?raw";
import { withDividers } from "./HomeView";
import { arrangeTimeline, buildTimeline } from "./homeModel";
import { skinAttributes, skinVariables } from "../../skin";
import { THREAD } from "../../skin";

const html = (n: React.ReactElement) => renderToStaticMarkup(n);
const strip = (text: string) => text.replace(/\/\*[\s\S]*?\*\//g, "");
const STYLES = ["hairline", "chalk", "double", "dotted"] as const;

describe("the divider", () => {
  it("is decoration: hidden from the accessibility tree and empty", () => {
    expect(html(<WDivider />)).toBe('<div class="w-divider" aria-hidden="true"></div>');
  });
  it("goes above each section after the first, and never above the first", () => {
    const out = (on: boolean) => html(<div>{withDividers([<p key="a">a</p>, null, <p key="b">b</p>, false, <p key="c">c</p>], on)}</div>);
    expect(out(true)).toBe('<div><p>a</p><div class="w-divider" aria-hidden="true"></div><p>b</p><div class="w-divider" aria-hidden="true"></div><p>c</p></div>');
    expect(out(true).match(/w-divider/g)?.length).toBe(2);
  });
  it("adds nothing for a skin with no divider style, so Thread's markup is the same", () => {
    expect(html(<div>{withDividers([<p key="a">a</p>, <p key="b">b</p>], false)}</div>)).toBe("<div><p>a</p><p>b</p></div>");
    expect(html(<div>{withDividers([], true)}</div>)).toBe("<div></div>");
    expect(skinAttributes(THREAD.skin)["data-dividers"]).toBe("none");
  });
  it("is hidden under every style, and no divider carries text", () => {
    for (const style of STYLES) expect(html(<WDivider />), style).not.toMatch(/>[^<]+</);
  });
});

describe("the ruled time line and the hero", () => {
  it("marks the hero stop so the style sheet can skip it", () => {
    const out = html(
      <WSpine label="Today" variant="ruled">
        <WSpineStop time="9:00 AM" node="past" label="9:00 AM, A">x</WSpineStop>
        <WSpineStop time="10:31 AM" node="upcoming" hero label="10:31 AM, B">y</WSpineStop>
        <WSpineStop time="1:00 PM" node="upcoming" label="1:00 PM, C">z</WSpineStop>
      </WSpine>,
    );
    expect(out.match(/w-spine-hero/g)?.length).toBe(1);
    expect(out).toContain('class="w-spine-stop w-spine-hero"');
    expect(out).toContain('class="w-spine-stop"');
  });
  it("draws no rule above the first row, on the hero, or on the row after it, with or without a divider style", () => {
    const css = strip(homeCss);
    expect(css).toMatch(/\.th-home \.w-spine-ruled \.w-spine-stop:first-child,\s*\.th-home \.w-spine-ruled \.w-spine-hero,\s*\.th-home \.w-spine-ruled \.w-spine-hero \+ \.w-spine-stop\s*\{\s*border-top: 0;/);
    expect(css).toContain(".w-spine-stop:not(:first-child):not(.w-spine-hero):not(.w-spine-hero + .w-spine-stop)::before");
  });
  it("leaves the hero meeting out of the stops it draws beside it, live or upcoming", () => {
    const at = (h: number, m = 0) => new Date(2026, 10, 25, h, m).getTime();
    const ev = (id: string, h: number, rec: number | null = null) => ({ eventId: id, title: id, startUnixMs: at(h), endUnixMs: at(h + 1), recordedMeetingId: rec, attendees: [], prepKey: null });
    const schedule = { events: [ev("e1", 10), ev("e2", 14, 7)], held: [{ id: 7, title: "Live one", startUnixMs: at(14), endUnixMs: null }], recording: null } as never;
    const stops = (heroEventId: string | null, heroMeetingId: number | null) =>
      arrangeTimeline(buildTimeline({ now: at(12), locale: "en-US", schedule, heroEventId, heroMeetingId, blocks: [] }));
    const keys = (r: ReturnType<typeof stops>) => [...r.ahead, ...r.behind].map((s) => s.kind + ":" + (s as { key?: string }).key);
    // with no hero the same meetings are listed, so the checks below can fail
    expect(keys(stops(null, null)).join()).toContain("e1");
    expect(keys(stops(null, null)).join()).toContain("e2");
    expect(keys(stops("e1", null)).join()).not.toContain("e1");
    expect(keys(stops(null, 7)).join()).not.toContain("e2");
  });
});

describe("the style sheet", () => {
  const css = strip(homeCss);
  it("sets a thickness per style: 1px, 2px, 4px (two 1px strokes and a 2px gap) and 1px dots", () => {
    expect(css).toMatch(/\.th-home\[data-dividers="hairline"\]\s*\{\s*--dv-h: 1px;/);
    expect(css).toMatch(/\.th-home\[data-dividers="chalk"\]\s*\{\s*--dv-h: 2px;/);
    expect(css).toMatch(/\.th-home\[data-dividers="double"\]\s*\{\s*--dv-h: 4px;/);
    expect(css).toMatch(/\.th-home\[data-dividers="dotted"\]\s*\{\s*--dv-h: 1px;/);
    expect(css).toMatch(/\.th-home \{\s*--dv-h: 0px;/);
  });
  it("draws each style: hairline, chalk at 60 percent with round ends, double, dotted at 3px", () => {
    const rule = (style: string) => css.match(new RegExp(`\\.th-home\\[data-dividers="${style}"\\] \\.w-divider,[^{]*\\{([^}]*)\\}`))?.[1] ?? "";
    expect(rule("hairline")).toContain("background: var(--color-border)");
    expect(rule("chalk")).toContain("opacity: 0.6");
    expect(rule("chalk")).toContain("border-radius: 1px");
    expect(rule("double")).toContain("var(--color-border) 0 1px, transparent 1px 3px, var(--color-border) 3px 4px");
    expect(rule("dotted")).toContain("repeating-linear-gradient(90deg, var(--color-border) 0 1px, transparent 1px 3px)");
  });
  it("uses the border token and the same strokes down the spine of the line time line", () => {
    expect(css).toContain("--dv-c: var(--color-border)");
    expect(css).toMatch(/\.th-home\[data-dividers="double"\] \.w-spine:not\(\.w-spine-ruled\) \.w-spine-rail::after \{[^}]*linear-gradient\(90deg, var\(--dv-c\) 0 1px, transparent 1px 3px, var\(--dv-c\) 3px 4px\)/);
    expect(css).toMatch(/\.th-home\[data-dividers="dotted"\] \.w-spine:not\(\.w-spine-ruled\) \.w-spine-rail::after \{[^}]*repeating-linear-gradient\(180deg, var\(--dv-c\) 0 1px, transparent 1px 3px\)/);
  });
  it("does not put a pseudo-element inside :is(), which would drop the rule", () => {
    expect(css).not.toMatch(/:is\([^)]*::(before|after)/);
  });
  it("draws solid in the system's text colour under forced colors", () => {
    expect(homeCss).toMatch(/@media \(forced-colors: active\)[\s\S]*background: CanvasText !important/);
  });
  it("stays on Home: the system style sheet knows nothing of dividers, and every rule here is under .th-home", () => {
    expect(systemCss).not.toContain("data-dividers");
    const selectors = [...css.matchAll(/(?:^|\})\s*([^{}@]+)\{/g)].map((m) => m[1].trim()).filter((sel) => /w-divider|w-spine/.test(sel) && sel.includes("dividers"));
    expect(selectors.length).toBeGreaterThan(8);
    for (const sel of selectors) expect(sel, sel).toContain(".th-home");
    expect(skinVariables(THREAD.skin)["--border-trim"]).toBe("0px");
  });
});
