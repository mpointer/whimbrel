import { t } from "../l10n";

/**
 * Settings sections, one shown at a time, in the Mac's order and wording (SettingsView.swift),
 * after the status page that opens first (issue 595; on the Mac it is the main window's Settings
 * entry). The Mac's Automations and Diagnostics sections have no Windows cards: routines live
 * in the Automations tab, and there is no diagnostics pane.
 */
export const SETTINGS_SECTIONS = ["status", "general", "profile", "capture", "meetings", "ai", "topics", "devices", "connectors"] as const;
export type SettingsSectionId = (typeof SETTINGS_SECTIONS)[number];

/** Where the Settings tab opens: the status page, as on the Mac. */
export const DEFAULT_SECTION: SettingsSectionId = "status";

export function sectionTitle(id: SettingsSectionId): string {
  switch (id) {
    case "status":
      return t("Status");
    case "general":
      return t("General");
    case "profile":
      return t("Profile");
    case "capture":
      return t("Capture & Privacy");
    case "meetings":
      return t("Meetings");
    case "ai":
      return t("AI");
    case "topics":
      return t("Topics");
    case "devices":
      return t("Sync & Devices");
    case "connectors":
      return t("Connectors");
  }
}

/** A requested section, or the status page when it isn't one (an old value, a typo). */
export function toSection(value: string | null | undefined): SettingsSectionId {
  return (SETTINGS_SECTIONS as readonly string[]).includes(value ?? "") ? (value as SettingsSectionId) : DEFAULT_SECTION;
}
