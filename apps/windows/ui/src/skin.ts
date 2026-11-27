// Applying a skin: which one is chosen, where that is remembered, and how it reaches the page
// (docs/superpowers/specs/2026-10-22-skins-windows-and-sync-design.md). The rules and the files are
// in core/skins; this draws what `skins_list` returns, from CSS variables and data attributes on the
// root element. Colours come from the skin's palette; everything else from the skin.
//
// Whimbrel (shown as Classic) with its Default palette under System appearance is left to tokens.css alone,
// exactly as before skins existed. Every other choice sets the variables, and follows the OS live when the
// appearance is System.
//
// Two rules sit on top of the choice (docs/ui-redesign/screens/switch-removal.md, A2). Once per device, a person
// still on the cream default is moved to Thread (`moveToThreadOnce`). And while the system asks for more contrast,
// every screen draws the High Contrast palette whatever skin is picked, without changing what is stored.

import { skinsList, skinsRevision } from "./api";
import { APPEARANCE_KEY, cssVariable, parseAppearance, resolveMode, type Appearance, type Mode, type ThemeWindow } from "./theme";
import { THEMES, TOKENS, type Palette } from "./themes";

// --- what `skins_list` returns ----------------------------------------------------------------

/** `Family`, `Family/weight` or `Family/weight/italic`. */
export type FaceName = string;

/** A skin manifest as `skins/*.json` writes it (core/skins `Skin`). Only what the page reads is typed. */
export interface SkinManifest {
  id: string;
  name: string;
  author: string;
  palettes: string[];
  typography: { display: FaceName; body: FaceName; numerals: FaceName; label: FaceName; caps: string };
  shape: { radius: number; border: string; borderWidth: number; density: string };
  icons: Record<string, string>;
  decoration: {
    headerBand: string;
    dividers: string;
    texture: string;
    imagery: { asset: string; placement: string; textFree: boolean }[];
  };
  voice: Record<string, string>;
  /** The skin's own wording per language (`es`, `fr-CA`), each key optional. */
  voiceLocalized?: Record<string, Record<string, string>>;
  today: { archetype: string; options: Record<string, string> };
  motion: string;
}

/** A palette as the shell sends it: only the variants it ships, each with all 15 colours. */
export interface SkinPalette {
  id: string;
  name: string;
  author: string;
  light?: Palette;
  dark?: Palette;
}

export interface SkinDto {
  skin: SkinManifest;
  /** In the skin's own order, already repaired for its Today layout. Never empty. */
  palettes: SkinPalette[];
  builtIn: boolean;
  /** How strongly the page colour must cover each background image (by asset name) for text to read at
   *  4.5:1, the worst over the palettes offered. The shell measures it when listing the skins; an image it
   *  could not measure is missing, and is fully covered. */
  imageScrims?: Record<string, number>;
}

// --- fonts ------------------------------------------------------------------------------------

const SEGOE = '"Segoe UI Variable Text", "Segoe UI Variable", "Segoe UI", system-ui, sans-serif';
const MONO = '"Cascadia Mono", Consolas, monospace';

/** The format's family names are Apple's; each maps to a font every supported Windows ships, so no font
 *  is downloaded (spec, "Fonts"). The Text optical size comes first so Whimbrel's body text is what it
 *  was before skins. */
export const FONT_MAP: Record<string, string> = {
  "SF Pro": SEGOE,
  "SF Pro Rounded": SEGOE, // Windows has no rounded system face; an accepted difference
  "SF Mono": MONO,
  Menlo: MONO,
  "New York": "Cambria, Georgia, serif",
  Georgia: "Georgia, serif",
  "Courier New": '"Courier New", monospace',
  "Helvetica Neue": "Arial, sans-serif",
  "Avenir Next": SEGOE,
};

/** CSS `font-weight` for each weight name in the format. */
export const WEIGHTS: Record<string, number> = {
  ultraLight: 100,
  thin: 200,
  light: 300,
  regular: 400,
  medium: 500,
  semibold: 600,
  bold: 700,
  heavy: 800,
  black: 900,
};

export interface Face {
  stack: string;
  weight: number;
  italic: boolean;
}

/** A face name to CSS. A family or weight this build does not know draws as Segoe regular rather than
 *  failing: a skin made on the Mac must always draw. */
