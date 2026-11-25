import { t } from "../../l10n";
import { sectionTitle, SETTINGS_SECTIONS, type SettingsSectionId } from "../settingsSections";
import { memorySectionTitle } from "./names";

/** The redesigned Settings keeps the old section ids and order (so `openSettings("capture")` and the open-settings
 *  event keep working) and changes only the words: "Capture & Privacy" is "Memory & Privacy". */
export const REDESIGN_SECTIONS = SETTINGS_SECTIONS;

export function redesignSectionTitle(id: SettingsSectionId): string {
  if (id === "capture") return memorySectionTitle();
  return sectionTitle(id);
}

/** The picker above the content in a narrow window or at large text. */
export const sectionPickerLabel = () => t("Settings section");
