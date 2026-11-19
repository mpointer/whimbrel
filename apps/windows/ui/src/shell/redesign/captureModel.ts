// The capture state and the timed pause (docs/ui-redesign/screens/home.md, "Timed pause for capture"). Pure: the
// state table lives here, and the shell (src-tauri capture/service.rs) follows the same rows.
//
// "On" and "off" are today's states. A pause is on with a deadline: `capture.enabled` stays true while it runs,
// so the deadline passing can only undo the pause, never turn on capture the person turned off.

import type { CaptureStatus } from "../../generated/types";

export type CaptureState = { kind: "on" } | { kind: "off" } | { kind: "paused"; resumeAt: number };

/** The three timed choices the spec lists. */
export type PauseChoice = "15m" | "1h" | "tomorrow";

export const PAUSE_CHOICES: readonly PauseChoice[] = ["15m", "1h", "tomorrow"];

const MINUTE = 60_000;

/** The hour "Pause until tomorrow" ends, local time. */
export const DAY_ROLLS_AT_HOUR = 4;

/**
 * When a timed pause chosen at `now` ends. "Until tomorrow" is 4:00 AM local time on the next calendar day, except
 * between midnight and 4:00 AM, when it is 4:00 AM the same day: a late night is still today. The Mac's
 * `CapturePauseModel` follows the same rule.
 */
export function pauseDeadline(choice: PauseChoice, now: number): number {
  if (choice === "15m") return now + 15 * MINUTE;
  if (choice === "1h") return now + 60 * MINUTE;
  const d = new Date(now);
  const sameDay = new Date(d.getFullYear(), d.getMonth(), d.getDate(), DAY_ROLLS_AT_HOUR, 0, 0, 0).getTime();
  if (now < sameDay) return sameDay;
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1, DAY_ROLLS_AT_HOUR, 0, 0, 0).getTime();
}

/** Everything that can happen to the state, one per row of the spec's table. */
export type CaptureEvent =
  | { kind: "pause"; choice: PauseChoice; now: number }
  | { kind: "resumeNow" }
  | { kind: "turnOff" }
  | { kind: "turnOn" }
  /** The time reached while the app runs, and a wake from sleep: both compare the wall clock. */
  | { kind: "clock"; now: number }
  /** The app starts: a stored deadline still ahead keeps the pause, one that passed is over. */
  | { kind: "appStart"; now: number }
  | { kind: "deleteEverything" };

/** The state table. A pause is only offered while capture is on, so "pause from off" changes nothing. */
export function nextCaptureState(state: CaptureState, event: CaptureEvent): CaptureState {
  switch (event.kind) {
    case "pause":
      return state.kind === "off" ? state : { kind: "paused", resumeAt: pauseDeadline(event.choice, event.now) };
    case "resumeNow":
      return state.kind === "paused" ? { kind: "on" } : state;
    case "turnOff":
    case "deleteEverything":
      return { kind: "off" };
    case "turnOn":
      return state.kind === "off" ? { kind: "on" } : state;
    case "clock":
    case "appStart":
      return state.kind === "paused" && event.now >= state.resumeAt ? { kind: "on" } : state;
  }
}

/** The state the shell reports. `enabled` false is off whatever a stale deadline says; a deadline ahead is a pause. */
export function captureStateOf(status: Pick<CaptureStatus, "enabled" | "resumeAtUnixMs"> | null, now: number): CaptureState | null {
  if (!status) return null;
  if (!status.enabled) return { kind: "off" };
  const at = status.resumeAtUnixMs;
  return at != null && at > now ? { kind: "paused", resumeAt: at } : { kind: "on" };
}

/** The text after "Paused until": "3:10 PM" for today, "Thu 4:00 AM" for another day (the Mac's `untilText`). */
export function pauseEndLabel(resumeAt: number, now: number, locale: string): string {
  const at = new Date(resumeAt);
  const time = new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit" }).format(at);
  if (at.toDateString() === new Date(now).toDateString()) return time;
  return `${new Intl.DateTimeFormat(locale, { weekday: "short" }).format(at)} ${time}`;
}
