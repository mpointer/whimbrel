import { resolveSelection, type Selection, type SkinDto, type SkinPalette } from "../skin";
import type { Mode } from "../theme";
import { voiceText } from "../today/layoutModel";

/** The five colours a palette row previews, in the order the Mac draws them. */
export const SWATCH_TOKENS = ["backgroundPage", "backgroundCard", "accentPrimary", "accentSecondary", "textPrimary"] as const;

/** The footnote under the gallery (the Mac's, for this device). */
export const GALLERY_FOOTNOTE = "Each PC keeps its own skin.";

export interface PaletteChoice {
  id: string;
  name: string;
  current: boolean;
}

export interface Card {
  id: string;
  name: string;
  tagline: string;
  /** A skin made by the user (ids start `user-`), shown after the built-ins and marked. */
  yours: boolean;
  selected: boolean;
  /** The palette this card previews: the selected skin's current one, else the skin's first. */
  previewPalette: SkinPalette;
  /** Offered only on the selected card. */
  palettes: PaletteChoice[];
  /** The selected skin's palette ships one variant only, so System/Light/Dark cannot change it. */
  darkOnlyLocked: boolean;
}

export const isUserSkin = (id: string): boolean => id.startsWith("user-");

/** Built-ins in installed order, then the user's by name. */
export function orderedSkins(skins: readonly SkinDto[]): SkinDto[] {
  const byName = (a: SkinDto, b: SkinDto) => a.skin.name.localeCompare(b.skin.name, undefined, { numeric: true, sensitivity: "base" });
  return [...skins.filter((s) => !isUserSkin(s.skin.id)), ...skins.filter((s) => isUserSkin(s.skin.id)).sort(byName)];
}

/** Shown under the picker when the palette in force overrides it; null when the picker is live. */
export function lockedCaption(palette: SkinPalette): string | null {
  if (palette.light && palette.dark) return null;
  return palette.dark ? "This palette is dark only, so it always uses Dark." : "This palette is light only, so it always uses Light.";
}

export function galleryState(skins: readonly SkinDto[], selection: Selection): Card[] {
  const { dto: selectedDto, palette: selectedPalette } = resolveSelection(skins, selection);
  return orderedSkins(skins).map((dto) => {
    const selected = dto.skin.id === selectedDto.skin.id;
    const previewPalette = selected ? selectedPalette : dto.palettes[0];
    return {
      id: dto.skin.id,
      name: dto.skin.name,
      tagline: voiceText(dto.skin.voice, "tagline"),
      yours: isUserSkin(dto.skin.id),
      selected,
      previewPalette,
      palettes: selected ? dto.palettes.map((p) => ({ id: p.id, name: p.name, current: p.id === selectedPalette.id })) : [],
      darkOnlyLocked: selected && lockedCaption(selectedPalette) !== null,
    };
  });
}

/** Clicking a card picks the skin's first palette, except on the skin already selected (it keeps its current one). */
export function pickCard(skins: readonly SkinDto[], selection: Selection, skinId: string): Selection {
  const { dto, palette } = resolveSelection(skins, selection);
  if (dto.skin.id === skinId) return { ...selection, skinId, paletteId: palette.id };
  const target = skins.find((s) => s.skin.id === skinId);
  return target ? { ...selection, skinId, paletteId: target.palettes[0].id } : selection;
}

export const pickPalette = (selection: Selection, skinId: string, paletteId: string): Selection => ({ ...selection, skinId, paletteId });

/** The colours of one palette row in `mode`; a single-variant palette shows the variant it has. */
export function swatchColors(palette: SkinPalette, mode: Mode): string[] {
  const variant = palette[mode] ?? palette.light ?? palette.dark;
  return variant ? SWATCH_TOKENS.map((t) => variant[t]) : [];
}
