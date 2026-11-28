import type { MeetingRecordingView } from "../generated/types";
import { t } from "../l10n";

/** A running recording's length: "0:07", "12:04", "1:02:09". */
export function elapsedLabel(startedMs: number, nowMs: number): string {
  const total = Math.max(0, Math.floor((nowMs - startedMs) / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

export type RecordAction = "start" | "stop" | "wait";

/** What the record button does in each state; a start in flight can't be double-pressed. */
export function recordAction(view: MeetingRecordingView | null): RecordAction {
  if (!view || view.state === "starting") return "wait";
  return view.state === "recording" ? "stop" : "start";
}

/** "Record now" / "Stop recording" is off while busy, while the state is settling, and, for a
 *  start, while Whimbrel runs without the memory database (no-store mode). Stop always works. */
export function recordButtonDisabled(action: RecordAction, busy: boolean, noStore: boolean): boolean {
  return busy || action === "wait" || (noStore && action !== "stop");
}

/** Whether the "Record a meeting" card is drawn. The shell's recording bar and the live screen own the recording state (and say
 *  Paused while paused), so the card gives way while a meeting starts, records, pauses or is processed, and before the first
 *  status is read; it only offers Record now when nothing records. Otherwise a screen reader heard two recording timers, one
 *  of them saying Recording during a pause. */
export function recordingCardShown(view: MeetingRecordingView | null): boolean {
  return view != null && view.state === "idle" && !view.processing;
}

/** The meeting whose recording is paused right now, or null. A paused meeting is still stored as "recording" (spec:
 *  docs/ui-redesign/screens/live-meeting.md), so a row only knows it is paused by matching this id. */
export function pausedMeetingId(view: MeetingRecordingView | null): number | null {
  return view?.state === "recording" && view.paused ? (view.meetingId ?? null) : null;
}

/** The old record card's and workspace's line: "Recording paused" while paused (the tray popup's words), else
 *  "Recording · 12:04". */
export function recordingLine(paused: boolean, startedMs: number, nowMs: number): string {
  return paused ? t("Recording paused") : t("Recording · {elapsed}", { elapsed: elapsedLabel(startedMs, nowMs) });
}