export function parseFace(name: FaceName): Face {
  const [family = "", weight = "", italic = ""] = name.split("/");
  return { stack: FONT_MAP[family] ?? SEGOE, weight: WEIGHTS[weight] ?? 400, italic: italic === "italic" };
}

// --- variables --------------------------------------------------------------------------------

const DENSITY: Record<string, number> = { compact: 0.75, regular: 1, airy: 1.25 };

/** Body emphasis: two steps (200) above the body weight, up to 800. A body already heavier than 800 keeps its own weight,
 *  so emphasis is never lighter than the text it sits in (docs/superpowers/plans/2026-11-25-richer-skins.md, decision 6). */
export const emphasisWeight = (body: number): number => Math.max(body, Math.min(body + 200, 800));

/** The control radius is two thirds of the radius, rounded (core/skins `Shape::control_radius`). */
export const controlRadius = (radius: number): number => Math.round((radius * 2) / 3);

/** The surface radius is one and a half times the radius, as on the Mac. Thread keeps the 16px it has always drawn
 *  (its own radius is 8, which would make 12). */
export const surfaceRadius = (skin: SkinManifest): number => (skin.id === THREAD_ID ? 16 : skin.shape.radius * 1.5);

/** A double border: CSS draws two lines and a gap from `border-style: double`, which needs three times the line width, so
 *  the width is tripled, with each line and the gap at most 2px (6px in all, Mike 2026-11-25), and `--border-trim` is the
 *  extra two line widths a side, which padding gives back so the box keeps
 *  its size (docs/superpowers/plans/2026-11-25-richer-skins.md, section 2). A line under 1px is too fine for two, so it
 *  draws solid. Any other border has no trim. */
export function borderVariables(border: string, width: number): Record<string, string> {
  const double = border === "double" && width >= 1;
  const line = Math.min(width, 2);
  return {
    "--border-style": border === "double" && !double ? "solid" : border,
    "--border-width": `${double ? line * 3 : width}px`,
    "--border-trim": `${double ? line * 2 : 0}px`,
  };
}

/** The motion a skin gives the interface, in milliseconds (docs/superpowers/plans/2026-11-25-richer-skins.md, section 6). One
 *  set for both OSes (`skins/testdata/expected/motion.json`). `none`, and any skin when the system asks for less motion,
 *  gives zero everywhere. Only interface transitions are here: the recording indicator, the capture control, the consent
 *  banner and approvals never read this. */
export interface MotionTokens {
  /** A Home section fades in and rises `rise` px, once, on first show. */
  section: number;
  rise: number;
  /** A button, link or the ask bar changes state. */
  control: number;
  /** The "now" marker settles into its new place. */
  marker: number;
  /** A card or panel opens. */
  panel: number;
}

export const MOTION_NONE: MotionTokens = { section: 0, rise: 0, control: 0, marker: 0, panel: 0 };
export const MOTION_SUBTLE: MotionTokens = { section: 150, rise: 4, control: 120, marker: 200, panel: 150 };

export function motionTokens(motion: string, reduced: boolean): MotionTokens {
  return motion === "subtle" && !reduced ? MOTION_SUBTLE : MOTION_NONE;
}

/** The motion custom properties. Thread sets none, so every rule that reads one falls back to what Thread has always drawn.
 *  A rule that animates names its keyframes through `--anim-*`, which is `none` unless the skin is subtle. */
export function motionVariables(skin: SkinManifest, reduced: boolean): Record<string, string> {
  if (skin.id === THREAD_ID) return {};
  const m = motionTokens(skin.motion, reduced);
  const on = (ms: number, name: string) => (ms > 0 ? name : "none");
  return {
    "--dur-section": `${m.section}ms`,
    "--rise-section": `${m.rise}px`,
    "--dur-control": `${m.control}ms`,
    "--dur-marker": `${m.marker}ms`,
    "--dur-panel": `${m.panel}ms`,
    "--anim-section": on(m.section, "w-rise"),
    "--anim-marker": on(m.marker, "w-settle"),
    "--anim-panel": on(m.panel, "w-fade"),
  };
}

export const MOTION_VARIABLES = ["--dur-section", "--rise-section", "--dur-control", "--dur-marker", "--dur-panel", "--anim-section", "--anim-marker", "--anim-panel"];

