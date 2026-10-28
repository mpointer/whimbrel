import { describe, expect, it } from "vitest";
import {
  DEFAULT_THEME,
  THEMES,
  TOKENS,
  contrastFailures,
  contrastRatio,
  loadThemes,
  parseDefaultTheme,
  parseTheme,
  themeById,
  type Palette,
} from "./themes";

// themes/ is the shared corpus: the Mac's ThemeLoaderTests read the same files, with the
// expected outcomes in themes/invalid/README.md.
const files = (glob: Record<string, unknown>) => glob as Record<string, string>;
const invalid = files(import.meta.glob("../../../../themes/invalid/*.json", { query: "?raw", import: "default", eager: true }));
const bundled = files(import.meta.glob("../../../../themes/*.json", { query: "?raw", import: "default", eager: true }));

const textOf = (set: Record<string, string>, name: string): string => {
  const path = Object.keys(set).find((p) => p.endsWith(`/${name}`));
  if (!path) throw new Error(`${name} is not in the corpus`);
  return set[path];
};

/** Parses a corpus file against Default, collecting what the loader reports. */
function parse(name: string, set = invalid) {
  const reports: string[] = [];
  const theme = parseTheme(textOf(set, name), DEFAULT_THEME, (m) => reports.push(m));
  return { theme, reports };
}

const HEX = /^#[0-9a-f]{6}$/;

describe("the shared invalid corpus", () => {
  it("is all there", () => {
    expect(Object.keys(invalid).map((p) => p.slice(p.lastIndexOf("/") + 1)).sort()).toEqual([
      "bad-hex.json",
      "legacy-12-token.json",
      "low-contrast.json",
      "missing-tokens.json",
      "no-id.json",
      "no-variants.json",
      "not-json.json",
      "single-variant.json",
    ]);
  });

  it.each(["not-json.json", "no-id.json", "no-variants.json", "bad-hex.json"])("skips %s and says why", (name) => {
    const { theme, reports } = parse(name);
    expect(theme).toBeNull();
    expect(reports).toHaveLength(1);
  });

  it("replaces a low-contrast variant with Default's and keeps the good one", () => {
    const { theme, reports } = parse("low-contrast.json");
    expect(theme?.id).toBe("low-contrast");
    expect(theme?.light).toEqual(DEFAULT_THEME.light);
    expect(theme?.dark).toEqual({ ...DEFAULT_THEME.dark, backgroundPage: "#000000", textPrimary: "#ffffff" });
    expect(reports).toHaveLength(1);
    expect(reports[0]).toContain("light variant fails the contrast guard");
    expect(reports[0]).toContain("textPrimary on backgroundPage");
  });

  it("fills missing tokens from Default and lets a single light variant serve both appearances", () => {
    const { theme, reports } = parse("missing-tokens.json");
    expect(theme?.light).toEqual({ ...DEFAULT_THEME.light, accentPrimary: "#336699" });
    expect(theme?.dark).toEqual(theme?.light);
    expect(reports).toEqual([]);
  });

  it("lets a single dark variant serve light mode too, with missing tokens from Default's dark", () => {
    const { theme, reports } = parse("single-variant.json");
    expect(theme?.dark).toEqual({
      ...DEFAULT_THEME.dark,
      backgroundPage: "#101010",
      backgroundCard: "#181818",
      textPrimary: "#f0f0f0",
      textSecondary: "#c8c8c8",
      selection: "#2a2a2a",
      highlight: "#3a3a20",
    });
    expect(theme?.light).toEqual(theme?.dark);
    expect(reports).toEqual([]);
  });

  it("gives a 12-token theme Default's selection, highlight and textOnAccent", () => {
    const { theme, reports } = parse("legacy-12-token.json");
    expect(theme?.light.accentPrimary).toBe("#1f6f4a");
    expect(theme?.light.selection).toBe(DEFAULT_THEME.light.selection);
    expect(theme?.light.highlight).toBe(DEFAULT_THEME.light.highlight);
    expect(theme?.light.textOnAccent).toBe(DEFAULT_THEME.light.textOnAccent);
    expect(theme?.dark).toEqual(theme?.light);
    expect(reports).toEqual([]);
  });
});

