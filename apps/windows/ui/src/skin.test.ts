import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_SELECTION,
  FONT_MAP,
  WHIMBREL,
  LEGACY_THEME_KEY,
  PALETTE_KEY,
  SKIN_KEY,
  WEIGHTS,
  applySkin,
  chooseSkin,
  controlRadius,
  currentSelection,
  installedSkins,
  migrateThemeId,
  paletteColors,
  paletteMode,
  parseFace,
  readSelection,
  reloadSkins,
  resolveSelection,
  saveSelection,
  setInstalledSkins,
  skinAttributes,
  skinVariables,
  startSkin,
  type SkinDto,
  type SkinManifest,
  type SkinPalette,
} from "./skin";
import { APPEARANCE_KEY, cssVariable } from "./theme";
import { THEMES, TOKENS, themeById } from "./themes";
import { fakeWindow } from "./testWindow";

const tokensCss = (await import("./tokens.css?raw")).default;

/** The format's lists as the Mac's catalogue states them (hand-derived; core/skins pins it to the Rust). */
const catalogue = Object.values(
  import.meta.glob("../../../../core/skins/tests/fixtures/catalogue.json", { import: "default", eager: true }),
)[0] as { fonts: { families: string[]; weights: string[] } };

/** Every bundled skin, with the palettes it names: the plain themes and the ones in skins/palettes. */
const bundledManifests = Object.values(import.meta.glob("../../../../skins/*.json", { import: "default", eager: true }) as Record<string, SkinManifest>).filter(
  (m) => Array.isArray(m.palettes),
);
const palettePool: SkinPalette[] = [
  ...THEMES.map((t) => ({ id: t.id, name: t.name, author: t.author, light: t.light, dark: t.dark })),
  ...Object.values(import.meta.glob("../../../../skins/palettes/*.json", { import: "default", eager: true }) as Record<string, SkinPalette>),
];
const dtoFor = (skin: SkinManifest): SkinDto => ({
  skin,
  palettes: skin.palettes.flatMap((id) => palettePool.filter((p) => p.id === id)),
  builtIn: true,
});

const slate = themeById("slate");

/** A copy of Whimbrel's manifest with some of it replaced. */
function variant(patch: Partial<Omit<SkinManifest, "shape" | "typography">> & { shape?: Partial<SkinManifest["shape"]>; typography?: Partial<SkinManifest["typography"]> }): SkinDto {
  const base = WHIMBREL.skin;
  const skin: SkinManifest = {
    ...base,
    ...patch,
    shape: { ...base.shape, ...patch.shape },
    typography: { ...base.typography, ...patch.typography },
  };
  return { ...WHIMBREL, skin };
}

const colorProps = (props: Map<string, string>) => [...props.keys()].filter((k) => k.startsWith("--color-"));

beforeEach(() => {
  setInstalledSkins([]);
});

describe("fonts", () => {
  it("maps every allowed font family", () => {
    expect(catalogue.fonts.families).toHaveLength(9);
    for (const family of catalogue.fonts.families) {
      const stack = FONT_MAP[family];
      expect(stack, family).toBeTruthy();
      // Each ends in a generic family, so a missing font still draws in the right kind of face.
      expect(stack, family).toMatch(/(sans-serif|serif|monospace)$/);
    }
    expect(Object.keys(FONT_MAP).sort()).toEqual([...catalogue.fonts.families].sort());
  });

  it("maps every weight to a CSS weight, ascending", () => {
    expect(Object.keys(WEIGHTS).sort()).toEqual([...catalogue.fonts.weights].sort());
    const order = ["ultraLight", "thin", "light", "regular", "medium", "semibold", "bold", "heavy", "black"];
    expect(order.map((w) => WEIGHTS[w])).toEqual([100, 200, 300, 400, 500, 600, 700, 800, 900]);
  });

  it("reads family, weight and italic from a face name", () => {
    expect(parseFace("New York/semibold")).toEqual({ stack: FONT_MAP["New York"], weight: 600, italic: false });
    expect(parseFace("Avenir Next/ultraLight/italic")).toEqual({ stack: FONT_MAP["Avenir Next"], weight: 100, italic: true });
    expect(parseFace("SF Mono").weight).toBe(400);
  });

  it("draws a family or weight it does not know as Segoe regular rather than failing", () => {
    expect(parseFace("Comic Sans/blackest")).toEqual({ stack: FONT_MAP["SF Pro"], weight: 400, italic: false });
    expect(parseFace("")).toEqual({ stack: FONT_MAP["SF Pro"], weight: 400, italic: false });
  });
});

