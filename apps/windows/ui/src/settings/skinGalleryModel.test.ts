import { afterEach, describe, expect, it } from "vitest";
import { chooseSkin, DEFAULT_SELECTION, readSelection, setInstalledSkins, type Selection, type SkinDto, type SkinManifest, type SkinPalette } from "../skin";
import { fakeWindow } from "../testWindow";
import { THEMES } from "../themes";
import {
  actionsEnabled,
  afterDelete,
  afterImport,
  cardElementId,
  deletePrompt,
  galleryState,
  GALLERY_FOOTNOTE,
  lockedCaption,
  orderedSkins,
  pickCard,
  pickPalette,
  swatchColors,
} from "./skinGalleryModel";

const manifests = Object.values(import.meta.glob("../../../../../skins/*.json", { import: "default", eager: true }) as Record<string, SkinManifest>).filter((m) =>
  Array.isArray(m.palettes),
);
const pool: SkinPalette[] = [
  ...THEMES.map((t) => ({ id: t.id, name: t.name, author: t.author, light: t.light, dark: t.dark })),
  ...Object.values(import.meta.glob("../../../../../skins/palettes/*.json", { import: "default", eager: true }) as Record<string, SkinPalette>),
];
const dto = (skin: SkinManifest, builtIn = true): SkinDto => ({ skin, palettes: skin.palettes.flatMap((id) => pool.filter((p) => p.id === id)), builtIn });

// The shell's order: Whimbrel, then the rest by name.
const builtIns = [...manifests].sort((a, b) => (a.id === "whimbrel" ? -1 : b.id === "whimbrel" ? 1 : a.name.localeCompare(b.name))).map((m) => dto(m));
const mine = (id: string, name: string): SkinDto => ({ ...dto({ ...manifests.find((m) => m.id === "whimbrel")!, id, name }), builtIn: false });

