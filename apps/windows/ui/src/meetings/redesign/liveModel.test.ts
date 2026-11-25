import { describe, expect, it } from "vitest";
import type { MeetingRecordingView } from "../../generated/types";
import { announcementFor, audioIsArriving, clockOffsetCs, degradedShown, liveControls, liveScreenActive, liveStateOf, pauseToggle, recordedMs, recordedSeconds, statusPollMs } from "./liveModel";

const idle: MeetingRecordingView = { state: "idle", paused: false, pausedTotalMs: 0, resuming: false };
const recording = (extra: Partial<MeetingRecordingView> = {}): MeetingRecordingView => ({
  state: "recording",
  meetingId: 6,
  startedUnixMs: 1_000_000,
  paused: false,
  pausedTotalMs: 0,
  resuming: false,
  ...extra,
});
const MIN = 60_000;

describe("the state of the screen, row by row from the spec's table", () => {
  it("nothing recording and nothing processing is idle", () => {
    expect(liveStateOf(null)).toBe("idle");
    expect(liveStateOf(idle)).toBe("idle");
  });
  it("a start in flight is starting", () => {
    expect(liveStateOf({ ...idle, state: "starting" })).toBe("starting");
  });
  it("pause: recording to paused", () => {
    expect(liveStateOf(recording())).toBe("recording");
    expect(liveStateOf(recording({ paused: true, pausedSinceUnixMs: 1_100_000 }))).toBe("paused");
  });
  it("resume: paused to resuming until audio arrives, then recording", () => {
    expect(liveStateOf(recording({ paused: true }))).toBe("paused");
    expect(liveStateOf(recording({ resuming: true, pausedTotalMs: 5_000 }))).toBe("resuming");
    expect(liveStateOf(recording({ pausedTotalMs: 5_000 }))).toBe("recording");
  });
  it("stop from recording or paused: the shell reports processing", () => {
    const processing = { meetingId: 6, label: "Transcribing" } as MeetingRecordingView["processing"];
    expect(liveStateOf({ ...idle, processing })).toBe("processing");
    expect(liveStateOf(recording({ processing }))).toBe("recording");
  });
  it("a capture that fails to restart on resume is a problem, and the problem wins over resuming ending", () => {
    expect(liveStateOf(recording({ degraded: "The microphone isn't delivering audio, so this meeting records system audio only." }))).toBe("problem");
  });
  it("a paused meeting stays paused even with an old degraded reason", () => {
    expect(liveStateOf(recording({ paused: true, degraded: "x" }))).toBe("paused");
  });
  it("a pause hides only the problems it causes: audio that is not delivered, never a headset, start or save problem", () => {
    const mic = "The microphone isn't delivering audio, so this meeting records system audio only.";
    const system = "System audio isn't reaching Whimbrel, so this meeting records your microphone only.";
    const headset = "Your headset switched to call quality, so the meeting's audio may sound muffled.";
    const start = "Whimbrel couldn't start recording.";
    expect(degradedShown(null)).toBeNull();
    expect(degradedShown(recording())).toBeNull();
    // Not paused: every problem is shown.
    for (const reason of [mic, system, headset, start]) expect(degradedShown(recording({ degraded: reason }))).toBe(reason);
    // Paused: the two "not delivering" lines are the pause itself.
    expect(degradedShown(recording({ paused: true, degraded: mic }))).toBeNull();
    expect(degradedShown(recording({ paused: true, degraded: system }))).toBeNull();
    // Paused: a problem the pause did not cause is still there.
    expect(degradedShown(recording({ paused: true, degraded: headset }))).toBe(headset);
    expect(degradedShown(recording({ paused: true, degraded: start }))).toBe(start);
  });
  it("another device's meeting is read only whatever the status says", () => {
    expect(liveStateOf(recording(), { readOnly: true })).toBe("readOnly");
    expect(liveStateOf(null, { readOnly: true })).toBe("readOnly");
  });
  it("the live screen takes Home's place while a meeting starts, records, pauses, resumes or has a problem", () => {
    for (const s of ["starting", "recording", "paused", "resuming", "problem"] as const) expect(liveScreenActive(s)).toBe(true);
    for (const s of ["idle", "processing", "readOnly"] as const) expect(liveScreenActive(s)).toBe(false);
  });
});

describe("recorded time is wall time minus paused time", () => {
  const start = 1_000_000;
  it("runs with the clock when nothing was paused", () => {
    expect(recordedMs(recording(), start + 14 * MIN)).toBe(14 * MIN);
    expect(recordedSeconds(recording(), start + 14 * MIN + 8_000)).toBe(14 * 60 + 8);
  });
  it("leaves out a finished pause", () => {
    // 10 minutes in, paused for 3: at 14 minutes of wall time the screen shows 11.
    expect(recordedMs(recording({ pausedTotalMs: 3 * MIN }), start + 14 * MIN)).toBe(11 * MIN);
  });
  it("freezes while paused, whatever 'now' is", () => {
    // Status read at 12 min of wall time; the pause began at 10, so 2 minutes of it have counted already.
    const view = recording({ paused: true, pausedSinceUnixMs: start + 10 * MIN, pausedTotalMs: 2 * MIN });
    const readAt = start + 12 * MIN;
    expect(recordedMs(view, readAt, readAt)).toBe(10 * MIN);
    expect(recordedMs(view, start + 30 * MIN, readAt)).toBe(10 * MIN);
  });
  it("adds earlier pauses to a running one", () => {
    // A 3 minute pause, then recording, then paused again from minute 20; read at minute 25 (5 + 3 paused).
    const view = recording({ paused: true, pausedSinceUnixMs: start + 20 * MIN, pausedTotalMs: 8 * MIN });
    expect(recordedMs(view, start + 25 * MIN, start + 25 * MIN)).toBe(17 * MIN);
  });
  it("never goes negative and is zero before a start time exists", () => {
    expect(recordedMs(recording({ pausedTotalMs: 99 * MIN }), start + MIN)).toBe(0);
    expect(recordedMs({ startedUnixMs: null, paused: false, pausedTotalMs: 0 }, 5)).toBe(0);
  });
});