describe("applying a skin", () => {
  it("applies every variable for whimbrel", () => {
    const f = fakeWindow();
    applySkin(WHIMBREL, "default", "dark", f.root as unknown as HTMLElement, false);
    const set = (name: string) => f.props.get(name);
    expect(set("--font-display")).toBe(FONT_MAP["New York"]);
    expect(set("--font-body")).toBe(FONT_MAP["SF Pro"]);
    expect(set("--font-numerals")).toBe(FONT_MAP["SF Pro"]);
    expect(set("--font-label")).toBe(FONT_MAP["SF Pro"]);
    expect(set("--weight-display")).toBe("600");
    expect(set("--weight-body")).toBe("400");
    expect(set("--style-display")).toBe("normal");
    expect(set("--caps")).toBe("uppercase");
    expect(set("--caps-variant")).toBe("normal");
    expect(set("--radius")).toBe("12px");
    expect(set("--radius-control")).toBe("8px");
    expect(set("--border-style")).toBe("solid");
    expect(set("--border-width")).toBe("1px");
    expect(set("--density")).toBe("1");
    expect(set("--motion")).toBe("none");
    expect(f.attrs.get("data-skin")).toBe("whimbrel");
    expect(f.attrs.get("data-archetype")).toBe("standard");
    expect(f.attrs.get("data-header-band")).toBe("none");
    expect(f.attrs.get("data-dividers")).toBe("none");
    expect(f.attrs.get("data-texture")).toBe("none");
    expect(f.attrs.get("data-motion")).toBe("none");
    // And the 15 colours, from the dark variant of Default.
    expect(colorProps(f.props)).toHaveLength(15);
    expect(set("--color-background-page")).toBe(themeById("default").dark.backgroundPage);
    expect(f.attrs.get("data-theme")).toBe("dark");
    expect(f.root.style.colorScheme).toBe("dark");
  });

  it("sets the colours of the palette asked for, in the mode asked for", () => {
    const f = fakeWindow();
    applySkin(WHIMBREL, "slate", "light", f.root as unknown as HTMLElement, true);
    for (const token of TOKENS) expect(f.props.get(cssVariable(token)), token).toBe(slate.light[token]);
  });

  it("leaves Whimbrel's Default with System appearance to tokens.css, but still sets the skin variables", () => {
    const f = fakeWindow();
    applySkin(WHIMBREL, "slate", "dark", f.root as unknown as HTMLElement, false);
    applySkin(WHIMBREL, "default", "system", f.root as unknown as HTMLElement, true);
    expect(colorProps(f.props)).toHaveLength(0);
    expect(f.attrs.has("data-theme")).toBe(false);
    expect(f.root.style.colorScheme).toBe("");
    expect(f.props.get("--radius")).toBe("12px");
  });

  it("draws the first palette when the one asked for is not offered", () => {
    const f = fakeWindow();
    applySkin(WHIMBREL, "no-such-palette", "light", f.root as unknown as HTMLElement, false);
    expect(f.props.get("--color-background-page")).toBe(themeById("default").light.backgroundPage);
  });

  it("follows the OS under System for any other palette, and an explicit appearance ignores it", () => {
    const f = fakeWindow();
    applySkin(WHIMBREL, "slate", "system", f.root as unknown as HTMLElement, true);
    expect(f.props.get("--color-background-page")).toBe(slate.dark.backgroundPage);
    expect(f.root.style.colorScheme).toBe("");
    applySkin(WHIMBREL, "slate", "light", f.root as unknown as HTMLElement, true);
    expect(f.props.get("--color-background-page")).toBe(slate.light.backgroundPage);
    expect(f.root.style.colorScheme).toBe("light");
  });

  it("derives shape, density, caps and motion from the skin", () => {
    const vars = skinVariables(
      variant({ shape: { radius: 15, border: "dashed", borderWidth: 2.5, density: "airy" }, typography: { caps: "small" }, motion: "subtle" }).skin,
    );
    expect(vars["--radius"]).toBe("15px");
    expect(vars["--radius-control"]).toBe("10px");
    expect(vars["--border-style"]).toBe("dashed");
    expect(vars["--border-width"]).toBe("2.5px");
    expect(vars["--density"]).toBe("1.25");
    expect(vars["--caps"]).toBe("none");
    expect(vars["--caps-variant"]).toBe("small-caps");
    expect(vars["--motion"]).toBe("subtle");
    expect(skinVariables(variant({ shape: { density: "compact" } }).skin)["--density"]).toBe("0.75");
    expect(skinVariables(variant({ typography: { caps: "none" } }).skin)["--caps"]).toBe("none");
    expect(skinVariables(variant({ typography: { caps: "none" } }).skin)["--caps-variant"]).toBe("normal");
  });

  it("rounds the control radius like core/skins: two thirds of the radius", () => {
    expect([0, 1, 2, 3, 4, 12, 15, 24].map(controlRadius)).toEqual([0, 1, 1, 2, 3, 8, 10, 16]);
  });

  it("carries weight and italic into their own variables", () => {
    const vars = skinVariables(variant({ typography: { label: "Georgia/black/italic" } }).skin);
    expect(vars["--font-label"]).toBe(FONT_MAP["Georgia"]);
    expect(vars["--weight-label"]).toBe("900");
    expect(vars["--style-label"]).toBe("italic");
  });

  it("names the skin's layout, band, dividers and texture", () => {
    const attrs = skinAttributes({
      ...WHIMBREL.skin,
      id: "x",
      today: { archetype: "glass", options: {} },
      decoration: { headerBand: "stripes-diagonal", dividers: "chalk", texture: "grain", imagery: [] },
    });
    expect(attrs).toEqual({ "data-skin": "x", "data-archetype": "glass", "data-header-band": "stripes-diagonal", "data-dividers": "chalk", "data-texture": "grain", "data-motion": "none" });
  });

  it("applies every bundled skin in both modes without a gap", () => {
    expect(bundledManifests.map((m) => m.id).sort()).toEqual(["almanac", "game-day", "high-contrast", "hypermodern", "reference", "retro-terminal", "whimbrel"]);
    for (const manifest of bundledManifests) {
      const dto = dtoFor(manifest);
      expect(dto.palettes.length, manifest.id).toBeGreaterThan(0);
      for (const palette of dto.palettes) {
        for (const appearance of ["light", "dark"] as const) {
          const f = fakeWindow();
          applySkin(dto, palette.id, appearance, f.root as unknown as HTMLElement, false);
          for (const [name, value] of f.props) expect(value, `${manifest.id} ${name}`).not.toBe("");
          expect(f.props.has("--font-display"), manifest.id).toBe(true);
          expect(f.attrs.get("data-archetype"), manifest.id).toBe(manifest.today.archetype);
        }
      }
    }
  });
});

