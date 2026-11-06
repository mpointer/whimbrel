import { t } from "../l10n";

/**
 * Settings sections, one shown at a time, in the Mac's order and wording (SettingsView.swift).
 * The Mac's Automations and Diagnostics sections have no Windows cards: routines live in the
 * Automations tab, and there is no diagnostics pane.
 */
export const SETTINGS_SECTIONS = ["general", "profile", "capture", "meetings", "ai", "devices", "connectors"] as const;
export type SettingsSectionId = (typeof SETTINGS_SECTIONS)[number];

export const DEFAULT_SECTION: SettingsSectionId = "general";

export function sectionTitle(id: SettingsSectionId): string {
  switch (id) {
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
    case "devices":
      return t("Sync & Devices");
    case "connectors":
      return t("Connectors");
  }
}

/** A stored or requested section, or the default when it isn't one (an old value, a typo). */
export function toSection(value: string | null | undefined): SettingsSectionId {
  return (SETTINGS_SECTIONS as readonly string[]).includes(value ?? "") ? (value as SettingsSectionId) : DEFAULT_SECTION;
}

const STORAGE_KEY = "whimbrel.settings.section";

/** The section shown last time, so Settings reopens where you left it. */
export function rememberedSection(storage: Pick<Storage, "getItem"> | null = safeStorage()): SettingsSectionId {
  try {
    return toSection(storage?.getItem(STORAGE_KEY));
  } catch {
    return DEFAULT_SECTION;
  }
}

export function rememberSection(id: SettingsSectionId, storage: Pick<Storage, "setItem"> | null = safeStorage()): void {
  try {
    storage?.setItem(STORAGE_KEY, id);
  } catch {
    // Remembering is a convenience; Settings opens at General without it.
  }
}

function safeStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}