const reducedMotion = (): boolean => typeof window !== "undefined" && !!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

/** Every custom property a skin sets other than colours. */
export function skinVariables(skin: SkinManifest, reduced: boolean = reducedMotion()): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const role of ["display", "body", "numerals", "label"] as const) {
    const face = parseFace(skin.typography[role]);
    vars[`--font-${role}`] = face.stack;
    vars[`--weight-${role}`] = String(face.weight);
    vars[`--style-${role}`] = face.italic ? "italic" : "normal";
  }
  vars["--weight-emphasis"] = String(emphasisWeight(parseFace(skin.typography.body).weight));
  // Upper case is `text-transform`; small caps is `font-variant-caps`. Each is its own property.
  vars["--caps"] = skin.typography.caps === "upper" ? "uppercase" : "none";
  vars["--caps-variant"] = skin.typography.caps === "small" ? "small-caps" : "normal";
  vars["--radius"] = `${skin.shape.radius}px`;
  vars["--radius-control"] = `${controlRadius(skin.shape.radius)}px`;
  vars["--radius-surface"] = `${surfaceRadius(skin)}px`;
  Object.assign(vars, borderVariables(skin.shape.border, skin.shape.borderWidth));
  vars["--density"] = String(DENSITY[skin.shape.density] ?? 1);
  vars["--motion"] = skin.motion;
  Object.assign(vars, motionVariables(skin, reduced));
  return vars;
}

// --- texture -----------------------------------------------------------------------------------

export type Texture = "none" | "paper" | "grain" | "grid";

/** The most a texture may move the page color, in relative luminance (0 black, 1 white): four percent. */
export const TEXTURE_MAX_SHIFT = 0.04;
/** What the strength is solved for: a little under the cap so rounding cannot cross it. */
const TEXTURE_TARGET_SHIFT = 0.039;
/** The ink's strongest alpha before the cap. A light page has the least room, so the cap lowers it there. */
export const TEXTURE_NOMINAL_ALPHA: Record<Texture, number> = { none: 0, paper: 0.05, grain: 0.06, grid: 0.05 };
/** Grid line spacing, in px. */
export const TEXTURE_GRID_SPACING = 24;
/** The skin whose look is plain by design: no texture. Thread has none either, by its own file. */
const NO_TEXTURE_SKINS = new Set(["high-contrast", "thread"]);

const hexChannels = (hex: string): [number, number, number] | null => {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
};
const linear = (c: number): number => (c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4));
const luminanceOf = (r: number, g: number, b: number): number => 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);

/** WCAG relative luminance of `#rrggbb`; 1 when it cannot be read (a page that is not understood gets the least ink). */
export function relativeLuminance(hex: string): number {
  const c = hexChannels(hex);
  return c ? luminanceOf(c[0], c[1], c[2]) : 1;
}

/** White ink on a dark page, black on a light one. */
export const textureInk = (pageHex: string): "white" | "black" => (relativeLuminance(pageHex) < 0.5 ? "white" : "black");

/** How far ink of `alpha` moves the page's luminance, blended in encoded sRGB as the screen does. */
export function textureAlphaShift(alpha: number, pageHex: string): number {
  const c = hexChannels(pageHex);
  if (!c) return 0;
  const ink = textureInk(pageHex) === "white" ? 1 : 0;
  const mixed = c.map((v) => v * (1 - alpha) + ink * alpha);
  return Math.abs(luminanceOf(mixed[0], mixed[1], mixed[2]) - luminanceOf(c[0], c[1], c[2]));
}

/** The strongest alpha `texture` may draw on `pageHex`: its nominal alpha, lowered where the page has too little room for it
 *  to stay inside `TEXTURE_MAX_SHIFT`. */
export function texturePeakAlpha(texture: Texture, pageHex: string): number {
  const nominal = TEXTURE_NOMINAL_ALPHA[texture] ?? 0;
  if (nominal === 0 || textureAlphaShift(nominal, pageHex) <= TEXTURE_TARGET_SHIFT) return nominal;
  let low = 0;
  let high = nominal;
  for (let i = 0; i < 40; i++) {
    const mid = (low + high) / 2;
    if (textureAlphaShift(mid, pageHex) <= TEXTURE_TARGET_SHIFT) low = mid;
    else high = mid;
  }
  return low;
}

