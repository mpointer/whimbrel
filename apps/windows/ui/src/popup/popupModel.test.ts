import { describe, expect, it } from "vitest";
import type { MeetingRecordingView } from "../generated/types";
import { liveControls } from "../meetings/redesign/liveModel";
import { attentionItems, memoryWords, OCR_PROBLEM_FRESH_MS, ocrProblem, popupMemory, popupRecording, popupRows, recordingBlock, readProblemLine, stoppedLine, stoppedReason, syncLine, type CaptureFields } from "./popupModel";

const NOW = 1_800_000_000_000;
const MIN = 60_000;
const view = (more: Partial<MeetingRecordingView> = {}): MeetingRecordingView => ({
  degraded: null,
  failure: null,
  meetingId: 6,
  paused: false,
  pausedTotalMs: 0,
  processing: null,
  resuming: false,
  startedUnixMs: NOW - 14 * MIN,
  state: "recording",
  ...more,
});
const cap = (more: Partial<CaptureFields> = {}): CaptureFields => ({ enabled: true, running: true, resumeAtUnixMs: null, paused: null, lastError: null, ...more });

describe("the recording row", () => {
  it("shows while a meeting starts, records, is paused or resumes, with Pause or Resume and Stop", () => {
    const rec = popupRecording(view());
    expect(rec?.state).toBe("recording");
    expect(rec?.controls).toMatchObject({ pause: "pause", stop: true });
    expect(rec?.showMeetingWindow).toBe(true);
    expect(rec?.problem).toBe(false);
    expect(popupRecording(view({ paused: true }))?.controls.pause).toBe("resume");
    expect(popupRecording(view({ paused: true }))?.state).toBe("paused");
    expect(popupRecording(view({ resuming: true }))?.state).toBe("resuming");
    expect(popupRecording(view({ state: "starting" }))?.controls.pauseDisabled).toBe(true);
  });

  it("says a problem in the shell's words, in addition to the recording state", () => {
    const problem = popupRecording(view({ degraded: "The microphone isn't delivering audio." }));
    expect(problem?.state).toBe("recording");
    expect(problem?.problem).toBe(true);
    expect(problem?.reason).toBe("The microphone isn't delivering audio.");
    // Pause and Stop stay exactly as the commands allow today.
    expect(problem?.controls).toMatchObject({ pause: "pause", stop: true, pauseDisabled: false, stopDisabled: false });
    expect(popupRecording(view())?.reason).toBeUndefined();
  });

  it("a problem the pause did not cause, while paused, says both: paused, with the problem, and Resume", () => {
    const headset = "Your headset switched to call quality, so the meeting's audio may sound muffled.";
    const both = popupRecording(view({ paused: true, degraded: headset }));
    expect(both?.state).toBe("paused");
    expect(both?.problem).toBe(true);
    expect(both?.reason).toBe(headset);
    expect(both?.controls).toMatchObject({ pause: "resume", stop: true });
  });

  it("while paused, audio that is not delivered is the pause itself: the row says paused and no problem", () => {
    for (const reason of ["The microphone isn't delivering audio, so this meeting records system audio only.", "System audio isn't reaching Whimbrel, so this meeting records your microphone only."]) {
      const paused = popupRecording(view({ paused: true, degraded: reason }));
      expect(paused?.state).toBe("paused");
      expect(paused?.problem).toBe(false);
      expect(paused?.reason).toBeUndefined();
      expect(paused?.controls).toMatchObject({ pause: "resume", stop: true });
      // The same reason on a recording that is running is a problem.
      expect(popupRecording(view({ degraded: reason }))?.problem).toBe(true);
    }
  });

  it("is absent when nothing records, while a meeting is processed, and before the shell answers", () => {
    expect(popupRecording(null)).toBeNull();
    expect(popupRecording(view({ state: "idle", startedUnixMs: null }))).toBeNull();
    expect(popupRecording(view({ state: "idle", startedUnixMs: null, degraded: "left over", processing: { label: "Writing the summary", meetingId: 6, stage: "summarizing" } }))).toBeNull();
  });

  it("disables Pause or Resume while a command runs, but never Stop: it is the way out of a recording", () => {
    expect(popupRecording(view(), true)?.controls).toMatchObject({ pauseDisabled: true, stopDisabled: false });
    expect(popupRecording(view({ paused: true }), true)?.controls).toMatchObject({ pause: "resume", pauseDisabled: true, stopDisabled: false });
    expect(popupRecording(view({ degraded: "The microphone isn't delivering audio." }), true)?.controls).toMatchObject({ stopDisabled: false });
    expect(popupRecording(view())?.controls).toMatchObject({ pauseDisabled: false, stopDisabled: false });
    // From the tray's own status, too.
    const block = recordingBlock({ view: null, viewFailed: true, tray: { recordingStartedUnixMs: NOW, recordingPaused: false }, trayFailed: false, busy: true });
    expect(block?.kind === "row" && block.row.controls).toMatchObject({ pauseDisabled: true, stopDisabled: false });
  });

  it("offers Stop while a recording is starting, as the live screen, the main window and the menu do; it is never held off", () => {
    const starting = popupRecording(view({ state: "starting", startedUnixMs: null }));
    expect(starting?.state).toBe("starting");
    expect(starting?.controls).toMatchObject({ stop: true, stopDisabled: false, pauseDisabled: true });
    expect(popupRecording(view({ state: "starting", startedUnixMs: null }), true)?.controls).toMatchObject({ stop: true, stopDisabled: false });
    expect(popupRecording(view({ resuming: true }))?.controls).toMatchObject({ stop: true, stopDisabled: false, pauseDisabled: true });
  });

  it("gives every state the live screen's controls, with Stop never held off", () => {
    for (const [v, state] of [
      [view(), "recording"],
      [view({ paused: true }), "paused"],
      [view({ state: "starting", startedUnixMs: null }), "starting"],
      [view({ resuming: true }), "resuming"],
    ] as const) {
      for (const busy of [false, true]) expect(popupRecording(v, busy)?.controls, `${state} busy=${busy}`).toEqual({ ...liveControls(state, busy), stopDisabled: false });
    }
  });
});

