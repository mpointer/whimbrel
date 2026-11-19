import { describe, expect, it } from "vitest";
import { captureStateOf, nextCaptureState, pauseDeadline, pauseEndLabel, type CaptureState } from "./captureModel";

const on: CaptureState = { kind: "on" };
const off: CaptureState = { kind: "off" };
const MIN = 60_000;
// A local time on 18 Nov 2026 (the machine's zone: the model works in local time on purpose).
const at = (h: number, m = 0, day = 18) => new Date(2026, 10, day, h, m, 0, 0).getTime();

describe("pauseDeadline", () => {
  it("adds 15 minutes and an hour to now", () => {
    expect(pauseDeadline("15m", at(10, 22))).toBe(at(10, 37));
    expect(pauseDeadline("1h", at(10, 22))).toBe(at(11, 22));
  });
  it("ends 'until tomorrow' at 4:00 AM local time on the next calendar day", () => {
    expect(pauseDeadline("tomorrow", at(10, 22))).toBe(at(4, 0, 19));
    expect(pauseDeadline("tomorrow", at(23, 59))).toBe(at(4, 0, 19));
    expect(pauseDeadline("tomorrow", at(4, 0))).toBe(at(4, 0, 19));
  });
  it("between midnight and 4:00 AM it ends at 4:00 AM the same day, as the Mac does", () => {
    expect(pauseDeadline("tomorrow", at(0, 30))).toBe(at(4, 0));
    expect(pauseDeadline("tomorrow", at(3, 59))).toBe(at(4, 0));
  });
  it("rolls over a month end", () => {
    expect(pauseDeadline("tomorrow", new Date(2026, 10, 30, 12, 0).getTime())).toBe(new Date(2026, 11, 1, 4, 0).getTime());
  });
});

describe("the state table", () => {
  const now = at(10, 22);
  const paused = (resumeAt: number): CaptureState => ({ kind: "paused", resumeAt });

  it("pause for a time: on to paused with the deadline set", () => {
    expect(nextCaptureState(on, { kind: "pause", choice: "15m", now })).toEqual(paused(now + 15 * MIN));
  });
  it("pause for a time: paused to paused with the deadline replaced", () => {
    expect(nextCaptureState(paused(now + 5 * MIN), { kind: "pause", choice: "1h", now })).toEqual(paused(now + 60 * MIN));
  });
  it("pause for a time: off stays off with no deadline (the menu does not offer it)", () => {
    expect(nextCaptureState(off, { kind: "pause", choice: "15m", now })).toEqual(off);
  });
  it("resume now: paused to on, deadline cleared", () => {
    expect(nextCaptureState(paused(now + 5 * MIN), { kind: "resumeNow" })).toEqual(on);
  });
  it("resume now changes nothing when there is no pause", () => {
    expect(nextCaptureState(on, { kind: "resumeNow" })).toEqual(on);
    expect(nextCaptureState(off, { kind: "resumeNow" })).toEqual(off);
  });
  it("turn off: on or paused to off", () => {
    expect(nextCaptureState(on, { kind: "turnOff" })).toEqual(off);
    expect(nextCaptureState(paused(now + 5 * MIN), { kind: "turnOff" })).toEqual(off);
  });
  it("turn on: off to on, no deadline", () => {
    expect(nextCaptureState(off, { kind: "turnOn" })).toEqual(on);
  });
  it("time reached while the app runs: paused to on", () => {
    expect(nextCaptureState(paused(now + 5 * MIN), { kind: "clock", now: now + 5 * MIN })).toEqual(on);
    expect(nextCaptureState(paused(now + 5 * MIN), { kind: "clock", now: now + 4 * MIN })).toEqual(paused(now + 5 * MIN));
  });
  it("expiry never turns on capture the person turned off", () => {
    expect(nextCaptureState(off, { kind: "clock", now: now + 60 * MIN })).toEqual(off);
    expect(nextCaptureState(off, { kind: "appStart", now: now + 60 * MIN })).toEqual(off);
  });
  it("app starts, time not reached: still paused, deadline kept", () => {
    expect(nextCaptureState(paused(now + 5 * MIN), { kind: "appStart", now })).toEqual(paused(now + 5 * MIN));
  });
  it("app starts after the time (the app was not running at the time): on", () => {
    expect(nextCaptureState(paused(now - MIN), { kind: "appStart", now })).toEqual(on);
  });
  it("delete everything: any state to off", () => {
    expect(nextCaptureState(on, { kind: "deleteEverything" })).toEqual(off);
    expect(nextCaptureState(paused(now + MIN), { kind: "deleteEverything" })).toEqual(off);
    expect(nextCaptureState(off, { kind: "deleteEverything" })).toEqual(off);
  });
  it("wake from sleep compares the clock: on once the time has passed, paused before it", () => {
    expect(nextCaptureState(paused(now + 30 * MIN), { kind: "clock", now: now + 45 * MIN })).toEqual(on);
    expect(nextCaptureState(paused(now + 30 * MIN), { kind: "clock", now: now + 10 * MIN })).toEqual(paused(now + 30 * MIN));
  });
});

describe("captureStateOf", () => {
  const now = at(10, 22);
  it("is unknown until the shell answers", () => expect(captureStateOf(null, now)).toBeNull());
  it("reads off, on and paused from the shell's status", () => {
    expect(captureStateOf({ enabled: false }, now)).toEqual(off);
    expect(captureStateOf({ enabled: true }, now)).toEqual(on);
    expect(captureStateOf({ enabled: true, resumeAtUnixMs: now + MIN }, now)).toEqual({ kind: "paused", resumeAt: now + MIN });
  });
  it("ignores a deadline that has passed, and any deadline while off", () => {
    expect(captureStateOf({ enabled: true, resumeAtUnixMs: now - MIN }, now)).toEqual(on);
    expect(captureStateOf({ enabled: false, resumeAtUnixMs: now + MIN }, now)).toEqual(off);
  });
});

describe("pauseEndLabel", () => {
  it("writes the clock time in the locale, with the weekday when it is another day", () => {
    const flat = (x: string) => x.replace(/\s/g, " ");
    expect(flat(pauseEndLabel(at(15, 10), at(10, 22), "en-US"))).toBe("3:10 PM");
    expect(flat(pauseEndLabel(at(4, 0, 19), at(10, 22), "en-US"))).toBe("Thu 4:00 AM");
  });
});
