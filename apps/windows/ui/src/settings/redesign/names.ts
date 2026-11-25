// The words the redesigned Settings uses for things that had two names (docs/ui-redesign/screens/settings.md, "Naming").
// Redesigned copy says "Memory" for saving the text of what is on screen, "your library" for the local database and
// "Keep forever" for no time limit. Stored setting names do not change.
import { t } from "../../l10n";

/** The section that was "Capture & Privacy". Settings search still finds it under "capture". */
export const memorySectionTitle = () => t("Memory & Privacy");
/** The retention rows (RetentionSettings.captureDays, meetingDays, conversationDays). */
export const retentionRowLabels = () => ({ captureDays: t("Screen text"), meetingDays: t("Meetings and recordings"), conversationDays: t("Conversations") });
