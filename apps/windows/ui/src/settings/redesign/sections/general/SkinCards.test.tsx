import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { chooseSkin, DEFAULT_SELECTION, setInstalledSkins, type SkinDto, type SkinManifest, type SkinPalette } from "../../../../skin";
import { fakeWindow } from "../../../../testWindow";
import { THEMES } from "../../../../themes";
import { AppearanceCard, SkinGalleryCard } from "./SkinCards";

// The bundled skins, without Thread, which these cases do not need.
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

/** What a screen reader gets from a piece of markup: its text, leaving out every element marked `aria-hidden="true"` and what is in it. */
function readable(markup: string): string {
  let out = "";
  let hiddenDepth = 0;
  const hiddenTags: string[] = [];
  for (const m of markup.matchAll(/<(\/?)([a-z0-9]+)([^>]*)>|([^<]+)/gi)) {
    if (m[4] !== undefined) {
      if (hiddenDepth === 0) out += m[4];
      continue;
    }
    const [, closing, tag, attrs] = m;
    if (/^(br|img|input|hr|path|svg)$/i.test(tag) && !closing) continue;
    if (closing) {
      if (hiddenDepth > 0 && hiddenTags[hiddenTags.length - 1] === tag) {
        hiddenTags.pop();
        hiddenDepth--;
      }
      continue;
    }
    if (hiddenDepth > 0 || /aria-hidden="true"/.test(attrs)) {
      hiddenDepth++;
      hiddenTags.push(tag);
    }
  }
  return out.replace(/\s+/g, " ").trim();
}

describe("the skin cards' Home preview (richer skins row 11)", () => {
  const mainButtons = (html: string): string[] => [...html.matchAll(/<button type="button" class="w-skin-main"[^>]*>([\s\S]*?)<\/button>/g)].map((m) => m[1]);
  /** The card's own button, by skin id. */
  const cardButton = (html: string, id: string): string => {
    const from = html.indexOf(`id="skin-card-${id}"`);
    expect(from, id).toBeGreaterThan(-1);
    return mainButtons(html.slice(from))[0];
  };

  it("draws the Home preview on every card, hidden from the accessibility tree", () => {
    const html = show(DEFAULT_SELECTION);
    const buttons = mainButtons(html);
    expect(buttons).toHaveLength(manifests.length);
    for (const inner of buttons) {
      expect(inner).toContain('<span class="w-skin-preview"><div class="skin-preview-frame" aria-hidden="true">');
      expect(inner).toContain('class="today th-home"');
    }
    expect(html.match(/class="skin-preview-frame" aria-hidden="true"/g)).toHaveLength(manifests.length);
    // No swatch strip any more: the picture carries the palette.
    expect(html).not.toContain("rs-general-swatches");
  });

  it("labels each card with its name and tagline and nothing from the picture", () => {
    const html = show(DEFAULT_SELECTION);
    manifests.forEach((m) => {
      const label = readable(cardButton(html, m.id));
      // The name and the tagline are two elements, one under the other.
      expect(label, m.id).toBe(`${m.name}${m.voice.tagline}`);
      expect(label, m.id).not.toContain("Design review");
      expect(label, m.id).not.toContain("Pipeline sync");
    });
  });

  it("puts no button inside the card's own button, so the picture cannot be pressed or tabbed to", () => {
    for (const inner of mainButtons(show(DEFAULT_SELECTION))) expect(inner).not.toContain("<button");
  });

  it("shows each skin's own picture whichever skin is selected, and the selected one in the palette in force", () => {
    const retro = skins.find((d) => d.skin.id === "retro-terminal")!;
    const html = show({ skinId: "retro-terminal", paletteId: retro.palettes[0].id, appearance: "dark" });
    const at = (id: string) => cardButton(html, id);
    expect(at("almanac")).toContain('data-skin="almanac"');
    expect(at("retro-terminal")).toContain('data-skin="retro-terminal"');
    expect(at("retro-terminal")).toContain('data-theme="dark"');
    expect(at("almanac")).not.toContain('data-skin="retro-terminal"');
  });
});