describe("a recording the detailed read has not reported", () => {
  const tray = (more: Partial<{ recordingStartedUnixMs: number | null; recordingPaused: boolean }> = {}) => ({ recordingStartedUnixMs: NOW - 5 * MIN, recordingPaused: false, ...more });
  const none = { view: null, viewFailed: false, tray: null, trayFailed: false, busy: false };

  it("shows Recording from the tray's start time until the detailed status arrives", () => {
    const block = recordingBlock({ ...none, tray: tray() });
    expect(block).toMatchObject({ kind: "row", row: { state: "recording", source: "tray", problem: false } });
    expect(block?.kind === "row" && block.row.controls).toMatchObject({ pause: "pause", stop: true });
    const paused = recordingBlock({ ...none, tray: tray({ recordingPaused: true }) });
    expect(paused).toMatchObject({ kind: "row", row: { state: "paused" } });
  });

  it("still shows it when the detailed read failed", () => {
    expect(recordingBlock({ ...none, viewFailed: true, tray: tray() })).toMatchObject({ kind: "row", row: { source: "tray" } });
  });

  it("prefers the detailed status once it is there", () => {
    expect(recordingBlock({ ...none, view: view(), tray: tray() })).toMatchObject({ kind: "row", row: { source: "detail" } });
    // And when it says nothing records, the tray's older word does not bring the row back.
    expect(recordingBlock({ ...none, view: view({ state: "idle", startedUnixMs: null }), tray: tray() })).toBeNull();
  });

  it("says it could not read the state when both reads failed, rather than showing nothing", () => {
    // Stop is still offered, and enabled: Stop with nothing recording does nothing, so it is safe, and a recording that is
    // running cannot be left without a way out because a read failed.
    expect(recordingBlock({ ...none, viewFailed: true, trayFailed: true })).toEqual({ kind: "unreadable", stop: true });
  });

  it("shows nothing while a read is only late, or when the tray reports no recording", () => {
    expect(recordingBlock(none)).toBeNull();
    expect(recordingBlock({ ...none, tray: tray({ recordingStartedUnixMs: null }) })).toBeNull();
    expect(recordingBlock({ ...none, viewFailed: true, tray: tray({ recordingStartedUnixMs: null }) })).toBeNull();
    expect(recordingBlock({ ...none, viewFailed: true })).toBeNull();
  });
});

