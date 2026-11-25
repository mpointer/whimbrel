// What the tray popup shows in each state (redesign only). Pure: RedesignedPopup.tsx reads the shell and draws what this says.
// The order is the spec's (docs/ui-redesign/screens/menu-bar.md): Recording, Memory, Needs attention, Next meeting, then
// Open Whimbrel and Quit. The popup has no Start recording today, and none is added here.

import type { CaptureStatus, MeetingRecordingView, TrayStatus } from "../generated/types";
import { t } from "../l10n";
import { degradedShown, liveControls, liveStateOf, type LiveControls, type LiveState } from "../meetings/redesign/liveModel";
import { PAUSE_CHOICES, type PauseChoice } from "../shell/redesign/captureModel";

/* ---- Recording ---- */

/** The recording row: shown while a meeting starts, records, is paused, resumes or is in trouble. */
export interface PopupRecording {
  /** The recording's own state. A problem does not replace it: a paused recording with a problem is still paused. */
  state: "starting" | "recording" | "paused" | "resuming";
  controls: LiveControls;
  /** The recording has a problem (the shell says it may be missing audio), whether it is paused or not. */
  problem: boolean;
  /** The problem, in the shell's words; set whenever `problem` is. */
  reason?: string;
  /** "Show in-meeting window" is offered whenever the row is. */
  showMeetingWindow: boolean;
  /** Where the row's facts came from: the detailed status, or only the tray's (the detailed read is late or failed). */
  source: "detail" | "tray";
}

function rowFor(state: LiveState, degraded: string | null | undefined, busy: boolean, source: PopupRecording["source"]): PopupRecording | null {
  switch (state) {
    case "starting":
    case "recording":
    case "paused":
    case "resuming":
    case "problem": {
      // The live model calls a recording with a problem "problem" (and lets paused win over it). The popup keeps the
      // recording's state and the problem apart, so each is said.
      const running = state === "problem" ? "recording" : state;
      const problem = (running === "recording" || running === "paused") && !!degraded;
      // The live screen's controls, except that Stop is never held off, not by a Pause or Resume call in flight and not while
      // a recording starts (there it cancels the start through the same `meeting_record_stop` the live screen and the menu
      // use): Stop is the way out and is safe to press at any time.
      const controls = { ...liveControls(state, busy), stopDisabled: false };
      return { state: running, controls, problem, reason: problem ? degraded ?? undefined : undefined, showMeetingWindow: true, source };
    }
    default:
      return null;
  }
}

/** From the detailed status. The controls are the live screen's: Pause or Resume and Stop, in a problem as in any state. */
export function popupRecording(view: MeetingRecordingView | null, busy = false): PopupRecording | null {
  if (!view) return null;
  return rowFor(liveStateOf(view), view.state === "recording" ? degradedShown(view) : null, busy, "detail");
}

/** What the popup draws for recording: the row, a line saying it could not tell, or nothing. */
export type RecordingBlock = { kind: "row"; row: PopupRecording } | { kind: "unreadable"; stop: true } | null;

/**
 * The detailed status (`meeting_recording_status`) is polled; the tray's status is read once on open. If the detailed read
 * is late or fails, a recording the tray knows about still shows ("Recording", without a time it cannot know). If both
 * reads fail, the popup says so rather than showing nothing, which would read as "not recording", and still offers Stop.
 */
export function recordingBlock(input: {
  view: MeetingRecordingView | null;
  viewFailed: boolean;
  tray: Pick<TrayStatus, "recordingStartedUnixMs" | "recordingPaused"> | null;
  trayFailed: boolean;
  busy: boolean;
}): RecordingBlock {
  if (input.view) {
    const row = popupRecording(input.view, input.busy);
    return row ? { kind: "row", row } : null;
  }
  if (input.tray && input.tray.recordingStartedUnixMs != null) {
    const row = rowFor(input.tray.recordingPaused ? "paused" : "recording", null, input.busy, "tray");
    return row ? { kind: "row", row } : null;
  }
  // With the state unknown, Stop is offered (it does nothing when nothing records), and the person can read again.
  if (input.viewFailed && input.trayFailed) return { kind: "unreadable", stop: true };
  return null;
}