describe("palettes that ship one variant", () => {
  const dark: SkinPalette = { id: "night", name: "Night", author: "", dark: slate.dark };
  const light: SkinPalette = { id: "day", name: "Day", author: "", light: slate.light };

  it("force the appearance they have", () => {
    expect(paletteMode(dark, "light", false)).toBe("dark");
    expect(paletteMode(dark, "system", false)).toBe("dark");
    expect(paletteMode(light, "dark", true)).toBe("light");
    expect(paletteMode({ ...dark, light: slate.light }, "light", true)).toBe("light");
    expect(paletteMode({ ...dark, light: slate.light }, "system", true)).toBe("dark");
  });

  it("draw all 15 colours from the variant that exists", () => {
    expect(Object.keys(paletteColors(dark, "light"))).toHaveLength(15);
    expect(paletteColors(dark, "light")["--color-text-primary"]).toBe(slate.dark.textPrimary);
    expect(paletteColors(light, "dark")["--color-text-primary"]).toBe(slate.light.textPrimary);
  });

  it("set the browser's color-scheme to the forced mode even under System", () => {
    const f = fakeWindow();
    applySkin({ ...WHIMBREL, palettes: [dark] }, "night", "system", f.root as unknown as HTMLElement, false);
    expect(f.attrs.get("data-theme")).toBe("dark");
    expect(f.root.style.colorScheme).toBe("dark");
  });
});

