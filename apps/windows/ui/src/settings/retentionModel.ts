import type { RetentionPreview, RetentionSettings } from "../generated/types";
import { t } from "../l10n";

/** One row of the first-run notice: what a window covers, and how much is past it. */
export type NoticeLine = { label: string; count: string };

/** "34.2 MB", the way the rest of the app writes sizes. */
export function megabytes(bytes: number): string {
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

/**
 * What the first sweep would remove under `settings`. Windows set to "keep forever" are
 * left out entirely — nothing to warn about — and so is the leftover-recordings line
 * when there are none, since it isn't a window the user chose.
 */
export function noticeLines(settings: RetentionSettings, preview: RetentionPreview): NoticeLine[] {
  const lines: NoticeLine[] = [];
  // Each label is one whole plural sentence per window, so a language can place the count itself.
  const windows: [number | null | undefined, (days: number) => string, string][] = [
    [
      settings.captureDays,
      (days) => t("Screen text older than {count} days", { count: days }),
      t("{count} items", { count: preview.captures }),
    ],
    [
      settings.meetingDays,
      (days) => t("Meetings and recordings older than {count} days", { count: days }),
      t("{count} meetings", { count: preview.meetings }),
    ],
    [
      settings.conversationDays,
      (days) => t("Chats older than {count} days", { count: days }),
      t("{count} chats", { count: preview.conversations }),
    ],
  ];
  for (const [days, label, count] of windows) {
    if (days == null) continue;
    lines.push({ label: label(days), count });
  }
  if (preview.orphans > 0) {
    lines.push({
      label: t("Recordings no meeting uses any more"),
      count: t("{count} files ({size})", { count: preview.orphans, size: megabytes(preview.orphanBytes) }),
    });
  }
  return lines;
}

/** True when the first sweep would delete nothing — the notice still asks, but softly. */
export function nothingToDelete(preview: RetentionPreview): boolean {
  return preview.captures + preview.meetings + preview.conversations + preview.orphans === 0;
}