/* ---- Memory ---- */

/** Why memory is stopped, in words that are safe to show: the shell's raw message never is. The service only ever stops for
 *  two reasons (capture/service.rs): Windows would not give it UI Automation ("UI Automation: ..."), or it never started. An
 *  OCR failure leaves it running, so it is not a reason here. */
export type StoppedReason = "windowText" | "unknown";

/** How screen-text reading (OCR) is failing, when it is (capture/service.rs `ReadProblem`): it could not start this session, or the
 *  last read of a window's picture failed. OCR is only the fallback for windows whose text Windows will not hand over (video,
 *  remote desktop, some browser pages), so memory keeps working in every other window. */
export type ReadProblem = "unavailable" | "failed";

/** A "failed" read counts as current this long when no later OCR read has worked (capture/service.rs `OCR_PROBLEM_FRESH_MS`). */
export const OCR_PROBLEM_FRESH_MS = 10 * 60 * 1000;

/**
 * The OCR problem that is current at `now`, from the capture status. The shell clears a "failed" one when an OCR read works
 * again, and an "unavailable" one lasts until Whimbrel restarts (OCR cannot run). A "failed" one nobody has retried for ten
 * minutes is history, not news. The raw `lastError` is never read here. Same rule as capture/service.rs `read_problem`.
 */
export function ocrProblem(status: Partial<Pick<CaptureStatus, "ocrProblem" | "ocrProblemAtUnixMs">>, now: number): ReadProblem | null {
  if (status.ocrProblem === "unavailable") return "unavailable";
  if (status.ocrProblem === "failed" && status.ocrProblemAtUnixMs != null && now - status.ocrProblemAtUnixMs < OCR_PROBLEM_FRESH_MS) return "failed";
  return null;
}

export type PopupMemory =
  | { state: "off" }
  /** On. `readProblem` is set while screen-text reading (OCR) is failing: memory still works, some windows are not read. */
  | { state: "on"; readProblem?: ReadProblem }
  /** Paused by the app with no time to name (the screen is locked, say). The tray's `MemoryState::Paused`. */
  | { state: "pausedByApp" }
  /** A pause the person chose, until `resumeAt`. The tray's `MemoryState::PausedUntil`. */
  | { state: "paused"; resumeAt: number }
  | { state: "stopped"; reason: StoppedReason };

export type CaptureFields = Pick<CaptureStatus, "enabled" | "running" | "resumeAtUnixMs"> & Partial<Pick<CaptureStatus, "paused" | "lastError" | "ocrProblem" | "ocrProblemAtUnixMs">>;

/** A raw service message to a reason we can say. Never the message itself: it can be an error code or an internal detail. */
export function stoppedReason(lastError: string | null | undefined): StoppedReason {
  return lastError && /^UI Automation\b/.test(lastError) ? "windowText" : "unknown";
}

/**
 * Memory's state, mirroring the tray exactly (tray.rs `current` then `memory_state`): off wins, then a service that could not
 * start, then a timed pause still ahead, then paused by the app for another reason, else on. `lastError` only gives a stopped
 * state its reason; it never changes the state.
 */
export function popupMemory(status: CaptureFields | null, now: number): PopupMemory | null {
  if (!status) return null;
  if (!status.enabled) return { state: "off" };
  if (!status.running) return { state: "stopped", reason: stoppedReason(status.lastError) };
  const at = status.resumeAtUnixMs;
  if (at != null && at > now) return { state: "paused", resumeAt: at };
  if (status.paused != null) return { state: "pausedByApp" };
  const readProblem = ocrProblem(status, now);
  return readProblem ? { state: "on", readProblem } : { state: "on" };
}

/** The clock time of a pause's end, "3:10 PM". */
export function clockLabel(ms: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit" }).format(new Date(ms));
}

