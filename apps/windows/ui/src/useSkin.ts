// The skin being shown, for components: re-read whenever skin.ts repaints (a pick, a reload, the OS).

import { useSyncExternalStore } from "react";
import { archetypeOf, imageryOf, type ArchetypeId, type Imagery } from "./today/layoutModel";
import { installedSkins, currentSelection, resolveSelection, skinRevision, subscribeSkin, type SkinDto, type SkinManifest, type SkinPalette } from "./skin";

export interface ShownSkin {
  dto: SkinDto;
  skin: SkinManifest;
  palette: SkinPalette;
  archetype: ArchetypeId;
  imagery: Imagery[];
}

export function useSkin(): ShownSkin {
  useSyncExternalStore(subscribeSkin, skinRevision);
  const { dto, palette } = resolveSelection(installedSkins(), currentSelection());
  return { dto, skin: dto.skin, palette, archetype: archetypeOf(dto.skin.today.archetype), imagery: imageryOf(dto.skin, dto.imageScrims) };
}
