// Richer skins, row 11 (docs/superpowers/plans/2026-11-25-richer-skins.md section 8): a skin card draws the real Home's parts from
// sample text, in its own skin, as a still picture hidden from the accessibility tree. These read the markup, the style
// sheets and the source. How it looks on a real PC, and that Narrator reads only the name and tagline, are checked by hand.

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { WButton } from "../../system";
import { THEMES } from "../../themes";
import type { SkinDto, SkinManifest, SkinPalette } from "../../skin";
import { SkinPreviewHome, previewStyle, SAMPLE } from "./SkinPreviewHome";
import previewCss from "./skinPreview.css?raw";
import source from "./SkinPreviewHome.tsx?raw";

const manifests = Object.values(import.meta.glob("../../../../../../skins/*.json", { import: "default", eager: true }) as Record<string, SkinManifest>).filter((m) => Array.isArray(m.palettes));
const pool: SkinPalette[] = [
  ...THEMES.map((t) => ({ id: t.id, name: t.name, author: t.author, light: t.light, dark: t.dark })),
  ...Object.values(import.meta.glob("../../../../../../skins/palettes/*.json", { import: "default", eager: true }) as Record<string, SkinPalette>),
];
const dtoOf = (id: string): SkinDto => {
  const skin = manifests.find((m) => m.id === id)!;
  return { skin, builtIn: true, palettes: skin.palettes.flatMap((p) => pool.filter((x) => x.id === p)) };
};
const show = (id: string, over: Partial<Parameters<typeof SkinPreviewHome>[0]> = {}) => {
  const dto = dtoOf(id);
  return renderToStaticMarkup(<SkinPreviewHome dto={dto} palette={dto.palettes[0]} appearance="light" systemDark={false} {...over} />);
};
const IDS = manifests.map((m) => m.id);

