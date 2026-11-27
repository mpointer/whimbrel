import { describe, expect, it, vi } from "vitest";

vi.mock("../api", () => {
  const never = () => new Promise(() => {});
  return {
    meetingRecordResume: never,
    meetingRecordStop: never,
    meetingRecordingStatus: never,
    stillHereKeep: never,
    stillHerePrompt: never,
    stillHereStop: never,
    chooseMeetingSnapshotsFolder: never,
    forgetVoice: never,
    meetingSettings: never,
    meetingSnapshotSettings: never,
    resetMeetingSnapshotsFolder: never,
    setMeetingSettings: never,
    setMeetingSnapshotsEnabled: never,
    voicePeople: never,
  };
});
const { countdownClock } = await import("./StillHereView");
const { CHECK_IN_HOURS, checkInLabel } = await import("../settings/redesign/sections/meetings/MeetingsCardRedesign");

describe("Still recording?", () => {
  it("counts down as m:ss and never below 0:00", () => {
    expect(countdownClock(300)).toBe("5:00");
    expect(countdownClock(299.2)).toBe("5:00");
    expect(countdownClock(59)).toBe("0:59");
    expect(countdownClock(5)).toBe("0:05");
    expect(countdownClock(-3)).toBe("0:00");
  });

  it("offers Off and 1 to 4 hours, as on the Mac", () => {
    expect([...CHECK_IN_HOURS]).toEqual([0, 1, 2, 3, 4]);
    expect(CHECK_IN_HOURS.map(checkInLabel)).toEqual(["Off", "1 hour", "2 hours", "3 hours", "4 hours"]);
  });
});
