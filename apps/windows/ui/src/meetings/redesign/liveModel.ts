// The live meeting screen's state, as plain functions of the recording status (redesign flag on only). The shell
// keeps the recording state in `MeetingRecordingView`; the pause fields are separate (`paused`, `resuming`, ...)
// because the state itself stays "recording" for everything that asks. Spec: docs/ui-redesign/screens/live-meeting.md.

import type { MeetingRecordingView } from "../../generated/types";

/** What the bar and the controls show. `idle` means nothing records and nothing is being processed. */
export type LiveState = "idle" | "starting" | "recording" | "paused" | "resuming" | "processing" | "problem" | "readOnly";

export interface LiveOptions {
  /** The meeting belongs to another device: no controls, no banner. */
  readOnly?: boolean;
}

/**
 * The state for a status. A paused recording is paused whatever else is set (a degraded reason stays visible as a
 * warning); "resuming" is only the stretch between the resume command and audio arriving; a recording with a
 * degraded reason is a problem: the bar says so and Stop stays available.
 */
export function liveStateOf(view: MeetingRecordingView | null, options: LiveOptions = {}): LiveState {
  if (options.readOnly) return "readOnly";
  if (!view) return "idle";
  if (view.state === "starting") return "starting";
  if (view.state === "recording") {
    if (view.paused) return "paused";
    if (view.resuming) return "resuming";
    if (view.degraded) return "problem";
    return "recording";
  }
  if (view.processing) return "processing";
  return "idle";
}

/** Whether the live screen takes Home's place: a meeting is starting, recording, paused or resuming (or in trouble). */
export function liveScreenActive(state: LiveState): boolean {
  return state === "starting" || state === "recording" || state === "paused" || state === "resuming" || state === "problem";
}

type PauseFields = Pick<MeetingRecordingView, "startedUnixMs" | "paused" | "pausedTotalMs">;

/**
 * Recorded time: wall time since the start minus time spent paused. `pausedTotalMs` counts a running pause up to the
 * moment the status was read (`readAt`), so while paused the answer is frozen at that moment; otherwise it runs to
 * `now`. The stored audio keeps wall-clock time (silence for the pause); this is only what the screen shows.
 */
export function recordedMs(view: PauseFields, now: number, readAt: number = now): number {
  if (view.startedUnixMs == null) return 0;
  const wall = view.paused ? readAt : now;
  return Math.max(0, wall - view.startedUnixMs - view.pausedTotalMs);
}

/** Whole seconds of recorded time, for the bar. */
export function recordedSeconds(view: PauseFields, now: number, readAt: number = now): number {
  return Math.floor(recordedMs(view, now, readAt) / 1000);
}

/** A moment's offset from the start of the recording, in centiseconds, on the clock live transcript lines and alerts
 *  use. It includes paused time, so it is not the recorded time shown on screen. */
export function clockOffsetCs(startedUnixMs: number, now: number): number {
  return Math.max(0, Math.round((now - startedUnixMs) / 10));
}

export interface LiveControls {
  /** Which of the two the pause button is, or none (processing, read only, idle). */
  pause: "pause" | "resume" | null;
  pauseDisabled: boolean;
  /** Stop is drawn and enabled whenever a recording exists; it never waits on a pause. */
  stop: boolean;
  stopDisabled: boolean;
  /** Mark this moment needs audio running: not while starting, paused, resuming or processing. */
  markEnabled: boolean;
}

/** The controls each state offers. Starting disables them; Stop stays available in a problem and while paused. */
export function liveControls(state: LiveState, busy = false): LiveControls {
  switch (state) {
    case "starting":
      return { pause: "pause", pauseDisabled: true, stop: true, stopDisabled: true, markEnabled: false };
    case "recording":
    case "problem":
      return { pause: "pause", pauseDisabled: busy, stop: true, stopDisabled: busy, markEnabled: true };
    case "paused":
      return { pause: "resume", pauseDisabled: busy, stop: true, stopDisabled: busy, markEnabled: false };
    case "resuming":
      return { pause: "resume", pauseDisabled: true, stop: true, stopDisabled: busy, markEnabled: false };
    default:
      return { pause: null, pauseDisabled: true, stop: false, stopDisabled: true, markEnabled: false };
  }
}

/** What Ctrl+Shift+P does in a state: pause a recording, resume a paused one, nothing otherwise. */
export function pauseToggle(state: LiveState): "pause" | "resume" | null {
  if (state === "recording" || state === "problem") return "pause";
  if (state === "paused") return "resume";
  return null;
}

/** The word a screen reader hears once when the state changes, or null when nothing should be announced. The recorded
 *  time is never announced every second. */
export type Announcement = "paused" | "recording" | null;

export function announcementFor(previous: LiveState, next: LiveState): Announcement {
  if (previous === next) return null;
  if (next === "paused") return "paused";
  if (next === "recording" && (previous === "paused" || previous === "resuming")) return "recording";
  return null;
}

/** The meter's accessible name: "Audio is arriving" or "No audio right now". */
export const AUDIO_THRESHOLD = 0.02;
export function audioIsArriving(level: number): boolean {
  return level >= AUDIO_THRESHOLD;
}
