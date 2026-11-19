import type { MeetingRecordingView } from "../generated/types";

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