/** The most the texture can move the page's luminance, at its strongest point. */
export const textureShift = (texture: Texture, pageHex: string): number => textureAlphaShift(texturePeakAlpha(texture, pageHex), pageHex);

/** The CSS `background` layers for a texture on `pageHex`, or "none". Drawn by code: noise is an SVG filter, the grid is two
 *  gradients, so a skin ships no image. The page color is the layer below, set by the rules that use `--sx-texture`. */
export function textureBackground(texture: Texture, pageHex: string): string {
  const alpha = texturePeakAlpha(texture, pageHex);
  if (texture === "none" || alpha <= 0) return "none";
  const rgb = textureInk(pageHex) === "white" ? "255,255,255" : "0,0,0";
  if (texture === "grid") {
    const line = `rgba(${rgb},${alpha.toFixed(4)})`;
    const n = TEXTURE_GRID_SPACING;
    return (
      `linear-gradient(to right, ${line} 1px, transparent 1px) 0 0 / ${n}px ${n}px, ` +
      `linear-gradient(to bottom, ${line} 1px, transparent 1px) 0 0 / ${n}px ${n}px`
    );
  }
  const white = textureInk(pageHex) === "white" ? 1 : 0;
  const frequency = texture === "grain" ? 0.45 : 0.9;
  // Noise in the red channel (0 to 1) becomes the ink's alpha, scaled to the page's peak.
  const svg =
    `<svg xmlns='http://www.w3.org/2000/svg' width='160' height='160'>` +
    `<filter id='n' x='0' y='0' width='100%' height='100%' color-interpolation-filters='sRGB'>` +
    `<feTurbulence type='fractalNoise' baseFrequency='${frequency}' numOctaves='2' stitchTiles='stitch'/>` +
    `<feColorMatrix values='0 0 0 0 ${white} 0 0 0 0 ${white} 0 0 0 0 ${white} ${alpha.toFixed(4)} 0 0 0 0'/>` +
    `</filter><rect width='100%' height='100%' filter='url(#n)'/></svg>`;
  return `url("data:image/svg+xml,${encodeURIComponent(svg)}") 0 0 / 160px 160px`;
}

/** The texture `skin` draws: its own, unless the skin is plain by design (Thread, High Contrast). */
export function skinTexture(skin: SkinManifest): Texture {
  if (NO_TEXTURE_SKINS.has(skin.id)) return "none";
  const t = skin.decoration.texture;
  return t === "paper" || t === "grain" || t === "grid" ? t : "none";
}

/** The data attributes the layouts and decoration style by. */
export function skinAttributes(skin: SkinManifest): Record<string, string> {
  return {
    "data-skin": skin.id,
    "data-archetype": skin.today.archetype,
    "data-header-band": skin.decoration.headerBand,
    "data-dividers": skin.decoration.dividers,
    "data-texture": skin.decoration.texture,
    "data-motion": skin.motion,
  };
}

/** The variant a palette draws under `appearance`. One that ships a single variant serves both, so the
 *  appearance it forces is the one it has. */
export function paletteMode(palette: SkinPalette, appearance: Appearance, systemDark: boolean): Mode {
  const wanted = resolveMode(appearance, systemDark);
  if (palette[wanted]) return wanted;
  return palette.light ? "light" : "dark";
}

/** Custom property to colour for one variant of a palette. */
export function paletteColors(palette: SkinPalette, mode: Mode): Record<string, string> {
  const variant = palette[mode] ?? palette.light ?? palette.dark;
  return variant ? Object.fromEntries(TOKENS.map((token) => [cssVariable(token), variant[token]])) : {};
}

// --- Whimbrel, always present ------------------------------------------------------------------

const WHIMBREL_ID = "whimbrel";
const bundledWhimbrel = import.meta.glob("../../../../skins/whimbrel.json", { import: "default", eager: true }) as Record<string, SkinManifest>;
const whimbrelManifest = Object.values(bundledWhimbrel)[0];
if (!whimbrelManifest) throw new Error("skins/whimbrel.json is missing from the build");

/** Whimbrel as it draws before the shell has answered (and if it never does): the bundled manifest and
 *  the plain themes as its palettes. core/skins pins `skins/whimbrel.json` to its own default. */
