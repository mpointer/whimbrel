// The words of the Keeping data card: the confirm before a shorter period, the confirm before "Delete old data now",
// and the last-run line. Built here so none of them says "Chats" (the old retention strings do).

import type { RetentionPreview, RetentionRun, RetentionSettings } from "../../../../../generated/types";
import { t } from "../../../../../l10n";
import { megabytes } from "../../../../retentionModel";
import { retentionRowLabels } from "../../../names";
import type { RetentionKey } from "./retentionKeys";

/** The row labels with a lower-case form for the middle of a sentence. */
function lower(key: RetentionKey): string {
  if (key === "captureDays") return t("screen text");
  if (key === "meetingDays") return t("meetings and recordings");
  return t("conversations");
}

export interface RetentionConfirmCopy {
  title: string;
  body: string;
  confirmLabel: string;
}

/** What committing `days` for one window will delete, in plain words. Used for a shorter period, or a period after forever. */
export function periodConfirmCopy(key: RetentionKey, days: number, from: number | null, acknowledged: boolean): RetentionConfirmCopy {
  const label = retentionRowLabels()[key];
  const parts = [
    t("{label} older than {days} days will be deleted once a day, along with the recordings that belong to it. This can't be undone.", { label, days }),
    from === null ? t("It is kept forever now.") : t("It is kept for {days} days now.", { days: from }),
    t("A meeting being recorded is never touched."),
  ];
  if (!acknowledged) parts.push(t("The daily cleanup is waiting for the notice on the main screen, so nothing is deleted until you answer it."));
  return {
    title: t("Delete {what} older than {days} days?", { what: lower(key), days }),
    body: parts.join(" "),
    confirmLabel: t("Delete older data"),
  };
}

/** One line per window that has a period, with the count when the app could work it out. */
export function deleteNowLines(settings: RetentionSettings, preview: RetentionPreview | null): string[] {
  const lines: string[] = [];
  const windows: [number | null | undefined, (days: number) => string, string | null][] = [
    [
      settings.captureDays,
      (days) => t("Screen text older than {count} days", { count: days }),
      preview ? t("{count} items", { count: preview.captures }) : null,
    ],
    [
      settings.meetingDays,
      (days) => t("Meetings and recordings older than {count} days", { count: days }),
      preview ? t("{count} meetings", { count: preview.meetings }) : null,
    ],
    [
      settings.conversationDays,
      (days) => t("Conversations older than {count} days", { count: days }),
      preview ? t("{count} conversations", { count: preview.conversations }) : null,
    ],
  ];
  for (const [days, label, count] of windows) {
    if (days == null) continue;
    lines.push(count ? t("{label}: {count}", { label: label(days), count }) : label(days));
  }
  if (preview && preview.orphans > 0) {
    lines.push(
      t("{label}: {count}", {
        label: t("Recordings no meeting uses any more"),
        count: t("{count} files ({size})", { count: preview.orphans, size: megabytes(preview.orphanBytes) }),
      }),
    );
  }
  return lines;
}

/** What the command really does (commands/retention.rs `retention_run_now`): sweeps every window that has a period, removes
 *  recordings no meeting uses any more whatever the periods are, and ends the first-run hold so the daily clean-up starts. */
export function deleteNowCopy(settings: RetentionSettings, preview: RetentionPreview | null): { title: string; body: string; lines: string[]; confirmLabel: string } {
  const lines = deleteNowLines(settings, preview);
  const allForever = settings.captureDays == null && settings.meetingDays == null && settings.conversationDays == null;
  const first = allForever
    ? t("Every period is set to Keep forever, so no screen text, meetings or conversations are deleted for their age.")
    : t("This deletes everything older than your periods, and the recordings that belong to it, right now. It can't be undone. A meeting being recorded is never touched.");
  return {
    title: t("Delete old data now?"),
    body: [
      first,
      t("It also removes recordings that no meeting uses any more, whatever the periods say."),
      t("It also starts the daily clean-up if that was waiting for the notice on the main screen."),
    ].join(" "),
    lines,
    confirmLabel: t("Delete old data"),
  };
}

/** "Last run 3:04 PM: 12 screen captures, 1 meetings, 0 conversations, 0 leftover recordings deleted." */
export function lastRunLine(run: RetentionRun, time: string): string {
  return t("Last run {time}: {captures} screen captures, {meetings} meetings, {conversations} conversations, {orphans} leftover recordings deleted.", {
    time,
    captures: run.capturesDeleted,
    meetings: run.meetingsDeleted,
    conversations: run.conversationsDeleted,
    orphans: run.orphansDeleted,
  });
}
