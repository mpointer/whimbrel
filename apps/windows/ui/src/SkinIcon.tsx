import { useSkin } from "./useSkin";
import { ICON_FONT, iconGlyph, type IconRole } from "./skinIcons";
import { SkinImage } from "./today/archetypes/parts";
import type { TabId } from "./tabs";

/** The role each sidebar tab draws (the Mac's shell: Library is the Ask screen). */
export const TAB_ICON: Record<TabId, IconRole> = {
  today: "today",
  meetings: "meetings",
  library: "ask",
  automations: "routines",
  settings: "settings",
};

/** A role's icon in the skin: one glyph from Segoe Fluent Icons (Segoe MDL2 Assets as the fallback). */
export function SkinIcon({ role }: { role: IconRole }) {
  const { skin } = useSkin();
  return (
    <span className="skin-icon" style={{ fontFamily: ICON_FONT }} aria-hidden="true">
      {iconGlyph(skin, role)}
    </span>
  );
}

/** The skin's header image and header band, above every top-level screen and outside its scrolling. A skin
 *  with neither (Whimbrel, High Contrast) adds nothing. */
export function SkinHeader() {
  const { skin, imagery } = useSkin();
  return (
    <div className="skin-header">
      <SkinImage imagery={imagery} skinId={skin.id} placement="header" />
      <div className="skin-band" aria-hidden="true" />
    </div>
  );
}
