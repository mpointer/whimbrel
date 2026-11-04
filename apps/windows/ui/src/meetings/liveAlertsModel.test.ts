import { describe, expect, it } from "vitest";
import { POLL_WINDOW_MS, shouldPoll } from "./liveAlertsModel";

const now = 10 * POLL_WINDOW_MS;
const base = { alertsOn: true, recording: false, endedUnixMs: null, rows: [], nowUnixMs: now };

describe("shouldPoll", () => {
  it("never polls with alerts off", () => {
    expect(shouldPoll({ ...base, alertsOn: false, recording: true })).toBe(false);
  });

  it("polls while recording and for 30 minutes after the meeting ends", () => {
    expect(shouldPoll({ ...base, recording: true })).toBe(true);
    expect(shouldPoll({ ...base, endedUnixMs: now - POLL_WINDOW_MS + 1 })).toBe(true);
    expect(shouldPoll({ ...base, endedUnixMs: now - POLL_WINDOW_MS })).toBe(false);
  });

  it("polls while a recent alert waits for its draft", () => {
    expect(shouldPoll({ ...base, rows: [{ draft: null, createdAtUnixMs: now - 60_000 }] })).toBe(true);
    expect(shouldPoll({ ...base, rows: [{ draft: "Here", createdAtUnixMs: now - 60_000 }] })).toBe(false);
    expect(shouldPoll({ ...base, rows: [{ draft: null, createdAtUnixMs: now - POLL_WINDOW_MS }] })).toBe(false);
  });
});