export const WHIMBREL: SkinDto = {
  skin: whimbrelManifest,
  palettes: whimbrelManifest.palettes.flatMap((id) => {
    const theme = THEMES.find((t) => t.id === id);
    return theme ? [{ id: theme.id, name: theme.name, author: theme.author, light: theme.light, dark: theme.dark }] : [];
  }),
  builtIn: true,
};

// --- Thread, always present too ----------------------------------------------------------------

export const THREAD_ID = "thread";
export const THREAD_PALETTE_ID = "thread";
const bundledThread = import.meta.glob("../../../../skins/thread.json", { import: "default", eager: true }) as Record<string, SkinManifest>;
const bundledThreadPalette = import.meta.glob("../../../../skins/palettes/thread.json", { import: "default", eager: true }) as Record<string, SkinPalette>;
const threadManifest = Object.values(bundledThread)[0];
const threadPalette = Object.values(bundledThreadPalette)[0];
if (!threadManifest || !threadPalette) throw new Error("skins/thread.json or its palette is missing from the build");

/** Thread as it draws before the shell has answered (and if its list never has it): the bundled manifest and palette,
 *  so the first paint of a window is already Thread and a short list cannot take it away. The shell's own entry
 *  replaces it once `skins_list` answers. */
export const THREAD: SkinDto = { skin: threadManifest, palettes: [threadPalette], builtIn: true };

// --- migration --------------------------------------------------------------------------------

/** The skin and palette a stored plain-theme id becomes (the Mac's `SkinStore.migrated`). */
export function migrateThemeId(themeId: string): { skinId: string; paletteId: string } {
  switch (themeId) {
    case "default":
    case "slate":
    case "sepia":
      return { skinId: WHIMBREL_ID, paletteId: themeId };
    case "high-contrast":
      return { skinId: "high-contrast", paletteId: "high-contrast" };
    default:
      return { skinId: WHIMBREL_ID, paletteId: "default" };
  }
}

// --- the choice -------------------------------------------------------------------------------

export interface Selection {
  /** As stored: an id that is no longer installed is kept, and shows as Whimbrel until the user picks. */
  skinId: string;
  paletteId: string;
  appearance: Appearance;
}

/** The cream default: Classic (id `whimbrel`) in its Default palette. */
export const DEFAULT_SELECTION: Selection = { skinId: WHIMBREL_ID, paletteId: "default", appearance: "system" };
/** Where the once-per-device move sends a person who is on the cream default. */
export const THREAD_SELECTION: Selection = { skinId: THREAD_ID, paletteId: THREAD_PALETTE_ID, appearance: "system" };

// Per device, so localStorage and never synced.
export const SKIN_KEY = "skinId";
export const PALETTE_KEY = "paletteId";
/** Before skins: the plain theme's id. It stays stored after the move, so a downgrade finds it. */
export const LEGACY_THEME_KEY = "themeId";
/** Set once the move to Thread has been looked at on this device, whether or not it moved anyone, so a person who
 *  later picks Classic is not moved again. Per device, never synced. */
export const THREAD_MOVE_KEY = "skinThreadMoveDone";

type Store = Pick<Storage, "getItem" | "setItem">;

/** Storage can be missing or throw (a private window, blocked site data), so every use is guarded. */
function storeOf(win: ThemeWindow): Store | null {
  try {
    return win.localStorage;
  } catch {
    return null;
  }
}

/** What is stored. With storage missing or blocked nothing can be remembered, so the move to Thread cannot be recorded
 *  either: the page shows Thread (the new default) for the session, and a pick still applies for the session. */
export function readSelection(win: ThemeWindow = window): Selection {
  try {
    const store = storeOf(win);
    if (!store) return THREAD_SELECTION;
    const appearance = parseAppearance(store.getItem(APPEARANCE_KEY));
    const skinId = store.getItem(SKIN_KEY);
    if (skinId) return { skinId, paletteId: store.getItem(PALETTE_KEY) || DEFAULT_SELECTION.paletteId, appearance };
    // First launch with skins: carry the old plain-theme choice over.
    const moved = migrateThemeId(store.getItem(LEGACY_THEME_KEY) || DEFAULT_SELECTION.paletteId);
    saveSelection({ ...moved, appearance }, win);
    return { ...moved, appearance };
  } catch {
    return THREAD_SELECTION;
  }
}