/**
 * Memory in words, one sentence. The tray says the same (tray.rs `memory_text`; its tooltip has a short form for a read problem): keep the two in step. A pause that ends on
 * another day says "tomorrow at", as the tray does, never a weekday.
 */
export function memoryWords(memory: PopupMemory, now: number, locale: string): string {
  switch (memory.state) {
    case "on":
      return memory.readProblem ? t("Memory on, but some windows could not be read") : t("Memory on");
    case "off":
      return t("Memory off");
    case "stopped":
      return t("Memory stopped");
    case "pausedByApp":
      return t("Memory paused");
    case "paused": {
      const time = clockLabel(memory.resumeAt, locale);
      return new Date(memory.resumeAt).toDateString() === new Date(now).toDateString()
        ? t("Memory paused until {time}", { time })
        : t("Memory paused until tomorrow at {time}", { time });
    }
  }
}

/** The plain sentence under "Memory stopped", or null when memory is not stopped. */
export function stoppedLine(memory: PopupMemory): string | null {
  if (memory.state !== "stopped") return null;
  return memory.reason === "windowText" ? t("Windows would not let Whimbrel read window text.") : t("Whimbrel could not start memory.");
}

/** The plain sentence under "Memory on, but some windows could not be read", or null when nothing is failing. Never the raw message. */
export function readProblemLine(memory: PopupMemory): string | null {
  if (memory.state !== "on" || !memory.readProblem) return null;
  return memory.readProblem === "unavailable"
    ? t("Reading text from pictures of windows did not start. If an OCR language is missing, Capture & Privacy in Settings shows which are installed.")
    : t("The last try at reading text from a window's picture did not work. Whimbrel will try again.");
}

/* ---- Needs attention ---- */

/** What can need attention in the popup, in this order. Google sign-in comes from the tray status (an in-memory fact the tray
 *  already keeps), never from a read on open. */
export type Attention = "noLibrary" | "googleSignIn" | "syncProblem";

export function attentionItems(input: { noStore: boolean; tray: (Pick<TrayStatus, "syncError"> & Partial<Pick<TrayStatus, "googleSignInExpired">>) | null }): Attention[] {
  const out: Attention[] = [];
  if (input.noStore) out.push("noLibrary");
  if (input.tray?.googleSignInExpired === true) out.push("googleSignIn");
  // The tray's own rule (`tray::sync_line` is Error), the same source as the tooltip's "Sync error".
  if (input.tray?.syncError === true) out.push("syncProblem");
  return out;
}

/** The quiet sync line under memory: what the old status line said ("Synced 2:05 PM", "Sync setting up"), or none. */
export function syncLine(tray: Pick<TrayStatus, "syncText"> | null): string | null {
  const text = tray?.syncText;
  return text ? text : null;
}

/** The timed choices the popup lists, in order. */
export const POPUP_PAUSE_CHOICES: readonly PauseChoice[] = PAUSE_CHOICES;

export interface PopupRows {
  recording: RecordingBlock;
  memory: PopupMemory | null;
  attention: Attention[];
  /** A next meeting with its title and time. */
  next: boolean;
}

export function popupRows(input: {
  recordingView: MeetingRecordingView | null;
  viewFailed: boolean;
  tray: Pick<TrayStatus, "recordingStartedUnixMs" | "recordingPaused" | "syncError"> & Partial<Pick<TrayStatus, "googleSignInExpired">> | null;
  trayFailed: boolean;
  busy: boolean;
  capture: CaptureFields | null;
  now: number;
  noStore: boolean;
  hasNext: boolean;
}): PopupRows {
  return {
    recording: recordingBlock({ view: input.recordingView, viewFailed: input.viewFailed, tray: input.tray, trayFailed: input.trayFailed, busy: input.busy }),
    // With no library the notice already says memory is off: one clear notice, not "Memory stopped" on top of it.
    memory: input.noStore ? null : popupMemory(input.capture, input.now),
    attention: attentionItems({ noStore: input.noStore, tray: input.tray }),
    next: input.hasNext,
  };
}
