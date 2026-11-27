// Richer skins, row 10: motion. One token set, read from `subtle` and `none`; Reduce Motion gives zero; Thread keeps what it
// has; the recording indicator and the consent banner never read the skin's motion. The style sheets are read as text, since
// jsdom draws no animation.

import { describe, expect, it } from "vitest";
import expected from "../../../../../skins/testdata/expected/motion.json";
import { MOTION_VARIABLES, THREAD, WHIMBREL, motionTokens, motionVariables, skinVariables, type SkinManifest } from "../skin";

const sheets = import.meta.glob("../**/*.css", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const strip = (text: string) => text.replace(/\/\*[\s\S]*?\*\//g, "");
const skin = (motion: string): SkinManifest => ({ ...WHIMBREL.skin, id: "almanac", motion });

describe("the motion tokens", () => {
  for (const c of expected.cases) {
    it(`give the shared values for ${c.motion}${c.reduceMotion ? " with Reduce Motion" : ""}`, () => {
      expect(motionTokens(c.motion, c.reduceMotion)).toEqual({ section: c.section, rise: c.rise, control: c.control, marker: c.marker, panel: c.panel });
    });
  }

  it("turn an unknown value into none", () => {
    expect(motionTokens("wild", false)).toEqual(motionTokens("none", false));
  });
});

describe("the motion variables", () => {
  it("are zero durations and no animation for none, and for any skin when the system asks for less", () => {
    for (const vars of [motionVariables(skin("none"), false), motionVariables(skin("subtle"), true)]) {
      expect(vars["--dur-section"]).toBe("0ms");
      expect(vars["--dur-control"]).toBe("0ms");
      expect(vars["--dur-marker"]).toBe("0ms");
      expect(vars["--dur-panel"]).toBe("0ms");
      expect(vars["--anim-section"]).toBe("none");
      expect(vars["--anim-marker"]).toBe("none");
      expect(vars["--anim-panel"]).toBe("none");
    }
  });

  it("are the plan's values for subtle", () => {
    const v = motionVariables(skin("subtle"), false);
    expect(v).toMatchObject({ "--dur-section": "150ms", "--rise-section": "4px", "--dur-control": "120ms", "--dur-marker": "200ms", "--dur-panel": "150ms" });
    expect(v["--anim-section"]).toBe("w-rise");
  });

  it("come through skinVariables, and the list of them is complete", () => {
    expect(skinVariables(skin("subtle"), false)["--dur-control"]).toBe("120ms");
    expect(skinVariables(skin("subtle"), true)["--dur-control"]).toBe("0ms");
    expect(Object.keys(motionVariables(skin("subtle"), false)).sort()).toEqual([...MOTION_VARIABLES].sort());
  });

  it("are not set for Thread, whatever its file says, so it draws as it did", () => {
    expect(THREAD.skin.motion).toBe("subtle");
    expect(motionVariables(THREAD.skin, false)).toEqual({});
    expect(Object.keys(skinVariables(THREAD.skin, false)).filter((k) => /^--(dur|anim|rise)-/.test(k))).toEqual([]);
  });
});

describe("the style sheets", () => {
  const all = Object.entries(sheets).map(([path, text]) => [path, strip(text)] as const);

  it("fall back to Thread's own durations where a rule already had one", () => {
    const text = all.map(([, t]) => t).join("\n");
    expect(text).toMatch(/--dur-control,\s*var\(--w-duration-fast\)/);
    expect(text).toMatch(/animation-name:\s*var\(--anim-section,\s*none\)/);
  });

  it("scale every motion duration to zero under Reduce Motion", () => {
    const tokens = strip(sheets["../tokens.css"]);
    expect(tokens).toMatch(/@media \(prefers-reduced-motion: reduce\)\s*\{\s*:root\s*\{\s*--motion-scale:\s*0/);
    for (const [path, text] of all) for (const m of text.matchAll(/var\(--dur-[a-z]+[^;]*;/g)) expect(m[0], path).toContain("--motion-scale");
  });

  it("never read the skin's motion in the recording indicator or the consent banner", () => {
    for (const [path, text] of all) {
      for (const m of text.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
        if (!/(recording|consent|capture-control)/i.test(m[1])) continue;
        expect(m[2], `${path}: ${m[1].trim()}`).not.toMatch(/--(dur|anim|rise)-/);
      }
    }
  });

  it("animate only inside Home, the ask bar, links and buttons", () => {
    for (const [path, text] of all) {
      if (!/--(anim)-/.test(text)) continue;
      expect(path).toMatch(/today\/redesign\/home\.css$/);
    }
  });
});