describe("memory", () => {
  it("is on, paused until a time still ahead, off, or stopped", () => {
    expect(popupMemory(cap(), NOW)).toEqual({ state: "on" });
    expect(popupMemory(cap({ resumeAtUnixMs: NOW + 28 * MIN }), NOW)).toEqual({ state: "paused", resumeAt: NOW + 28 * MIN });
    expect(popupMemory(cap({ enabled: false }), NOW)).toEqual({ state: "off" });
    expect(popupMemory(cap({ running: false }), NOW)).toEqual({ state: "stopped", reason: "unknown" });
  });

  it("treats a pause that has passed as on, and off as off whatever a stale deadline says", () => {
    expect(popupMemory(cap({ resumeAtUnixMs: NOW - 1 }), NOW)).toEqual({ state: "on" });
    expect(popupMemory(cap({ enabled: false, resumeAtUnixMs: NOW + MIN }), NOW)).toEqual({ state: "off" });
    expect(popupMemory(cap({ enabled: false, running: false }), NOW)).toEqual({ state: "off" });
  });

  it("is unknown until the shell answers", () => {
    expect(popupMemory(null, NOW)).toBeNull();
  });

  it("is paused by the app when the service says so and no deadline is ahead, as the tray says", () => {
    expect(popupMemory(cap({ paused: "the screen is locked" }), NOW)).toEqual({ state: "pausedByApp" });
    expect(popupMemory(cap({ paused: "the screen is locked", resumeAtUnixMs: NOW - 5 }), NOW)).toEqual({ state: "pausedByApp" });
    // A deadline still ahead wins over the service's own state, which can lag a moment behind a pause just set.
    expect(popupMemory(cap({ paused: "capture is paused for a set time", resumeAtUnixMs: NOW + MIN }), NOW)).toEqual({ state: "paused", resumeAt: NOW + MIN });
    expect(popupMemory(cap({ resumeAtUnixMs: NOW + MIN }), NOW)).toEqual({ state: "paused", resumeAt: NOW + MIN });
  });

  it("gives stopped a reason in plain words and never the raw message", () => {
    // The only two ways memory is stopped (src-tauri capture/service.rs, commands capture_status): the service could not
    // start because Windows would not give it UI Automation, or there is no service at all (it never started).
    const raw = "UI Automation: 0x80070005 access denied";
    const m = popupMemory(cap({ running: false, lastError: raw }), NOW)!;
    expect(m).toEqual({ state: "stopped", reason: "windowText" });
    expect(stoppedLine(m)).not.toContain("0x8007");
    expect(stoppedLine(m)).toBe("Windows would not let Whimbrel read window text.");
    expect(stoppedLine(popupMemory(cap({ running: false, lastError: "capture did not start; see About for the startup error" }), NOW)!)).toBe("Whimbrel could not start memory.");
    expect(stoppedLine(popupMemory(cap({ running: false }), NOW)!)).toBe("Whimbrel could not start memory.");
    // An OCR message is not a reason memory is stopped: the service keeps running after one, so it is never named here.
    expect(stoppedReason("OCR unavailable: 0x80070005")).toBe("unknown");
    expect(stoppedLine(popupMemory(cap(), NOW)!)).toBeNull();
    // lastError alone never changes the state: an OCR failure leaves memory on.
    expect(popupMemory(cap({ lastError: "OCR: x" }), NOW)).toEqual({ state: "on" });
    expect(popupMemory(cap({ lastError: "OCR unavailable: x" }), NOW)).toEqual({ state: "on" });
  });
});

