// Applying a skin: which one is chosen, where that is remembered, and how it reaches the page
// (docs/superpowers/specs/2026-10-22-skins-windows-and-sync-design.md). The rules and the files are
// in core/skins; this draws what `skins_list` returns, from CSS variables and data attributes on the
// root element. Colours come from the skin's palette; everything else from the skin.
//
// Whimbrel with its Default palette under System appearance is left to tokens.css alone, exactly as
// before skins existed. Every other choice sets the variables, and follows the OS live when the
// appearance is System.

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

/** The control radius is two thirds of the radius, rounded (core/skins `Shape::control_radius`). */
export const controlRadius = (radius: number): number => Math.round((radius * 2) / 3);

/** Every custom property a skin sets other than colours. */
export function skinVariables(skin: SkinManifest): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const role of ["display", "body", "numerals", "label"] as const) {
    const face = parseFace(skin.typography[role]);
    vars[`--font-${role}`] = face.stack;
    vars[`--weight-${role}`] = String(face.weight);
    vars[`--style-${role}`] = face.italic ? "italic" : "normal";
  }
  // Upper case is `text-transform`; small caps is `font-variant-caps`. Each is its own property.
  vars["--caps"] = skin.typography.caps === "upper" ? "uppercase" : "none";
  vars["--caps-variant"] = skin.typography.caps === "small" ? "small-caps" : "normal";
  vars["--radius"] = `${skin.shape.radius}px`;
  vars["--radius-control"] = `${controlRadius(skin.shape.radius)}px`;
  vars["--border-style"] = skin.shape.border;
  vars["--border-width"] = `${skin.shape.borderWidth}px`;
  vars["--density"] = String(DENSITY[skin.shape.density] ?? 1);
  vars["--motion"] = skin.motion;
  return vars;
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

export const DEFAULT_SELECTION: Selection = { skinId: WHIMBREL_ID, paletteId: "default", appearance: "system" };

// Per device, so localStorage and never synced.
export const SKIN_KEY = "skinId";
export const PALETTE_KEY = "paletteId";
/** Before skins: the plain theme's id. It stays stored after the move, so a downgrade finds it. */
export const LEGACY_THEME_KEY = "themeId";

type Store = Pick<Storage, "getItem" | "setItem">;

/** Storage can be missing or throw (a private window, blocked site data), so every use is guarded. */
function storeOf(win: ThemeWindow): Store | null {
  try {
    return win.localStorage;
  } catch {
    return null;
  }
}

export function readSelection(win: ThemeWindow = window): Selection {
  try {
    const store = storeOf(win);
    if (!store) return DEFAULT_SELECTION;
    const appearance = parseAppearance(store.getItem(APPEARANCE_KEY));
    const skinId = store.getItem(SKIN_KEY);
    if (skinId) return { skinId, paletteId: store.getItem(PALETTE_KEY) || DEFAULT_SELECTION.paletteId, appearance };
    // First launch with skins: carry the old plain-theme choice over.
    const moved = migrateThemeId(store.getItem(LEGACY_THEME_KEY) || DEFAULT_SELECTION.paletteId);
    saveSelection({ ...moved, appearance }, win);
    return { ...moved, appearance };
  } catch {
    return DEFAULT_SELECTION;
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

/** The skin a selection shows and the palette it shows it in: a skin that is gone is Whimbrel, and a
 *  palette the skin does not offer is the skin's first. */
export function resolveSelection(skins: readonly SkinDto[], selection: Selection): { dto: SkinDto; palette: SkinPalette } {
  const dto = skins.find((s) => s.skin.id === selection.skinId) ?? skins.find((s) => s.skin.id === WHIMBREL_ID) ?? WHIMBREL;
  const palette = dto.palettes.find((p) => p.id === selection.paletteId) ?? dto.palettes[0] ?? WHIMBREL.palettes[0];
  return { dto, palette };
}

// --- the page ---------------------------------------------------------------------------------

/** What `applySkin` needs from the root element; the real `documentElement` fits, and tests pass a fake. */
export interface SkinRoot {
  style: { setProperty(name: string, value: string): void; removeProperty(name: string): unknown; colorScheme: string };
  setAttribute(name: string, value: string): void;
  removeAttribute(name: string): void;
}

const systemIsDark = (): boolean => typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: dark)").matches;

/** Shows `dto` in `paletteId` (the skin's first when it does not offer that one) under `appearance`. */
export function applySkin(dto: SkinDto, paletteId: string, appearance: Appearance, root: SkinRoot, systemDark: boolean = systemIsDark()): void {
  const palette = dto.palettes.find((p) => p.id === paletteId) ?? dto.palettes[0];
  for (const [name, value] of Object.entries(skinVariables(dto.skin))) root.style.setProperty(name, value);
  for (const [name, value] of Object.entries(skinAttributes(dto.skin))) root.setAttribute(name, value);
  if (!palette) return;
  if (dto.skin.id === WHIMBREL_ID && palette.id === "default" && appearance === "system") {
    for (const token of TOKENS) root.style.removeProperty(cssVariable(token));
    root.removeAttribute("data-theme");
    root.style.colorScheme = "";
    return;
  }
  const mode = paletteMode(palette, appearance, systemDark);
  for (const [name, value] of Object.entries(paletteColors(palette, mode))) root.style.setProperty(name, value);
  root.setAttribute("data-theme", mode);
  // An explicit choice also tells the browser, so scrollbars and form controls match; System
  // leaves `color-scheme: light dark` from tokens.css in charge. A single-variant palette forces its mode.
  root.style.colorScheme = appearance === "system" && palette.light && palette.dark ? "" : mode;
}

/** What is installed right now: Whimbrel until the shell answers. */
let installed: readonly SkinDto[] = [WHIMBREL];
/** What is applied right now, so an OS change repaints even when storage could not be written. */
let current: Selection = DEFAULT_SELECTION;

export const currentSelection = (): Selection => current;
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
  if (!installed.some((s) => s.skin.id === current.skinId)) {
    current = { ...DEFAULT_SELECTION, appearance: current.appearance };
    saveSelection(current, win);
  }
}

function paint(win: ThemeWindow): void {
  const { dto, palette } = resolveSelection(installed, current);
  applySkin(dto, palette.id, current.appearance, win.document.documentElement, win.matchMedia("(prefers-color-scheme: dark)").matches);
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
  const onSystem = () => paint(win);
  const onStorage = (event: Event) => {
    const key = (event as StorageEvent).key;
    if (key !== null && key !== SKIN_KEY && key !== PALETTE_KEY && key !== APPEARANCE_KEY) return;
    current = readSelection(win);
    paint(win);
  };
  system.addEventListener("change", onSystem);
  win.addEventListener("storage", onStorage);
  return () => {
    live = false;
    if (poll !== null) clearInterval(poll);
    system.removeEventListener("change", onSystem);
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
