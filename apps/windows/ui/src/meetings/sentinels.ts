import { t } from "../l10n";

// The meeting store keeps a few English placeholders that core matches on: the title a
// meeting has until a summary or the calendar names it, and the speaker labels a transcript
// line carries. They are shown translated, but only here, at display time; the stored value
// stays English, so nothing that saves, searches or compares may use these results.

/** The placeholder title core gives a meeting until it has a real one. */
export const PLACEHOLDER_TITLE = "Meeting";

/** A stored speaker label for display: "Me", "Others" and "Speaker N" translated, a real name as is. */
export function speakerLabel(stored: string): string {
  if (stored === "Me") return t("Me");
  if (stored === "Others") return t("Others");
  const numbered = /^Speaker (\d+)$/.exec(stored);
  if (numbered) return t("Speaker {n}", { n: Number(numbered[1]) });
  return stored;
}

/** A stored meeting title for display: the placeholder translated, a real title as is. */
export function meetingTitleLabel(stored: string): string {
  return stored === PLACEHOLDER_TITLE ? t("Meeting") : stored;
}