describe("migrating a plain theme", () => {
  it("migrates each old theme id", () => {
    expect(migrateThemeId("default")).toEqual({ skinId: "whimbrel", paletteId: "default" });
    expect(migrateThemeId("slate")).toEqual({ skinId: "whimbrel", paletteId: "slate" });
    expect(migrateThemeId("sepia")).toEqual({ skinId: "whimbrel", paletteId: "sepia" });
    expect(migrateThemeId("high-contrast")).toEqual({ skinId: "high-contrast", paletteId: "high-contrast" });
  });

  it("sends an id it does not know to Whimbrel's Default", () => {
    expect(migrateThemeId("someone-elses")).toEqual({ skinId: "whimbrel", paletteId: "default" });
    expect(migrateThemeId("")).toEqual({ skinId: "whimbrel", paletteId: "default" });
  });
});

describe("remembering the choice", () => {
  it("starts on Whimbrel, Default and System when nothing is stored", () => {
    expect(readSelection(fakeWindow().win)).toEqual(DEFAULT_SELECTION);
    expect(DEFAULT_SELECTION).toEqual({ skinId: "whimbrel", paletteId: "default", appearance: "system" });
  });

  it("round-trips a choice under skinId, paletteId and appearance, the keys the plan names", () => {
    const { win, data } = fakeWindow();
    saveSelection({ skinId: "game-day", paletteId: "purple-gold", appearance: "dark" }, win);
    expect(data).toEqual({ skinId: "game-day", paletteId: "purple-gold", appearance: "dark" });
    expect(readSelection(win)).toEqual({ skinId: "game-day", paletteId: "purple-gold", appearance: "dark" });
    expect([SKIN_KEY, PALETTE_KEY, APPEARANCE_KEY]).toEqual(["skinId", "paletteId", "appearance"]);
  });

  it("carries an old themeId over on first launch, and leaves it stored for a downgrade", () => {
    const { win, data } = fakeWindow({ stored: { themeId: "high-contrast", appearance: "dark" } });
    expect(readSelection(win)).toEqual({ skinId: "high-contrast", paletteId: "high-contrast", appearance: "dark" });
    expect(data).toEqual({ themeId: "high-contrast", appearance: "dark", skinId: "high-contrast", paletteId: "high-contrast" });
    expect(LEGACY_THEME_KEY).toBe("themeId");
  });

  it("does not read themeId once a skin is stored", () => {
    const { win } = fakeWindow({ stored: { themeId: "sepia", skinId: "whimbrel", paletteId: "slate" } });
    expect(readSelection(win).paletteId).toBe("slate");
  });

  it("uses Default when a skin is stored without a palette", () => {
    expect(readSelection(fakeWindow({ stored: { skinId: "almanac" } }).win)).toEqual({ skinId: "almanac", paletteId: "default", appearance: "system" });
  });

  it("treats an unknown appearance as System and keeps an unknown skin id as stored", () => {
    const { win } = fakeWindow({ stored: { skinId: "no-longer-installed", paletteId: "x", appearance: "sepia" } });
    expect(readSelection(win)).toEqual({ skinId: "no-longer-installed", paletteId: "x", appearance: "system" });
  });

  it("copes with storage that throws, on both read and write", () => {
    const { win } = fakeWindow({ storageThrows: true });
    expect(readSelection(win)).toEqual(DEFAULT_SELECTION);
    expect(() => saveSelection({ skinId: "almanac", paletteId: "paper", appearance: "dark" }, win)).not.toThrow();
  });
});

describe("which skin a choice shows", () => {
  const retro = dtoFor(bundledManifests.find((m) => m.id === "retro-terminal")!);
  const list = [WHIMBREL, retro];

  it("unknown stored skin falls back to whimbrel", () => {
    const { dto, palette } = resolveSelection(list, { skinId: "gone", paletteId: "default", appearance: "system" });
    expect(dto.skin.id).toBe("whimbrel");
    expect(palette.id).toBe("default");
  });

  it("shows the skin's first palette when the stored one is not offered", () => {
    const { dto, palette } = resolveSelection(list, { skinId: "retro-terminal", paletteId: "slate", appearance: "system" });
    expect(dto.skin.id).toBe("retro-terminal");
    expect(palette.id).toBe(retro.palettes[0].id);
  });

  it("shows exactly what was stored when it is installed", () => {
    const { dto, palette } = resolveSelection(list, { skinId: "whimbrel", paletteId: "sepia", appearance: "dark" });
    expect([dto.skin.id, palette.id]).toEqual(["whimbrel", "sepia"]);
  });

  it("always has Whimbrel first, whatever the shell returned", () => {
    setInstalledSkins([retro]);
    expect(installedSkins().map((s) => s.skin.id)).toEqual(["whimbrel", "retro-terminal"]);
    setInstalledSkins([retro, { ...WHIMBREL, builtIn: false }]);
    expect(installedSkins().map((s) => s.skin.id)).toEqual(["whimbrel", "retro-terminal"]);
    expect(installedSkins()[0].builtIn).toBe(false);
  });
});