describe("the gallery", () => {
  afterEach(() => setInstalledSkins([]));

  it("lists the seven built-ins in the Mac's order, Whimbrel first", () => {
    const cards = galleryState(builtIns, DEFAULT_SELECTION);
    expect(cards.map((c) => c.name)).toEqual(["Whimbrel", "Almanac", "Game Day", "High Contrast", "Hypermodern", "Reference", "Retro Terminal"]);
    expect(cards.every((c) => !c.yours)).toBe(true);
  });

  it("puts your skins after the built-ins, by name, marked as yours", () => {
    const cards = galleryState([...builtIns, mine("user-b", "Beta 10"), mine("user-a", "Beta 2")], DEFAULT_SELECTION);
    expect(cards.slice(7).map((c) => [c.name, c.yours])).toEqual([
      ["Beta 2", true],
      ["Beta 10", true],
    ]);
    expect(orderedSkins([mine("user-z", "Zed"), ...builtIns]).map((s) => s.skin.id).slice(0, 1)).toEqual(["whimbrel"]);
  });

  it("marks one card selected, offers its palettes, and gives the tagline", () => {
    const cards = galleryState(builtIns, { skinId: "whimbrel", paletteId: "slate", appearance: "system" });
    const selected = cards.filter((c) => c.selected);
    expect(selected.map((c) => c.id)).toEqual(["whimbrel"]);
    expect(selected[0].palettes.find((p) => p.current)?.id).toBe("slate");
    expect(selected[0].previewPalette.id).toBe("slate");
    expect(cards.filter((c) => !c.selected).every((c) => c.palettes.length === 0)).toBe(true);
    expect(selected[0].tagline).toBe("Your meetings, remembered.");
  });

  it("previews an unselected skin with its first palette", () => {
    const retro = galleryState(builtIns, DEFAULT_SELECTION).find((c) => c.id === "retro-terminal")!;
    expect(retro.previewPalette.id).toBe(builtIns.find((s) => s.skin.id === "retro-terminal")!.palettes[0].id);
  });

  it("falls back to Whimbrel when the selected skin is gone", () => {
    const cards = galleryState(builtIns, { skinId: "user-gone", paletteId: "x", appearance: "dark" });
    expect(cards.find((c) => c.selected)?.id).toBe("whimbrel");
  });

  it("locks the appearance, with the Mac's caption, for a dark-only palette", () => {
    const retro = builtIns.find((s) => s.skin.id === "retro-terminal")!;
    const darkOnly = retro.palettes.find((p) => !p.light && p.dark)!;
    expect(darkOnly, "retro terminal ships a dark-only palette").toBeTruthy();
    const cards = galleryState(builtIns, { skinId: "retro-terminal", paletteId: darkOnly.id, appearance: "light" });
    expect(cards.find((c) => c.id === "retro-terminal")!.darkOnlyLocked).toBe(true);
    expect(cards.filter((c) => c.darkOnlyLocked)).toHaveLength(1);
    expect(lockedCaption(darkOnly)).toBe("This palette is dark only, so it always uses Dark.");
  });

  it("words a light-only lock and leaves a two-variant palette unlocked", () => {
    const both = builtIns[0].palettes[0];
    expect(both.light && both.dark).toBeTruthy();
    expect(lockedCaption(both)).toBeNull();
    expect(lockedCaption({ ...both, dark: undefined })).toBe("This palette is light only, so it always uses Light.");
    expect(galleryState(builtIns, DEFAULT_SELECTION).some((c) => c.darkOnlyLocked)).toBe(false);
  });

  it("clicking a skin picks its first palette; clicking the selected one keeps its palette", () => {
    const slate: Selection = { skinId: "whimbrel", paletteId: "slate", appearance: "dark" };
    expect(pickCard(builtIns, slate, "whimbrel")).toEqual(slate);
    const next = pickCard(builtIns, slate, "almanac");
    expect(next).toEqual({ skinId: "almanac", paletteId: builtIns.find((s) => s.skin.id === "almanac")!.palettes[0].id, appearance: "dark" });
    expect(pickCard(builtIns, slate, "no-such-skin")).toEqual(slate);
  });

  it("picking a card applies immediately and is remembered on this PC", () => {
    const f = fakeWindow();
    setInstalledSkins(builtIns);
    const next = pickCard(builtIns, DEFAULT_SELECTION, "almanac");
    chooseSkin(next, f.win);
    expect(f.attrs.get("data-skin")).toBe("almanac");
    expect(readSelection(f.win)).toEqual(next);
    chooseSkin(pickPalette(next, "almanac", builtIns.find((s) => s.skin.id === "almanac")!.palettes.at(-1)!.id), f.win);
    expect(readSelection(f.win).paletteId).toBe(builtIns.find((s) => s.skin.id === "almanac")!.palettes.at(-1)!.id);
  });

  it("swatches show five colours, and the one variant a palette has", () => {
    const retro = builtIns.find((s) => s.skin.id === "retro-terminal")!.palettes.find((p) => !p.light)!;
    expect(swatchColors(retro, "light")).toEqual(swatchColors(retro, "dark"));
    expect(swatchColors(builtIns[0].palettes[0], "dark")).toHaveLength(5);
    expect(swatchColors({ id: "x", name: "x", author: "" }, "dark")).toEqual([]);
  });

  it("states that each PC keeps its own skin", () => {
    expect(GALLERY_FOOTNOTE).toBe("Each PC keeps its own skin.");
  });
});

describe("managing your skins", () => {
  it("actions disabled while a turn runs", () => {
    expect(actionsEnabled(true)).toBe(false);
    expect(actionsEnabled(false)).toBe(true);
  });

  it("delete selected falls back to whimbrel", () => {
    const showing: Selection = { skinId: "user-a-1", paletteId: "user-a-1-p1", appearance: "dark" };
    expect(afterDelete(showing, "user-a-1")).toEqual({ skinId: "whimbrel", paletteId: "default", appearance: "dark" });
    // Deleting another skin leaves the selection alone.
    expect(afterDelete(showing, "user-b-1")).toBe(showing);
  });

  it("import selects nothing and scrolls to card", () => {
    const showing: Selection = { skinId: "game-day", paletteId: "game-day", appearance: "system" };
    expect(afterImport(showing, "user-imported-1")).toEqual({ selection: showing, scrollTo: "skin-card-user-imported-1" });
    expect(cardElementId("user-a-1")).toBe("skin-card-user-a-1");
  });

  it("asks before deleting, saying what goes with it", () => {
    expect(deletePrompt("Stadium")).toBe("Delete \u201CStadium\u201D? Its chat and earlier versions go too.");
  });
});
