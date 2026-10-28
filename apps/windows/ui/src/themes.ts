// The theme format and its loader: a port of the Mac's `ThemeLoader`, so both apps accept and
// reject the same files (the corpus in themes/invalid/README.md; the rules in
// docs/superpowers/specs/2026-10-06-themes-design.md). Pure: nothing here touches the DOM.

export const TOKENS = [
  "backgroundPage",
  "backgroundCard",
  "border",
  "textPrimary",
  "textSecondary",
  "accentPrimary",
  "accentSecondary",
  "accentHighlight",
  "error",
  "warning",
  "success",
  "link",
  "selection",
  "highlight",
  "textOnAccent",
] as const;

export type ThemeToken = (typeof TOKENS)[number];

/** All 15 colors of one appearance, each `#rrggbb` in lowercase. */
export type Palette = Record<ThemeToken, string>;

export interface Theme {
  id: string;
  name: string;
  author: string;
  /** Both palettes always hold all 15 tokens: whatever a file left out came from Default. */
  light: Palette;
  dark: Palette;
}

/** Where a problem with a theme file is reported. */
export type Report = (message: string) => void;

// --- contrast ---------------------------------------------------------------------------------

export const MINIMUM_CONTRAST = 4.5;

/** (foreground, background) pairs a variant must keep readable. */
export const REQUIRED_PAIRS: ReadonlyArray<readonly [ThemeToken, ThemeToken]> = [
  ["textPrimary", "backgroundPage"],
  ["textPrimary", "backgroundCard"],
  ["textSecondary", "backgroundPage"],
  ["textSecondary", "backgroundCard"],
  ["textPrimary", "selection"],
  ["textPrimary", "highlight"],
  ["textOnAccent", "accentPrimary"],
];

function channel(value: number): number {
  const c = value / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function relativeLuminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  return 0.2126 * channel((n >> 16) & 0xff) + 0.7152 * channel((n >> 8) & 0xff) + 0.0722 * channel(n & 0xff);
}

/** WCAG contrast ratio of two `#rrggbb` colors, 1 to 21. */
export function contrastRatio(a: string, b: string): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** One line per failing pair, e.g. `textOnAccent on accentPrimary = 3.26`. Empty when it passes. */
export function contrastFailures(palette: Palette): string[] {
  return REQUIRED_PAIRS.flatMap(([fg, bg]) => {
    const ratio = contrastRatio(palette[fg], palette[bg]);
    return ratio < MINIMUM_CONTRAST ? [`${fg} on ${bg} = ${ratio.toFixed(2)}`] : [];
  });
}

// --- parsing ----------------------------------------------------------------------------------

const HEX = /^#[0-9a-fA-F]{6}$/;
const ID = /^[a-z0-9-]+$/;

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/** The colors a variant sets, or null when any known token's value is not `#rrggbb`. Unknown keys are ignored. */
function overrides(raw: Record<string, unknown>): Partial<Palette> | null {
  const out: Partial<Palette> = {};
  for (const token of TOKENS) {
    const value = raw[token];
    if (value === undefined) continue;
    if (typeof value !== "string" || !HEX.test(value)) return null;
    out[token] = value.toLowerCase();
  }
  return out;
}

/** A theme file's text to a complete `Theme`, or null (reported) when the file is unusable. A variant
 *  that fails the contrast guard is replaced by `fallback`'s variant of the same kind. */