describe("screen-text reading that is failing", () => {
  const failed = { ocrProblem: "failed", ocrProblemAtUnixMs: NOW - MIN };

  it("is current when it could not start, or when the last read failed recently", () => {
    expect(ocrProblem({ ocrProblem: "unavailable", ocrProblemAtUnixMs: NOW - 100 * OCR_PROBLEM_FRESH_MS }, NOW)).toBe("unavailable");
    expect(ocrProblem(failed, NOW)).toBe("failed");
    expect(ocrProblem({ ocrProblem: "failed", ocrProblemAtUnixMs: NOW - OCR_PROBLEM_FRESH_MS + 1 }, NOW)).toBe("failed");
  });

  it("is over once a read works (the shell clears it), and old news after ten minutes, and never an unknown kind", () => {
    expect(ocrProblem({ ocrProblem: null, ocrProblemAtUnixMs: null }, NOW)).toBeNull();
    expect(ocrProblem({}, NOW)).toBeNull();
    expect(ocrProblem({ ocrProblem: "failed", ocrProblemAtUnixMs: NOW - OCR_PROBLEM_FRESH_MS }, NOW)).toBeNull();
    expect(ocrProblem({ ocrProblem: "failed", ocrProblemAtUnixMs: null }, NOW)).toBeNull();
    expect(ocrProblem({ ocrProblem: "OCR: 0x80004005", ocrProblemAtUnixMs: NOW }, NOW)).toBeNull();
  });

  it("changes only memory that is on, and never the state, the pause choices or the raw error", () => {
    expect(popupMemory(cap(failed), NOW)).toEqual({ state: "on", readProblem: "failed" });
    expect(popupMemory(cap({ ...failed, lastError: "OCR: 0x80004005 raw" }), NOW)).toEqual({ state: "on", readProblem: "failed" });
    expect(popupMemory(cap({ ...failed, enabled: false }), NOW)).toEqual({ state: "off" });
    expect(popupMemory(cap({ ...failed, running: false }), NOW)).toEqual({ state: "stopped", reason: "unknown" });
    expect(popupMemory(cap({ ...failed, paused: "the screen is locked" }), NOW)).toEqual({ state: "pausedByApp" });
    expect(popupMemory(cap({ ...failed, resumeAtUnixMs: NOW + MIN }), NOW)).toEqual({ state: "paused", resumeAt: NOW + MIN });
  });

  it("says so in memory's words, the with one plain reason line and never the raw message", () => {
    const m = popupMemory(cap({ ocrProblem: "unavailable", ocrProblemAtUnixMs: NOW, lastError: "OCR unavailable: 0x80070490" }), NOW)!;
    expect(memoryWords(m, NOW, "en-US")).toBe("Memory on, but some windows could not be read");
    expect(readProblemLine(m)).toBe("Reading text from pictures of windows did not start. If an OCR language is missing, Capture & Privacy in Settings shows which are installed.");
    expect(readProblemLine(popupMemory(cap(failed), NOW)!)).toBe("The last try at reading text from a window's picture did not work. Whimbrel will try again.");
    expect(readProblemLine(popupMemory(cap(), NOW)!)).toBeNull();
    expect(readProblemLine({ state: "off" })).toBeNull();
    for (const line of [readProblemLine(m), readProblemLine(popupMemory(cap(failed), NOW)!)]) expect(line).not.toMatch(/0x|OCR:|unavailable:/);
  });
});

// One row per capture status the tray knows (tray.rs MemoryState: On, PausedUntil, Paused, Off, Stopped), with the words the
// tray's `memory_text` gives it. The Rust test `the_memory_words_per_capture_status` lists the same rows: change both together.
describe("memory words, the same in the tray and the popup", () => {
  const at = (h: number, m: number, day = 18) => new Date(2026, 10, day, h, m).getTime();
  const noon = at(12, 0);
  const time = (ms: number) => new Intl.DateTimeFormat("en-US", { hour: "numeric", minute: "2-digit" }).format(new Date(ms));
  const rows: [string, CaptureFields, (now: number) => string][] = [
    ["on", cap(), () => "Memory on"],
    ["paused until a time today", cap({ paused: "capture is paused for a set time", resumeAtUnixMs: at(15, 10) }), () => `Memory paused until ${time(at(15, 10))}`],
    ["paused until another day", cap({ paused: "capture is paused for a set time", resumeAtUnixMs: at(4, 0, 19) }), () => `Memory paused until tomorrow at ${time(at(4, 0, 19))}`],
    ["paused by the app", cap({ paused: "the screen is locked" }), () => "Memory paused"],
    ["off", cap({ enabled: false }), () => "Memory off"],
    ["stopped", cap({ running: false }), () => "Memory stopped"],
    // The tooltip's `memory_text_with_problem` says it in short form, "Memory on, some windows not read" (tray.rs
    // `the_memory_words_per_capture_status`): the state and when it applies are the same, the words are the full sentence here.
    ["on, but some windows could not be read", cap({ ocrProblem: "failed", ocrProblemAtUnixMs: noon }), () => "Memory on, but some windows could not be read"],
  ];
  for (const [name, status, words] of rows)
    it(name, () => {
      expect(memoryWords(popupMemory(status, noon)!, noon, "en-US")).toBe(words(noon));
    });

  it("never says a weekday", () => {
    expect(memoryWords({ state: "paused", resumeAt: at(4, 0, 19) }, noon, "en-US")).not.toMatch(/Thu|Fri|Mon|Tue|Wed|Sat|Sun/);
  });
});