describe("parsing details", () => {
  const theme = (body: object, fallback = DEFAULT_THEME) => parseTheme(JSON.stringify(body), fallback, () => {});
  const light = { accentPrimary: "#336699" };
  const dark = { accentPrimary: "#336699" };

  it("accepts ids of lowercase letters, digits and hyphens only", () => {
    expect(theme({ id: "night-2", name: "N", light })?.id).toBe("night-2");
    for (const id of ["", "Night", "night_2", "night 2", "nuit-é", 7]) {
      expect(theme({ id, name: "N", light }), JSON.stringify(id)).toBeNull();
    }
  });

  it("needs a non-empty name", () => {
    expect(theme({ id: "x", name: "", light })).toBeNull();
    expect(theme({ id: "x", light })).toBeNull();
  });

  it("ignores unknown keys, in the file and in a variant", () => {
    const t = theme({ id: "x", name: "X", font: "Comic Sans", light: { ...light, accentPurple: "#ff00ff" } });
    expect(t?.light.accentPrimary).toBe("#336699");
    expect(Object.keys(t?.light ?? {}).sort()).toEqual([...TOKENS].sort());
  });

  it("lowercases colors and defaults a missing author to empty", () => {
    const t = theme({ id: "x", name: "X", light: { link: "#ABCDEF" } });
    expect(t?.light.link).toBe("#abcdef");
    expect(t?.author).toBe("");
  });

  it("skips the whole file when a known token is not #rrggbb, whatever its type", () => {
    for (const value of ["#fff", "#GGGGGG", "336699", "#33669900", 3, null, true, ["#336699"]]) {
      expect(theme({ id: "x", name: "X", light: { accentPrimary: value } }), JSON.stringify(value)).toBeNull();
    }
  });

  it("treats a variant that is not an object as absent", () => {
    expect(theme({ id: "x", name: "X", light: [], dark: "dark" })).toBeNull();
    expect(theme({ id: "x", name: "X", light: "no", dark })).not.toBeNull();
  });

  it("is not fooled by a JSON value that is not an object", () => {
    for (const text of ["[]", "null", "7", '"theme"', ""]) expect(parseTheme(text, DEFAULT_THEME, () => {})).toBeNull();
  });
});

describe("contrast", () => {
  it("matches the WCAG known values", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrastRatio("#336699", "#336699")).toBeCloseTo(1, 5);
    expect(contrastRatio("#ffffff", "#767676")).toBeCloseTo(4.54, 2);
    expect(contrastRatio("#ffffff", "#777777")).toBeLessThan(4.5);
  });

  it("is symmetric", () => {
    expect(contrastRatio("#c97b54", "#1c1712")).toBeCloseTo(contrastRatio("#1c1712", "#c97b54"), 10);
  });

  it("rejects white on Default's dark accent, the reason Default's dark textOnAccent is near-black", () => {
    expect(contrastRatio("#ffffff", DEFAULT_THEME.dark.accentPrimary)).toBeCloseTo(3.26, 2);
    expect(contrastFailures({ ...DEFAULT_THEME.dark, textOnAccent: "#ffffff" })).toEqual(["textOnAccent on accentPrimary = 3.26"]);
  });

  it("names every failing pair, including the selection and highlight ones", () => {
    const bad: Palette = { ...DEFAULT_THEME.light, selection: "#241e19", highlight: "#241e19" };
    expect(contrastFailures(bad)).toEqual(["textPrimary on selection = 1.00", "textPrimary on highlight = 1.00"]);
  });

  it("passes every bundled variant, Default included", () => {
    for (const theme of THEMES) {
      expect(contrastFailures(theme.light), `${theme.id} light`).toEqual([]);
      expect(contrastFailures(theme.dark), `${theme.id} dark`).toEqual([]);
    }
  });
});

