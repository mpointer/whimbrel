import type { LiveAlertView } from "../generated/types";

/** The Mac's 30-minute window for late drafts and peer alerts (#532). */
export const POLL_WINDOW_MS = 30 * 60 * 1000;

/**
 * Whether "Questions for you" reloads every 10 s: while the meeting records with alerts on,
 * for 30 minutes after it ends, or while an alert from the last 30 minutes still has no
 * draft, so a late draft or another device's alert shows without reopening the page. The
 * Mac's `LiveAlertsSection.shouldPoll`.
 */
export function shouldPoll(opts: {
  alertsOn: boolean;
  recording: boolean;
  endedUnixMs: number | null;
  rows: Pick<LiveAlertView, "draft" | "createdAtUnixMs">[];
  nowUnixMs: number;
}): boolean {
  const { alertsOn, recording, endedUnixMs, rows, nowUnixMs } = opts;
  if (!alertsOn) return false;
  return (
    recording ||
    (endedUnixMs != null && nowUnixMs - endedUnixMs < POLL_WINDOW_MS) ||
    rows.some((r) => r.draft == null && nowUnixMs - r.createdAtUnixMs < POLL_WINDOW_MS)
  );
}
