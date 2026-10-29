import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { chooseSkin, DEFAULT_SELECTION, setInstalledSkins, type SkinDto, type SkinManifest, type SkinPalette } from "../skin";
import { fakeWindow } from "../testWindow";
import { THEMES } from "../themes";
import { SkinGallery } from "./SkinGallery";

const manifests = Object.values(import.meta.glob("../../../../../skins/*.json", { import: "default", eager: true }) as Record<string, SkinManifest>).filter((m) =>
  Array.isArray(m.palettes),
);
const pool: SkinPalette[] = [
  ...THEMES.map((t) => ({ id: t.id, name: t.name, author: t.author, light: t.light, dark: t.dark })),
  ...Object.values(import.meta.glob("../../../../../skins/palettes/*.json", { import: "default", eager: true }) as Record<string, SkinPalette>),
];
const skins: SkinDto[] = manifests.map((skin) => ({ skin, builtIn: true, palettes: skin.palettes.flatMap((id) => pool.filter((p) => p.id === id)) }));

function show(selection: typeof DEFAULT_SELECTION): string {
  const f = fakeWindow();
  vi.stubGlobal("window", Object.assign(f.win, { matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) }));
  setInstalledSkins(skins);
  chooseSkin(selection, f.win);
  return renderToStaticMarkup(<SkinGallery />);
}

describe("the gallery card", () => {
  beforeEach(() => vi.unstubAllGlobals());
  afterEach(() => {
    vi.unstubAllGlobals();
    setInstalledSkins([]);
  });

  it("draws a card per skin with its tagline, and the per-PC footnote", async () => {
    const html = show(DEFAULT_SELECTION);
    for (const m of manifests) expect(html).toContain(m.name);
    expect(html.match(/class="skin-card( |")/g)).toHaveLength(manifests.length);
    expect(html).toContain("Each PC keeps its own skin.");
    expect(html).not.toContain("always uses");
  });

  it("locks System/Light/Dark and says why for a dark-only palette, and offers the selected skin's palettes", () => {
    const retro = skins.find((d) => d.skin.id === "retro-terminal")!;
    const darkOnly = retro.palettes.find((p) => !p.light)!;
    const html = show({ skinId: "retro-terminal", paletteId: darkOnly.id, appearance: "light" });
    expect(html).toContain("This palette is dark only, so it always uses Dark.");
    expect(html.match(/class="mode[^"]*"[^>]*disabled/g)).toHaveLength(3);
    expect(html).toContain('aria-label="Retro Terminal palettes"');
    expect(html.match(/class="skin-card skin-card--selected"/g)).toHaveLength(1);
    expect(show(DEFAULT_SELECTION)).not.toMatch(/class="mode[^"]*"[^>]*disabled/);
  });

  it("puts the skin chat below the gallery, its Send held until the shell says a provider is usable", () => {
    const html = show(DEFAULT_SELECTION);
    expect(html.indexOf("Make a skin")).toBeGreaterThan(html.indexOf("Each PC keeps its own skin."));
    expect(html).toContain('placeholder="Describe a skin…"');
    expect(html).not.toContain("Set up an AI provider");
    expect(html).toContain("Add your own image…");
  });
});