/** Whether `selection` is the cream default: Classic in its Default palette. */
export const isCreamDefault = (selection: Selection): boolean => selection.skinId === WHIMBREL_ID && selection.paletteId === "default";

/** The once-per-device move, as a decision: the cream default becomes Thread (keeping the appearance); anything else,
 *  and anyone the move has already been looked at for, stays as it is. Pure. */
export function movedToThread(selection: Selection, done: boolean): Selection {
  if (done || !isCreamDefault(selection)) return selection;
  return { ...THREAD_SELECTION, appearance: selection.appearance };
}

/** Runs the move on this device if it has not run: reads the stored choice (carrying an old plain theme over first),
 *  stores Thread when the choice is the cream default, then stores the done-marker. It runs before the first paint
 *  resolves the selection, in every window. Several windows starting at once are safe: they all decide from the same
 *  storage, a second one that finds Thread (or the marker) writes nothing different, and the choice is written before
 *  the marker so a window that sees the marker never reads a choice that is still cream. A person who then picks
 *  Classic keeps it, because the marker stays. Without usable storage nothing is written (`readSelection` already
 *  answers Thread for the session). */
export function moveToThreadOnce(win: ThemeWindow = window): void {
  try {
    const store = storeOf(win);
    if (!store || store.getItem(THREAD_MOVE_KEY)) return;
    const stored = readSelection(win);
    const moved = movedToThread(stored, false);
    if (moved !== stored) saveSelection(moved, win);
    store.setItem(THREAD_MOVE_KEY, "1");
  } catch {
    // Not remembered: it is looked at again next time, which moves the same people.
  }
}

export function saveSelection(selection: Selection, win: ThemeWindow = window): void {
  try {
    const store = storeOf(win);
    store?.setItem(SKIN_KEY, selection.skinId);
    store?.setItem(PALETTE_KEY, selection.paletteId);
    store?.setItem(APPEARANCE_KEY, selection.appearance);
  } catch {
    // Not remembered, but the choice still applies for this session.
  }
}

/** The High Contrast palette: the bundled plain theme of that id, which the High Contrast skin also names. */
const highContrastTheme = THEMES.find((t) => t.id === "high-contrast");
export const HIGH_CONTRAST_PALETTE: SkinPalette | null = highContrastTheme
  ? { id: highContrastTheme.id, name: highContrastTheme.name, author: highContrastTheme.author, light: highContrastTheme.light, dark: highContrastTheme.dark }
  : null;

/** The media query for "the system asks for more contrast". Not while Windows contrast themes are on
 *  (`forced-colors: active`): there the browser forces the colors itself and the page is left as it was. */
export const MORE_CONTRAST_QUERY = "(prefers-contrast: more) and (not (forced-colors: active))";

/** Which palette is drawn: `palette` as picked, or the High Contrast palette while the system asks for more contrast,
 *  whatever the pick. Pure. Why a palette and not the tokens' own high contrast values: the page's colors come from
 *  the skin's palette as inline variables on the root, and those win over any stylesheet, so a media query could not
 *  bring text and backgrounds to high contrast; and the tokens give high contrast values to the Thread-only colors
 *  (the control border) and to nothing for the plain root. One palette, applied by the same code as any other, keeps
 *  text, backgrounds and parts together in light and in dark, and `data-palette` then says "high-contrast", so the
 *  Thread control border steps aside for that palette's own border. */
export function paletteToDraw(palette: SkinPalette, contrastMore: boolean): SkinPalette {
  return contrastMore && HIGH_CONTRAST_PALETTE ? HIGH_CONTRAST_PALETTE : palette;
}

/** The skin a selection shows and the palette it shows it in: a skin that is gone is Whimbrel (Thread, which always
 *  ships, is never gone), and a palette the skin does not offer is the skin's first. With `contrastMore` the palette
 *  is the High Contrast one whatever the skin; the skin itself is the one picked. */
