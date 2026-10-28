import type { RetentionPreview, RetentionSettings } from "../generated/types";

/** One row of the first-run notice: what a window covers, and how much is past it. */
export type NoticeLine = { label: string; count: string };

function plural(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

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
  const windows: [number | null | undefined, string, string][] = [
    [settings.captureDays, "Screen text", plural(preview.captures, "item")],
    [settings.meetingDays, "Meetings and recordings", plural(preview.meetings, "meeting")],
    [settings.conversationDays, "Chats", plural(preview.conversations, "chat")],
  ];
  for (const [days, label, count] of windows) {
    if (days == null) continue;
    lines.push({ label: `${label} older than ${plural(days, "day")}`, count });
  }
  if (preview.orphans > 0) {
    lines.push({
      label: "Recordings no meeting uses any more",
      count: `${plural(preview.orphans, "file")} (${megabytes(preview.orphanBytes)})`,
    });
  }
  return lines;
}

/** True when the first sweep would delete nothing — the notice still asks, but softly. */
export function nothingToDelete(preview: RetentionPreview): boolean {
  return preview.captures + preview.meetings + preview.conversations + preview.orphans === 0;
}