describe("needs attention", () => {
  it("is empty when nothing is wrong", () => {
    expect(attentionItems({ noStore: false, tray: { syncError: false } })).toEqual([]);
    expect(attentionItems({ noStore: false, tray: null })).toEqual([]);
  });

  it("lists the library notice, then a sync problem, from the tray's own flag", () => {
    expect(attentionItems({ noStore: true, tray: { syncError: true } })).toEqual(["noLibrary", "syncProblem"]);
    expect(attentionItems({ noStore: false, tray: { syncError: true } })).toEqual(["syncProblem"]);
  });

  it("lists an expired Google sign-in from the tray's own flag, after the library notice and before a sync problem", () => {
    expect(attentionItems({ noStore: false, tray: { syncError: false, googleSignInExpired: true } })).toEqual(["googleSignIn"]);
    expect(attentionItems({ noStore: true, tray: { syncError: true, googleSignInExpired: true } })).toEqual(["noLibrary", "googleSignIn", "syncProblem"]);
    expect(attentionItems({ noStore: false, tray: { syncError: false, googleSignInExpired: false } })).toEqual([]);
  });
});

describe("the sync line", () => {
  it("is what the old status line said, or nothing", () => {
    expect(syncLine({ syncText: "Synced 2:05 PM" })).toBe("Synced 2:05 PM");
    expect(syncLine({ syncText: "Sync setting up" })).toBe("Sync setting up");
    expect(syncLine({ syncText: "" })).toBeNull();
    expect(syncLine(null)).toBeNull();
  });
});

describe("the popup's rows", () => {
  const base = { viewFailed: false, tray: null, trayFailed: false, busy: false, now: NOW, noStore: false, hasNext: true };

  it("shows Memory and the next meeting, and no recording, when idle", () => {
    const rows = popupRows({ ...base, recordingView: view({ state: "idle", startedUnixMs: null }), capture: cap() });
    expect(rows.recording).toBeNull();
    expect(rows.memory).toEqual({ state: "on" });
    expect(rows.attention).toEqual([]);
    expect(rows.next).toBe(true);
  });

  it("adds the recording row above Memory while a meeting records, and never removes Memory", () => {
    const rows = popupRows({ ...base, recordingView: view(), capture: cap({ enabled: false }) });
    expect(rows.recording).toMatchObject({ kind: "row", row: { state: "recording" } });
    expect(rows.memory).toEqual({ state: "off" });
  });

  it("leaves out the next meeting when there is none", () => {
    expect(popupRows({ ...base, hasNext: false, recordingView: null, capture: cap() }).next).toBe(false);
  });

  it("leaves out Memory until the shell has answered, never guessing a state", () => {
    expect(popupRows({ ...base, recordingView: null, capture: null }).memory).toBeNull();
  });

  it("with no library says so once: the notice, and no Memory stopped on top of it", () => {
    const rows = popupRows({ ...base, noStore: true, recordingView: null, capture: cap({ running: false }) });
    expect(rows.attention).toEqual(["noLibrary"]);
    expect(rows.memory).toBeNull();
  });
});
