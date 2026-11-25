import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { chooseSkin, DEFAULT_SELECTION, setInstalledSkins, type SkinDto, type SkinManifest, type SkinPalette } from "../../../../skin";
import { fakeWindow } from "../../../../testWindow";
import { THEMES } from "../../../../themes";
import { AppearanceCard, SkinGalleryCard } from "./SkinCards";

// The same fixtures as SkinGallery.test.tsx: the bundled skins, without Thread (listed only with the redesign flag).
const manifests = Object.values(import.meta.glob("../../../../../../../../skins/*.json", { import: "default", eager: true }) as Record<string, SkinManifest>).filter(
  (m) => Array.isArray(m.palettes) && m.id !== "thread",
);
const pool: SkinPalette[] = [
  ...THEMES.map((t) => ({ id: t.id, name: t.name, author: t.author, light: t.light, dark: t.dark })),
  ...Object.values(import.meta.glob("../../../../../../../../skins/palettes/*.json", { import: "default", eager: true }) as Record<string, SkinPalette>),
];
const skins: SkinDto[] = manifests.map((skin) => ({ skin, builtIn: true, palettes: skin.palettes.flatMap((id) => pool.filter((p) => p.id === id)) }));

function show(selection: typeof DEFAULT_SELECTION, turnRunning = false, extra: SkinDto[] = []): string {
  const f = fakeWindow();
  vi.stubGlobal("window", Object.assign(f.win, { matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) }));
  setInstalledSkins([...skins, ...extra]);
  chooseSkin(selection, f.win);
  return renderToStaticMarkup(
    <>
      <AppearanceCard />
      <SkinGalleryCard turnRunning={turnRunning} />
    </>,
  );
}

describe("the redesigned skin gallery", () => {
  beforeEach(() => vi.unstubAllGlobals());
  afterEach(() => {
    vi.unstubAllGlobals();
    setInstalledSkins([]);
  });

  it("is headed cards: Appearance (with its anchor), then Skin with a card per skin and the per-PC footnote", () => {
    const html = show(DEFAULT_SELECTION);
    expect(html).toContain('id="general.appearance"');
    expect(html.indexOf("Appearance")).toBeLessThan(html.indexOf("Skin"));
    for (const m of manifests) expect(html).toContain(m.name);
    expect(html.match(/class="w-skin( |")/g)).toHaveLength(manifests.length);
    expect(html).toContain("Each PC keeps its own skin.");
    expect(html).toContain("Import Skin…");
    expect(html.match(/aria-pressed="true"/g)).toHaveLength(1);
    expect(html).not.toContain("always uses");
  });

  it("offers Appearance as System, Light and Dark, locked with the reason in words for a dark-only palette", () => {
    const open = show(DEFAULT_SELECTION);
    for (const label of ["System", "Light", "Dark"]) expect(open).toContain(label);
    const retro = skins.find((d) => d.skin.id === "retro-terminal")!;
    const darkOnly = retro.palettes.find((p) => !p.light)!;
    const html = show({ skinId: "retro-terminal", paletteId: darkOnly.id, appearance: "light" });
    expect(html).toContain("This palette is dark only, so it always uses Dark.");
    expect(html).toContain('aria-label="Retro Terminal palettes"');
    expect(html.match(/class="w-setting-reason"/g)).toHaveLength(1);
  });

  it("gives the palette dots only to the selected skin, and a card id so an import can scroll to it", () => {
    const retro = skins.find((d) => d.skin.id === "retro-terminal")!;
    const html = show({ skinId: "retro-terminal", paletteId: retro.palettes[0].id, appearance: "dark" });
    // The Appearance choice is the other radio group, ahead of the skin cards.
    const cards = html.slice(html.indexOf('id="general.skin"'));
    expect(cards.match(/role="radiogroup"/g)).toHaveLength(1);
    expect(cards.match(/role="radio"/g)).toHaveLength(retro.palettes.length);
    expect(html).toContain('id="skin-card-retro-terminal"');
  });

  it("marks a skin you made, with a named options button, and waits on a running chat turn with a reason", () => {
    const base = skins[0];
    const mine: SkinDto = { skin: { ...base.skin, id: "user-1", name: "My skin" }, builtIn: false, palettes: base.palettes };
    const html = show(DEFAULT_SELECTION, false, [mine]);
    expect(html).toContain("My skin");
    expect(html).toContain(">Yours<");
    expect(html).toContain('aria-label="My skin options"');
    expect(html).not.toContain("wait until the skin chat finishes");
    const busy = show(DEFAULT_SELECTION, true, [mine]);
    expect(busy).toContain("Rename, export, delete and import wait until the skin chat finishes.");
    expect(busy).toMatch(/<button[^>]*disabled[^>]*>(<span>)?Import Skin…/);
  });
});