describe("the marked moment's time is on the clock transcript lines use", () => {
  it("includes paused time, unlike the recorded time shown", () => {
    const start = 1_000_000;
    const view = recording({ pausedTotalMs: 3 * MIN });
    const now = start + 14 * MIN;
    expect(recordedMs(view, now)).toBe(11 * MIN);
    expect(clockOffsetCs(start, now)).toBe(14 * 60 * 100);
  });
  it("a mark made after a pause lands at the wall-clock offset, not the recorded time on screen", () => {
    const start = 1_000_000;
    // 10 minutes in, paused 5, resumed; marked at minute 20 of wall time with 5 minutes paused: 15 recorded, 20 on the clock.
    const view = recording({ pausedTotalMs: 5 * MIN });
    const now = start + 20 * MIN;
    expect(recordedMs(view, now)).toBe(15 * MIN);
    expect(clockOffsetCs(start, now)).toBe(20 * 60 * 100);
    expect(clockOffsetCs(start, now)).not.toBe((recordedMs(view, now) / 10) | 0);
  });
  it("rounds to a centisecond and never goes negative", () => {
    expect(clockOffsetCs(0, 1_234)).toBe(123);
    expect(clockOffsetCs(5_000, 1_000)).toBe(0);
  });
});

describe("the controls", () => {
  it("starting disables Pause and Mark but offers Stop, which cancels the start (meeting_record_stop)", () => {
    expect(liveControls("starting")).toEqual({ pause: "pause", pauseDisabled: true, stop: true, stopDisabled: false, markEnabled: false });
    // A command already running still holds the buttons.
    expect(liveControls("starting", true)).toMatchObject({ stop: true, stopDisabled: true });
  });
  it("recording offers Pause, Stop and Mark; a busy command disables the buttons", () => {
    expect(liveControls("recording")).toMatchObject({ pause: "pause", pauseDisabled: false, stopDisabled: false, markEnabled: true });
    expect(liveControls("recording", true)).toMatchObject({ pauseDisabled: true, stopDisabled: true });
  });
  it("paused offers Resume and Stop, and no Mark", () => {
    expect(liveControls("paused")).toMatchObject({ pause: "resume", pauseDisabled: false, stop: true, stopDisabled: false, markEnabled: false });
  });
  it("resuming waits for capture to start again but Stop stays available", () => {
    expect(liveControls("resuming")).toMatchObject({ pause: "resume", pauseDisabled: true, stopDisabled: false });
  });
  it("Mark works while resuming: the moment is on the recording's clock, so a mark pressed then is kept, not dropped", () => {
    expect(liveControls("resuming").markEnabled).toBe(true);
  });
  it("a problem keeps Stop", () => {
    expect(liveControls("problem")).toMatchObject({ stop: true, stopDisabled: false });
  });
  it("processing and read only offer no controls", () => {
    for (const s of ["processing", "readOnly", "idle"] as const) expect(liveControls(s)).toMatchObject({ pause: null, stop: false });
  });
});

describe("Ctrl+Shift+P", () => {
  it("pauses a recording, resumes a paused one and does nothing in any other state", () => {
    expect(pauseToggle("recording")).toBe("pause");
    expect(pauseToggle("problem")).toBe("pause");
    expect(pauseToggle("paused")).toBe("resume");
    for (const s of ["idle", "starting", "resuming", "processing", "readOnly"] as const) expect(pauseToggle(s)).toBeNull();
  });
});

describe("announcements", () => {
  it("says 'paused' once on pause and 'recording' once on return", () => {
    expect(announcementFor("recording", "paused")).toBe("paused");
    expect(announcementFor("paused", "paused")).toBeNull();
    expect(announcementFor("paused", "resuming")).toBeNull();
    expect(announcementFor("resuming", "recording")).toBe("recording");
    expect(announcementFor("paused", "recording")).toBe("recording");
  });
  it("says nothing for a start or a stop", () => {
    expect(announcementFor("idle", "recording")).toBeNull();
    expect(announcementFor("starting", "recording")).toBeNull();
    expect(announcementFor("recording", "processing")).toBeNull();
  });
});

describe("the level meter's name", () => {
  it("is arriving above a floor and not otherwise", () => {
    expect(audioIsArriving(0)).toBe(false);
    expect(audioIsArriving(0.01)).toBe(false);
    expect(audioIsArriving(0.2)).toBe(true);
  });
});

describe("how often the status is read", () => {
  it("every 3 s as before, and quickly while a recording is starting or resuming, so the bar leaves Resuming as soon as capture is back", () => {
    expect(statusPollMs("recording")).toBe(3_000);
    expect(statusPollMs("paused")).toBe(3_000);
    expect(statusPollMs("idle")).toBe(3_000);
    expect(statusPollMs("resuming")).toBeLessThanOrEqual(500);
    expect(statusPollMs("starting")).toBeLessThanOrEqual(500);
  });
});