export function parseTheme(text: string, fallback: Theme, report: Report = console.warn): Theme | null {
  let root: unknown;
  try {
    root = JSON.parse(text);
  } catch {
    report("theme skipped: not valid JSON");
    return null;
  }
  if (!isObject(root) || typeof root.id !== "string" || !ID.test(root.id) || typeof root.name !== "string" || root.name === "") {
    report("theme skipped: not a JSON object with a valid id and name");
    return null;
  }
  const id = root.id;
  const lightRaw = isObject(root.light) ? root.light : undefined;
  const darkRaw = isObject(root.dark) ? root.dark : undefined;
  if (!lightRaw && !darkRaw) {
    report(`theme ${id} skipped: no light or dark variant`);
    return null;
  }

  let light: Palette | undefined;
  let dark: Palette | undefined;
  if (lightRaw) {
    const set = overrides(lightRaw);
    if (!set) {
      report(`theme ${id} skipped: a light color is not #rrggbb`);
      return null;
    }
    light = { ...fallback.light, ...set };
  }
  if (darkRaw) {
    const set = overrides(darkRaw);
    if (!set) {
      report(`theme ${id} skipped: a dark color is not #rrggbb`);
      return null;
    }
    dark = { ...fallback.dark, ...set };
  }

  // A theme that ships one variant serves both appearances.
  let lightPalette = light ?? dark;
  let darkPalette = dark ?? light;
  if (!lightPalette || !darkPalette) return null;
  const lightFailures = contrastFailures(lightPalette);
  if (lightFailures.length > 0) {
    report(`theme ${id}: light variant fails the contrast guard, using Default's: ${lightFailures.join("; ")}`);
    lightPalette = fallback.light;
  }
  const darkFailures = contrastFailures(darkPalette);
  if (darkFailures.length > 0) {
    report(`theme ${id}: dark variant fails the contrast guard, using Default's: ${darkFailures.join("; ")}`);
    darkPalette = fallback.dark;
  }
  return { id, name: root.name, author: typeof root.author === "string" ? root.author : "", light: lightPalette, dark: darkPalette };
}

/** Default: the one theme that must be complete and readable on its own, since every other
 *  theme falls back to it. A defect here is a bug in themes/default.json, so it throws. */
export function parseDefaultTheme(text: string): Theme {
  const root: unknown = JSON.parse(text);
  const variant = (kind: "light" | "dark"): Palette => {
    const raw = isObject(root) && isObject(root[kind]) ? overrides(root[kind]) : null;
    if (!raw || TOKENS.some((token) => raw[token] === undefined)) {
      throw new Error(`themes/default.json: ${kind} must set all ${TOKENS.length} colors as #rrggbb`);
    }
    const palette = raw as Palette;
    const failures = contrastFailures(palette);
    if (failures.length > 0) throw new Error(`themes/default.json: ${kind} fails the contrast guard: ${failures.join("; ")}`);
    return palette;
  };
  return { id: "default", name: "Default", author: "Whimbrel", light: variant("light"), dark: variant("dark") };
}

const fileName = (path: string) => path.slice(path.lastIndexOf("/") + 1);
const byCodeUnits = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

/** Every theme in `files` (path to file text), Default first and then by name. Broken files and
 *  duplicate ids are skipped and reported; an empty set still yields Default. */
export function loadThemes(files: Record<string, string>, fallback: Theme, report: Report = console.warn): Theme[] {
  const seen = new Set([fallback.id]);
  const loaded: Theme[] = [];
  for (const path of Object.keys(files).sort((a, b) => byCodeUnits(fileName(a), fileName(b)))) {
    const name = fileName(path);
    const theme = parseTheme(files[path], fallback, (message) => report(`${name}: ${message}`));
    if (!theme) continue;
    if (seen.has(theme.id)) {
      report(`${name}: theme ${theme.id} skipped: that id is already taken`);
      continue;
    }
    seen.add(theme.id);
    loaded.push(theme);
  }
  return [fallback, ...loaded.sort((a, b) => byCodeUnits(a.name, b.name))];
}

// --- the themes that ship ---------------------------------------------------------------------

const bundled = import.meta.glob("../../../../themes/*.json", { query: "?raw", import: "default", eager: true }) as Record<
  string,
  string
>;

const DEFAULT_PATH = Object.keys(bundled).find((path) => fileName(path) === "default.json");
if (!DEFAULT_PATH) throw new Error("themes/default.json is missing from the build");

export const DEFAULT_THEME: Theme = parseDefaultTheme(bundled[DEFAULT_PATH]);

/** Default first, then the other bundled themes by name. */
export const THEMES: Theme[] = loadThemes(
  Object.fromEntries(Object.entries(bundled).filter(([path]) => path !== DEFAULT_PATH)),
  DEFAULT_THEME,
);

/** The bundled theme with this id, or Default when it is unknown (a stored choice that no longer exists). */
export function themeById(id: string | null | undefined): Theme {
  return THEMES.find((theme) => theme.id === id) ?? DEFAULT_THEME;
}