describe("Whimbrel before the shell answers", () => {
  it("is the bundled manifest with the three plain themes it names", () => {
    expect(WHIMBREL.skin.id).toBe("whimbrel");
    expect(WHIMBREL.palettes.map((p) => p.id)).toEqual(["default", "slate", "sepia"]);
    expect(WHIMBREL.builtIn).toBe(true);
  });

  it("is what tokens.css already says, so the first paint needs no script", () => {
    const declared = (name: string): string | undefined => {
      const m = new RegExp(`${name}:\\s*([^;]+);`).exec(tokensCss);
      const value = m?.[1].trim();
      const alias = value && /^var\((--[a-z-]+)\)$/.exec(value);
      return alias ? declared(alias[1]) : value;
    };
    for (const [name, value] of Object.entries(skinVariables(WHIMBREL.skin))) expect(declared(name), name).toBe(value);
  });
});

describe("the page follows the choice", () => {
  const fixture = (id: string): SkinDto => dtoFor(bundledManifests.find((m) => m.id === id)!);

  it("paints Whimbrel's own look at once, with nothing from the shell", () => {
    const f = fakeWindow({ systemDark: true });
    startSkin(f.win, () => new Promise(() => {}));
    expect(colorProps(f.props)).toHaveLength(0); // Default and System: tokens.css alone
    expect(f.props.get("--radius")).toBe("12px");
    expect(f.attrs.get("data-archetype")).toBe("standard");
  });

  it("applies a stored plain theme before the shell answers", () => {
    const f = fakeWindow({ stored: { themeId: "slate", appearance: "dark" } });
    startSkin(f.win, () => new Promise(() => {}));
    expect(f.props.get("--color-background-page")).toBe(slate.dark.backgroundPage);
    expect(f.attrs.get("data-theme")).toBe("dark");
  });

  it("switches to the stored skin once the shell answers", async () => {
    const f = fakeWindow({ stored: { skinId: "retro-terminal", paletteId: fixture("retro-terminal").palettes[0].id, appearance: "light" } });
    const load = vi.fn(async () => [WHIMBREL, fixture("retro-terminal")]);
    startSkin(f.win, load);
    expect(f.attrs.get("data-skin")).toBe("whimbrel"); // not known yet: Whimbrel's look
    await vi.waitFor(() => expect(f.attrs.get("data-skin")).toBe("retro-terminal"));
    expect(f.attrs.get("data-archetype")).toBe("terminal");
    expect(f.props.get("--font-body")).toBe(FONT_MAP["SF Mono"]);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it("keeps Whimbrel, without throwing, when the shell fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const f = fakeWindow({ stored: { skinId: "retro-terminal" } });
    startSkin(f.win, async () => {
      throw new Error("shell unavailable");
    });
    await vi.waitFor(() => expect(warn).toHaveBeenCalled());
    expect(f.attrs.get("data-skin")).toBe("whimbrel");
    warn.mockRestore();
  });

  it("follows the OS live under System, and stops following once the explicit choice is made", () => {
    const f = fakeWindow({ stored: { skinId: "whimbrel", paletteId: "slate", appearance: "system" } });
    startSkin(f.win, () => new Promise(() => {}));
    expect(f.props.get("--color-background-page")).toBe(slate.light.backgroundPage);
    f.flipSystem(true);
    expect(f.props.get("--color-background-page")).toBe(slate.dark.backgroundPage);
    chooseSkin({ skinId: "whimbrel", paletteId: "slate", appearance: "light" }, f.win);
    f.flipSystem(false);
    f.flipSystem(true);
    expect(f.props.get("--color-background-page")).toBe(slate.light.backgroundPage);
  });

  it("repaints when another window changes the choice, and ignores unrelated storage", () => {
    const f = fakeWindow();
    startSkin(f.win, () => new Promise(() => {}));
    f.data[SKIN_KEY] = "whimbrel";
    f.data[PALETTE_KEY] = "sepia";
    f.data[APPEARANCE_KEY] = "dark";
    f.storageEvent("unrelated");
    expect(colorProps(f.props)).toHaveLength(0);
    f.storageEvent(PALETTE_KEY);
    expect(f.props.get("--color-background-page")).toBe(themeById("sepia").dark.backgroundPage);
    f.data[APPEARANCE_KEY] = "light";
    f.storageEvent(null); // storage.clear() reports a null key
    expect(f.props.get("--color-background-page")).toBe(themeById("sepia").light.backgroundPage);
  });

  it("stops listening, and ignores a late answer, when asked to", async () => {
    const f = fakeWindow({ stored: { skinId: "retro-terminal" } });
    let answer: (list: SkinDto[]) => void = () => {};
    const stop = startSkin(f.win, () => new Promise((resolve) => (answer = resolve)));
    expect(f.listeners()).toBe(2);
    stop();
    expect(f.listeners()).toBe(0);
    answer([WHIMBREL, fixture("retro-terminal")]);
    await Promise.resolve();
    expect(f.attrs.get("data-skin")).toBe("whimbrel");
  });

  it("remembers a pick and shows it", () => {
    const f = fakeWindow();
    startSkin(f.win, () => new Promise(() => {}));
    chooseSkin({ skinId: "whimbrel", paletteId: "sepia", appearance: "dark" }, f.win);
    expect(f.data).toMatchObject({ skinId: "whimbrel", paletteId: "sepia", appearance: "dark" });
    expect(f.props.get("--color-background-page")).toBe(themeById("sepia").dark.backgroundPage);
    expect(currentSelection()).toEqual({ skinId: "whimbrel", paletteId: "sepia", appearance: "dark" });
  });

  it("still applies a pick for this session when storage is blocked, and an OS change does not undo it", () => {
    const f = fakeWindow({ storageThrows: true });
    startSkin(f.win, () => new Promise(() => {}));
    chooseSkin({ skinId: "whimbrel", paletteId: "slate", appearance: "system" }, f.win);
    f.flipSystem(true);
    expect(f.props.get("--color-background-page")).toBe(slate.dark.backgroundPage);
  });

  it("clears every colour when the user goes back to Whimbrel's Default with System", () => {
    const f = fakeWindow();
    startSkin(f.win, () => new Promise(() => {}));
    chooseSkin({ skinId: "whimbrel", paletteId: "slate", appearance: "dark" }, f.win);
    expect(colorProps(f.props)).toHaveLength(15);
    chooseSkin(DEFAULT_SELECTION, f.win);
    expect(colorProps(f.props)).toHaveLength(0);
    expect(f.attrs.has("data-theme")).toBe(false);
    expect(f.root.style.colorScheme).toBe("");
  });

  it("falls back to Whimbrel when the shown skin is deleted and the list is reloaded", async () => {
    const f = fakeWindow({ stored: { skinId: "retro-terminal" } });
    startSkin(f.win, async () => [WHIMBREL, fixture("retro-terminal")]);
    await vi.waitFor(() => expect(f.attrs.get("data-skin")).toBe("retro-terminal"));
    await reloadSkins(f.win, async () => [WHIMBREL]);
    expect(f.attrs.get("data-skin")).toBe("whimbrel");
    expect(f.data[SKIN_KEY]).toBe("retro-terminal"); // kept, in case it returns
  });

  it("knows every bundled plain theme through Whimbrel and High Contrast", () => {
    for (const theme of THEMES) {
      const { skinId, paletteId } = migrateThemeId(theme.id);
      const f = fakeWindow({ stored: { skinId, paletteId, appearance: "light" } });
      setInstalledSkins([fixture("high-contrast")]);
      startSkin(f.win, () => new Promise(() => {}));
      expect(f.props.get("--color-text-primary"), theme.id).toBe(theme.light.textPrimary);
    }
  });
});
