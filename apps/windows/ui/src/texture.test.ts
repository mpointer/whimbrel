// Richer skins, row 9: a skin's texture is drawn by code on the page background, moves the page colour by at most 4 percent in
// luminance on every built-in palette, and is off in High Contrast, forced colors and more contrast.

import { describe, expect, it } from "vitest";
import tokensCss from "./tokens.css?raw";
import appCss from "./app.css?raw";
import shellCss from "./shell/redesign/shell.css?raw";
import systemCss from "./system/system.css?raw";
import {
  TEXTURE_MAX_SHIFT,
  TEXTURE_NOMINAL_ALPHA,
  TEXTURE_VARIABLE,
  WHIMBREL,
  applySkin,
  relativeLuminance,
  skinTexture,
  textureAlphaShift,
  textureBackground,
  textureInk,
  texturePeakAlpha,
  textureShift,
  type SkinDto,
  type SkinPalette,
  type Texture,
} from "./skin";

const TEXTURES: Texture[] = ["paper", "grain", "grid"];
const palettes = Object.values(
  import.meta.glob("../../../../skins/palettes/*.json", { import: "default", eager: true }) as Record<string, SkinPalette>,
);
const manifests = Object.values(
  import.meta.glob("../../../../skins/*.json", { import: "default", eager: true }) as Record<string, SkinDto["skin"]>,
);

/** Every page colour a built-in palette ships, with the palette it is from. */
const pages = palettes.flatMap((p) =>
  (["light", "dark"] as const).flatMap((mode) => (p[mode] ? [{ id: `${p.id} ${mode}`, hex: p[mode]!.backgroundPage }] : [])),
);

function fixture() {
  const props = new Map<string, string>();
  const root = {
    style: {
      setProperty: (n: string, v: string) => void props.set(n, v),
      removeProperty: (n: string) => void props.delete(n),
      colorScheme: "",
    },
    setAttribute: () => {},
    removeAttribute: () => {},
  };
  return { props, root: root as unknown as HTMLElement };
}
const withTexture = (id: string, texture: string): SkinDto => ({
  ...WHIMBREL,
  skin: { ...WHIMBREL.skin, id, decoration: { ...WHIMBREL.skin.decoration, texture } },
});

describe("texture strength", () => {
  it("has every built-in palette to test", () => {
    expect(palettes.length).toBeGreaterThanOrEqual(12);
    expect(pages.length).toBeGreaterThanOrEqual(12);
  });

  it("moves no built-in palette's page colour by more than 4 percent of luminance", () => {
    expect(TEXTURE_MAX_SHIFT).toBe(0.04);
    for (const page of pages) {
      for (const texture of TEXTURES) {
        const shift = textureShift(texture, page.hex);
        expect(shift, `${page.id} ${texture}`).toBeLessThanOrEqual(TEXTURE_MAX_SHIFT);
        expect(shift, `${page.id} ${texture} draws something`).toBeGreaterThan(0);
      }
    }
  });

  it("holds the cap on every grey page, the worst case for black ink and for white ink", () => {
    for (let v = 0; v <= 255; v += 5) {
      const hex = `#${v.toString(16).padStart(2, "0").repeat(3)}`;
      for (const texture of TEXTURES) expect(textureShift(texture, hex), `${hex} ${texture}`).toBeLessThanOrEqual(TEXTURE_MAX_SHIFT);
    }
  });

  it("gives a light page less ink than a dark one, never more than nominal", () => {
    for (const texture of TEXTURES) {
      const light = texturePeakAlpha(texture, "#ffffff");
      const dark = texturePeakAlpha(texture, "#101418");
      expect(light).toBeGreaterThan(0);
      expect(light).toBeLessThan(dark);
      expect(dark).toBeLessThanOrEqual(TEXTURE_NOMINAL_ALPHA[texture]);
    }
    expect(texturePeakAlpha("none", "#ffffff")).toBe(0);
  });

  it("uses white ink on a dark page and black on a light one", () => {
    expect(textureInk("#101418")).toBe("white");
    expect(textureInk("#f7f2e7")).toBe("black");
    expect(relativeLuminance("#ffffff")).toBeCloseTo(1, 5);
    expect(textureAlphaShift(0, "#f7f2e7")).toBe(0);
  });
});