export function resolveSelection(skins: readonly SkinDto[], selection: Selection, contrastMore = false): { dto: SkinDto; palette: SkinPalette } {
  const dto =
    skins.find((s) => s.skin.id === selection.skinId) ??
    (selection.skinId === THREAD_ID ? THREAD : undefined) ??
    skins.find((s) => s.skin.id === WHIMBREL_ID) ??
    WHIMBREL;
  const palette = dto.palettes.find((p) => p.id === selection.paletteId) ?? dto.palettes[0] ?? WHIMBREL.palettes[0];
  return { dto, palette: paletteToDraw(palette, contrastMore) };
}

// --- the page ---------------------------------------------------------------------------------

/** What `applySkin` needs from the root element; the real `documentElement` fits, and tests pass a fake. */
export interface SkinRoot {
  style: { setProperty(name: string, value: string): void; removeProperty(name: string): unknown; colorScheme: string };
  setAttribute(name: string, value: string): void;
  removeAttribute(name: string): void;
}

const systemIsDark = (): boolean => typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: dark)").matches;

const moreContrast = (): boolean => typeof window !== "undefined" && window.matchMedia(MORE_CONTRAST_QUERY).matches;

/** Set on the root; `system.css` reads it as `--sx-texture`, which forced colors and more contrast turn off. */
export const TEXTURE_VARIABLE = "--texture-image";

function applyTexture(root: SkinRoot, texture: Texture, pageHex: string | undefined): void {
  if (texture === "none" || !pageHex) root.style.removeProperty(TEXTURE_VARIABLE);
  else root.style.setProperty(TEXTURE_VARIABLE, textureBackground(texture, pageHex));
}

/** Shows `dto` in `paletteId` (the skin's first when it does not offer that one) under `appearance`; with
 *  `contrastMore` the palette is the High Contrast one (`paletteToDraw`). */
export function applySkin(
  dto: SkinDto,
  paletteId: string,
  appearance: Appearance,
  root: SkinRoot,
  systemDark: boolean = systemIsDark(),
  contrastMore: boolean = moreContrast(),
): void {
  const picked = dto.palettes.find((p) => p.id === paletteId) ?? dto.palettes[0];
  const palette = picked && paletteToDraw(picked, contrastMore);
  const vars = skinVariables(dto.skin);
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value);
  // Thread sets no motion variable, so one left by the skin before it must go.
  for (const name of MOTION_VARIABLES) if (!(name in vars)) root.style.removeProperty(name);
  for (const [name, value] of Object.entries(skinAttributes(dto.skin))) root.setAttribute(name, value);
  if (!palette) {
    root.style.removeProperty(TEXTURE_VARIABLE);
    return;
  }
  const textureMode = paletteMode(palette, appearance, systemDark);
  applyTexture(root, contrastMore ? "none" : skinTexture(dto.skin), (palette[textureMode] ?? palette.light ?? palette.dark)?.backgroundPage);
  if (dto.skin.id === WHIMBREL_ID && palette.id === "default" && appearance === "system") {
    for (const token of TOKENS) root.style.removeProperty(cssVariable(token));
    root.removeAttribute("data-theme");
    root.removeAttribute("data-palette");
    root.style.colorScheme = "";
    return;
  }
  const mode = paletteMode(palette, appearance, systemDark);
  for (const [name, value] of Object.entries(paletteColors(palette, mode))) root.style.setProperty(name, value);
  root.setAttribute("data-theme", mode);
  // Which palette is showing, so tokens.generated.css can give Thread's palette its own control border.
  root.setAttribute("data-palette", palette.id);
  // An explicit choice also tells the browser, so scrollbars and form controls match; System
  // leaves `color-scheme: light dark` from tokens.css in charge. A single-variant palette forces its mode.
  root.style.colorScheme = appearance === "system" && palette.light && palette.dark ? "" : mode;
}

/** What is installed right now: Whimbrel and Thread until the shell answers. */
let installed: readonly SkinDto[] = [WHIMBREL, THREAD];
/** What is applied right now, so an OS change repaints even when storage could not be written. */
let current: Selection = DEFAULT_SELECTION;
/** Whether the system asks for more contrast right now (`MORE_CONTRAST_QUERY`), as of the last repaint. */
let contrastMore = false;

export const currentSelection = (): Selection => current;
export const currentContrastMore = (): boolean => contrastMore;
export const installedSkins = (): readonly SkinDto[] => installed;

