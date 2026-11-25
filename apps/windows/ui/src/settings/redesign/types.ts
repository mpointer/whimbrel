import type { SettingsSectionId } from "../settingsSections";

/** What every redrawn section receives from the frame (the same hand-offs the old SettingsView gives its cards). */
export interface SectionProps {
  /** Opens another Settings section (for example "Set up an AI provider" opens AI). */
  onSection: (id: SettingsSectionId) => void;
  /** The retention notice on the main screen reads the saved period again. */
  onRetentionSaved: () => void;
  /** Routines live in the Automations tab on Windows. */
  onOpenAutomations: () => void;
}