describe("texture is drawn by code", () => {
  it("draws noise from an SVG filter and the grid from gradients, with no file", () => {
    const paper = textureBackground("paper", "#f7f2e7");
    const grain = textureBackground("grain", "#f7f2e7");
    const grid = textureBackground("grid", "#f7f2e7");
    expect(paper).toContain("data:image/svg+xml");
    expect(decodeURIComponent(paper)).toContain("feTurbulence");
    expect(decodeURIComponent(grain)).toContain("baseFrequency='0.45'");
    expect(decodeURIComponent(paper)).toContain("baseFrequency='0.9'");
    expect(grid).toContain("linear-gradient");
    expect(grid).toContain("24px 24px");
    expect(textureBackground("none", "#f7f2e7")).toBe("none");
  });

  it("never asks the noise for more alpha than the page's cap", () => {
    for (const page of pages) {
      for (const texture of ["paper", "grain"] as const) {
        const matrix = /values='([^']*)'/.exec(decodeURIComponent(textureBackground(texture, page.hex)))![1].split(/\s+/).map(Number);
        // the last row maps the noise (0 to 1) to alpha: its first number is the strongest alpha
        const alpha = matrix[15];
        expect(alpha, `${page.id} ${texture}`).toBeCloseTo(texturePeakAlpha(texture, page.hex), 3);
        expect(textureAlphaShift(alpha, page.hex), `${page.id} ${texture}`).toBeLessThanOrEqual(TEXTURE_MAX_SHIFT);
      }
    }
  });

  it("sets the variable the style sheets read, and clears it for none", () => {
    const f = fixture();
    applySkin(withTexture("x", "grain"), "default", "light", f.root, false, false);
    expect(f.props.get(TEXTURE_VARIABLE)).toContain("data:image/svg+xml");
    applySkin(withTexture("x", "none"), "default", "light", f.root, false, false);
    expect(f.props.has(TEXTURE_VARIABLE)).toBe(false);
  });

  it("follows the page colour of the mode it draws", () => {
    const f = fixture();
    applySkin(withTexture("x", "grid"), "default", "light", f.root, false, false);
    const light = f.props.get(TEXTURE_VARIABLE);
    applySkin(withTexture("x", "grid"), "default", "dark", f.root, false, false);
    expect(f.props.get(TEXTURE_VARIABLE)).not.toBe(light);
    expect(light).toContain("rgba(0,0,0,");
    expect(f.props.get(TEXTURE_VARIABLE)).toContain("rgba(255,255,255,");
  });
});

describe("texture is off where it must be", () => {
  it("is off for Thread and the High Contrast skin, whatever the file says", () => {
    expect(skinTexture(withTexture("thread", "grain").skin)).toBe("none");
    expect(skinTexture(withTexture("high-contrast", "grid").skin)).toBe("none");
    expect(skinTexture(withTexture("almanac", "grid").skin)).toBe("grid");
    const f = fixture();
    applySkin(withTexture("high-contrast", "grain"), "default", "light", f.root, false, false);
    expect(f.props.has(TEXTURE_VARIABLE)).toBe(false);
  });

  it("is off when the system asks for more contrast", () => {
    const f = fixture();
    applySkin(withTexture("x", "paper"), "default", "light", f.root, false, true);
    expect(f.props.has(TEXTURE_VARIABLE)).toBe(false);
  });

  it("is off under forced colors and more contrast in the style sheet, and for the High Contrast skin", () => {
    const css = tokensCss.replace(/\/\*[\s\S]*?\*\//g, "");
    expect(css).toMatch(/--sx-texture:\s*var\(--texture-image,\s*none\)/);
    expect(css).toMatch(/:root\[data-skin="high-contrast"\]\s*\{\s*--sx-texture:\s*none;/);
    expect(css).toMatch(/@media \(forced-colors: active\), \(prefers-contrast: more\)\s*\{[^@]*--sx-texture:\s*none;/);
  });

  it("no built-in skin asks for a texture the app cannot switch off", () => {
    for (const m of manifests.filter((x) => x.decoration)) expect(["none", "paper", "grain", "grid"], m.id).toContain(m.decoration.texture);
  });
});

describe("texture is for the page background only", () => {
  it("is read by the page's own rules (body, shell, first run) under the page colour, and nowhere else", () => {
    const all = import.meta.glob("./**/*.css", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
    const users = Object.entries(all)
      .filter(([, text]) => text.replace(/\/\*[\s\S]*?\*\//g, "").includes("var(--sx-texture"))
      .map(([name]) => name)
      .sort();
    // A skin card's picture of Home sets the same variable for its own skin (richer skins row 11); it is the picture's own background.
    expect(users).toEqual(["./app.css", "./shell/redesign/shell.css", "./system/system.css", "./today/redesign/skinPreview.css"]);
    for (const text of [appCss, shellCss, systemCss]) {
      const rules = [...text.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*var\(--sx-texture[^{}]*)\}/g)];
      expect(rules.length).toBeGreaterThan(0);
      for (const rule of rules) {
        expect(rule[1].trim()).toMatch(/^(body|\.w-shell|\.w-flow-root)$/);
        expect(rule[2]).toMatch(/background:\s*var\(--sx-texture, none\), var\(--color-background-page\)/);
      }
    }
  });
});
