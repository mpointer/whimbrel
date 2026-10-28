// Icons: a skin names one SF Symbol per role, and the shared table (skins/icons.json) maps each allowed
// symbol to a Segoe Fluent Icons code point, with Segoe MDL2 Assets as the fallback font. A name outside
// the table (a skin made on a newer Mac) draws the role's Whimbrel icon and never fails.

import type { SkinManifest } from "./skin";

export interface IconEntry {
  symbol: string;
  role: string;
  fluent: number;
  mdl2: number;
}

const tables = import.meta.glob("../../../../skins/icons.json", { import: "default", eager: true }) as Record<string, IconEntry[]>;
export const ICON_TABLE: readonly IconEntry[] = Object.values(tables)[0] ?? [];
if (ICON_TABLE.length === 0) throw new Error("skins/icons.json is missing from the build");

/** The nine roles a skin picks an icon for. */
export const ICON_ROLES = ["today", "meetings", "routines", "settings", "ask", "person", "time", "prep", "recap"] as const;
export type IconRole = (typeof ICON_ROLES)[number];

/** The font stack that draws the glyphs. */
export const ICON_FONT = '"Segoe Fluent Icons", "Segoe MDL2 Assets"';

/** Whimbrel's own symbol for each role: what a role draws when its skin's choice is not in the table. */
const WHIMBREL_SYMBOLS: Record<IconRole, string> = {
  today: "sun.max",
  meetings: "person.2",
  routines: "bolt",
  settings: "gearshape",
  ask: "arrow.up.circle.fill",
  person: "person",
  time: "clock",
  prep: "doc.text",
  recap: "text.alignleft",
};

const bySymbol = (symbol: string | undefined): IconEntry | undefined => ICON_TABLE.find((e) => e.symbol === symbol);

/** The table entry a skin's role draws: its own choice, else the role's Whimbrel icon. */
export function iconEntry(skin: Pick<SkinManifest, "icons">, role: IconRole): IconEntry {
  const entry = bySymbol(skin.icons[role]) ?? bySymbol(WHIMBREL_SYMBOLS[role]);
  if (!entry) throw new Error(`icons.json has no ${WHIMBREL_SYMBOLS[role]}`);
  return entry;
}

/** The character to draw in `ICON_FONT`. Fluent and MDL2 share every code point in the table (a test pins it). */
export const iconGlyph = (skin: Pick<SkinManifest, "icons">, role: IconRole): string => String.fromCodePoint(iconEntry(skin, role).fluent);