/** Replaces what is installed. Whimbrel is always first and always present. */
export function setInstalledSkins(list: readonly SkinDto[]): void {
  const whimbrel = list.find((s) => s.skin.id === WHIMBREL_ID) ?? WHIMBREL;
  installed = [whimbrel, ...list.filter((s) => s.skin.id !== WHIMBREL_ID)];
}

/** Bumped on every repaint, so React can re-read what is shown (useSkin.ts). */
let revision = 0;
const listeners = new Set<() => void>();
export const skinRevision = (): number => revision;
export function subscribeSkin(listener: () => void): () => void {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** How often every window asks whether a sync changed the skins (the shell emits no events; Task 15). */
export const REVISION_POLL_MS = 5000;

/** Installs a reloaded `list`. When the selected skin is no longer in it (deleted here or on another device),
 *  the selection falls back to Whimbrel, keeping the appearance, as the Mac's `reloadUserSkins` does; other
 *  windows follow through storage. */
function install(list: readonly SkinDto[], win: ThemeWindow): void {
  setInstalledSkins(list);
  // Thread is bundled with the page, so a list that lacks it (a short one) must not reset a person off it for good.
  if (current.skinId !== THREAD_ID && !installed.some((s) => s.skin.id === current.skinId)) {
    current = { ...DEFAULT_SELECTION, appearance: current.appearance };
    saveSelection(current, win);
  }
}

function paint(win: ThemeWindow): void {
  contrastMore = win.matchMedia(MORE_CONTRAST_QUERY).matches;
  const { dto, palette } = resolveSelection(installed, current, contrastMore);
  applySkin(dto, palette.id, current.appearance, win.document.documentElement, win.matchMedia("(prefers-color-scheme: dark)").matches, contrastMore);
  revision += 1;
  listeners.forEach((listener) => listener());
}

/** Applies the stored choice and keeps the page in step with the OS and with other windows (the popup
 *  and the alert share this bundle and this storage). Call before the first render. The first paint is
 *  Whimbrel's, which needs nothing from the shell; the rest of the skins follow when `load` answers. */
export function startSkin(
  win: ThemeWindow = window,
  load: typeof skinsList = skinsList,
  revision: typeof skinsRevision | null = skinsRevision,
): () => void {
  moveToThreadOnce(win);
  current = readSelection(win);
  paint(win);
  let live = true;
  load()
    .then((list) => {
      if (!live) return;
      // At launch a missing skin only draws as Whimbrel; the stored choice stays, in case the list was short.
      setInstalledSkins(list);
      paint(win);
    })
    .catch((error) => console.warn("could not load skins; showing Whimbrel", error));
  // A skin made, changed or deleted on another device: reload when the shell's revision moves. It starts at 0
  // with the shell, so a change synced before the first look still reloads.
  let seen = 0;
  const poll = revision
    ? setInterval(() => {
        revision().then(
          (now) => {
            if (live && now !== seen) void reloadSkins(win, load).catch(() => {});
            seen = now;
          },
          () => {},
        );
      }, REVISION_POLL_MS)
    : null;
  const system = win.matchMedia("(prefers-color-scheme: dark)");
  const contrast = win.matchMedia(MORE_CONTRAST_QUERY);
  const onSystem = () => paint(win);
  const onStorage = (event: Event) => {
    const key = (event as StorageEvent).key;
    if (key !== null && key !== SKIN_KEY && key !== PALETTE_KEY && key !== APPEARANCE_KEY) return;
    current = readSelection(win);
    paint(win);
  };
  system.addEventListener("change", onSystem);
  contrast.addEventListener("change", onSystem);
  win.addEventListener("storage", onStorage);
  return () => {
    live = false;
    if (poll !== null) clearInterval(poll);
    system.removeEventListener("change", onSystem);
    contrast.removeEventListener("change", onSystem);
    win.removeEventListener("storage", onStorage);
  };
}

/** Fetches the skins again (after one is made, imported or deleted) and repaints. */
export async function reloadSkins(win: ThemeWindow = window, load: typeof skinsList = skinsList): Promise<void> {
  install(await load(), win);
  paint(win);
}

/** The user picked something: remember it and show it. Other windows hear about it through storage. */
export function chooseSkin(next: Selection, win: ThemeWindow = window): void {
  current = next;
  saveSelection(next, win);
  paint(win);
}