describe("a skin card's picture of Home", () => {
  it("is drawn for every built-in skin", () => {
    expect(IDS.length).toBeGreaterThanOrEqual(8);
    for (const id of IDS) {
      const out = show(id);
      expect(out, id).toContain('class="today th-home"');
      expect(out, id).toContain(SAMPLE.next.title);
      expect(out, id).toContain(`data-skin="${id}"`);
    }
  });

  it("is hidden from the accessibility tree as a whole, and holds nothing to focus or press", () => {
    for (const id of IDS) {
      const out = show(id);
      expect(out.startsWith('<div class="skin-preview-frame" aria-hidden="true">'), id).toBe(true);
      for (const bad of ["<button", "<input", "<a ", "tabindex", " href=", "id=\"th-", "<form", "<select", "<textarea", "role="]) expect(out, `${id} ${bad}`).not.toContain(bad);
    }
  });

  it("draws the skin's own element choices, the same views the real Home uses", () => {
    expect(show("game-day")).toContain("w-next-scoreboard");
    expect(show("reference")).toContain("w-next-infobox");
    expect(show("reference")).toContain("w-spine-ruled");
    expect(show("almanac")).toContain("w-spine-ruled");
    expect(show("almanac")).toContain("w-label-smallCaps");
    expect(show("retro-terminal")).toContain("w-label-bracketed");
    expect(show("thread")).not.toContain("w-spine-ruled");
    expect(show("thread")).not.toContain("w-next-scoreboard");
    // Actions as links (Reference) are drawn as links; the rest as buttons.
    expect(show("reference")).toContain("w-button-link");
    expect(show("thread")).not.toContain("w-button-link");
  });

  it("draws a divider above each section after the first, only for a skin that has a style", () => {
    for (const id of IDS) {
      const style = dtoOf(id).skin.decoration.dividers;
      const count = show(id).match(/class="w-divider"/g)?.length ?? 0;
      expect(count, id).toBe(style === "none" ? 0 : 3);
      const home = show(id).match(/<div class="today th-home"[^>]*>/)![0];
      expect(home.includes(`data-dividers="${style}"`), id).toBe(style !== "none");
    }
  });

  it("carries each card's own divider style on its Home, so a card does not take the page's", () => {
    const a = show("almanac");
    const b = show("game-day");
    const home = (out: string) => out.match(/<div class="today th-home"[^>]*>/)![0];
    expect(home(a)).toContain('data-dividers="double"');
    expect(home(b)).toContain('data-dividers="chalk"');
    expect(home(a)).not.toContain("chalk");
  });

  it("carries the skin's palette, fonts and shape as its own variables, with motion off", () => {
    for (const id of IDS) {
      const dto = dtoOf(id);
      const style = previewStyle(dto, dto.palettes[0], "light") as Record<string, string>;
      expect(style["--color-background-page"], id).toBe(dto.palettes[0].light?.backgroundPage ?? dto.palettes[0].dark?.backgroundPage);
      expect(style["--font-body"], id).toBeTruthy();
      expect(style["--radius"], id).toBe(`${dto.skin.shape.radius}px`);
      if (id !== "thread") {
        expect(style["--dur-section"], id).toBe("0ms");
        expect(style["--dur-marker"], id).toBe("0ms");
        expect(style["--anim-section"], id).toBe("none");
      }
    }
  });

  it("draws the header band of its own skin and none for a skin without one", () => {
    expect(show("almanac")).toContain('data-header-band="rule"');
    expect(show("game-day")).toContain('data-header-band="stripes-diagonal"');
    expect(show("thread")).toContain('data-header-band="none"');
    expect(previewCss).toMatch(/\.skin-preview\[data-header-band="rule"\] \.skin-preview-band \{\s*display: block;\s*height: 2px;/);
    expect(previewCss).toMatch(/\.skin-preview\[data-header-band="stripes-diagonal"\] \.skin-preview-band/);
  });

  it("shows the palette for the appearance asked, and the High Contrast one with more contrast", () => {
    const dto = dtoOf("whimbrel");
    const both = dto.palettes.find((p) => p.light && p.dark)!;
    const light = previewStyle(dto, both, "light") as Record<string, string>;
    const dark = previewStyle(dto, both, "dark") as Record<string, string>;
    expect(light["--color-background-page"]).toBe(both.light!.backgroundPage);
    expect(dark["--color-background-page"]).toBe(both.dark!.backgroundPage);
    expect(show("whimbrel", { palette: both, appearance: "dark" })).toContain('data-theme="dark"');
    expect(show("whimbrel", { palette: both, appearance: "light" })).toContain('data-theme="light"');
    expect(show("whimbrel", { palette: both, appearance: "system", systemDark: true })).toContain('data-theme="dark"');
  });

  it("is the same picture every time: fixed sample text, no clock", () => {
    for (const id of IDS) expect(show(id), id).toBe(show(id));
    expect(show("thread")).not.toContain(String(new Date().getFullYear() + 5));
    expect(show("thread")).toContain(SAMPLE.threads[0].subject);
  });

  it("is still: no timers, no reads from the shell, no live hooks in its source, and no animation or transition in its styles", () => {
    const code = source.replace(/\/\/.*$/gm, "").replace(/\/\*[\s\S]*?\*\//g, "");
    for (const live of ["setInterval", "setTimeout", "requestAnimationFrame", "useEffect", "useSyncExternalStore", "useCapture", "useHomeData", "useDayBlocks", "useApprovals", "useSkin(", "from \"../../api\"", "invoke("]) {
      expect(code, live).not.toContain(live);
    }
    expect(previewCss).toMatch(/\.skin-preview \*::after \{\s*animation: none !important;\s*transition: none !important;/);
    expect(previewCss).toMatch(/@media \(forced-colors: active\), \(prefers-contrast: more\) \{\s*\.skin-preview \{\s*--sx-texture: none;/);
  });
});

describe("a still picture's buttons", () => {
  it("look the same and are not buttons, so none sits inside the card's own button", () => {
    const live = renderToStaticMarkup(<WButton variant="primary" onClick={() => {}}>Start</WButton>);
    const still = renderToStaticMarkup(
      <WButton variant="primary" still onClick={() => {}}>
        Start
      </WButton>,
    );
    expect(live).toBe('<button type="button" class="w-button w-button-primary"><span>Start</span></button>');
    expect(still).toBe('<span class="w-button w-button-primary"><span>Start</span></span>');
  });
});