describe("the bundled themes", () => {
  it("are Default first, then the rest by name", () => {
    expect(THEMES[0]).toBe(DEFAULT_THEME);
    expect(THEMES.map((t) => t.id).sort()).toEqual(["default", "high-contrast", "sepia", "slate"]);
    const rest = THEMES.slice(1).map((t) => t.name);
    expect(rest).toEqual([...rest].sort());
  });

  it("all hold the 15 tokens as #rrggbb in both appearances", () => {
    for (const theme of THEMES) {
      for (const palette of [theme.light, theme.dark]) {
        expect(Object.keys(palette).sort(), theme.id).toEqual([...TOKENS].sort());
        for (const token of TOKENS) expect(palette[token], `${theme.id} ${token}`).toMatch(HEX);
      }
    }
  });

  it("load as written: none needs a repair or is skipped", () => {
    for (const path of Object.keys(bundled)) {
      const reports: string[] = [];
      const theme = parseTheme(bundled[path], DEFAULT_THEME, (m) => reports.push(m));
      expect(theme, path).not.toBeNull();
      expect(reports, path).toEqual([]);
    }
  });

  it("find a theme by id and fall back to Default for an unknown, empty or missing one", () => {
    expect(themeById("slate").id).toBe("slate");
    for (const id of ["gone", "", null, undefined]) expect(themeById(id)).toBe(DEFAULT_THEME);
  });
});

describe("loading a folder", () => {
  const theme = (id: string, name: string, accent = "#336699") => JSON.stringify({ id, name, light: { accentPrimary: accent } });

  it("always yields Default, even from nothing", () => {
    expect(loadThemes({}, DEFAULT_THEME)).toEqual([DEFAULT_THEME]);
  });

  it("puts Default first, then sorts by name, whatever order the files come in", () => {
    const themes = loadThemes({ "t/a.json": theme("zeta", "Zeta"), "t/b.json": theme("alpha", "Alpha") }, DEFAULT_THEME);
    expect(themes.map((t) => t.id)).toEqual(["default", "alpha", "zeta"]);
  });

  it("skips broken files and reports them by file name", () => {
    const reports: string[] = [];
    const themes = loadThemes({ "t/ok.json": theme("ok", "Ok"), "t/bad.json": "{ nope" }, DEFAULT_THEME, (m) => reports.push(m));
    expect(themes.map((t) => t.id)).toEqual(["default", "ok"]);
    expect(reports).toHaveLength(1);
    expect(reports[0]).toContain("bad.json");
  });

  it("keeps the first of two files with one id, by file name, and never lets a file replace Default", () => {
    const reports: string[] = [];
    const themes = loadThemes(
      { "t/2.json": theme("dup", "Second", "#111111"), "t/1.json": theme("dup", "First", "#222222"), "t/d.json": theme("default", "Imposter") },
      DEFAULT_THEME,
      (m) => reports.push(m),
    );
    expect(themes.map((t) => t.name)).toEqual(["Default", "First"]);
    expect(themes[1].light.accentPrimary).toBe("#222222");
    expect(reports).toHaveLength(2);
  });
});

describe("Default", () => {
  it("is built from themes/default.json and is complete", () => {
    const fromFile = parseDefaultTheme(textOf(bundled, "default.json"));
    expect(fromFile).toEqual(DEFAULT_THEME);
    expect(DEFAULT_THEME.id).toBe("default");
  });

  it("refuses a Default that is incomplete or unreadable, since everything falls back to it", () => {
    const missing = JSON.stringify({ id: "default", name: "Default", light: DEFAULT_THEME.light });
    expect(() => parseDefaultTheme(missing)).toThrow(/dark must set all 15 colors/);
    const lowContrast = JSON.stringify({
      id: "default",
      name: "Default",
      light: { ...DEFAULT_THEME.light, textPrimary: "#eeeeee" },
      dark: DEFAULT_THEME.dark,
    });
    expect(() => parseDefaultTheme(lowContrast)).toThrow(/light fails the contrast guard/);
  });
});
