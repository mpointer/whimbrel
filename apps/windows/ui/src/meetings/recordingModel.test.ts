import { describe, expect, it } from "vitest";
import type { MeetingRecordingView } from "../generated/types";
import { elapsedLabel, recordAction } from "./recordingModel";

const view = (state: MeetingRecordingView["state"]): MeetingRecordingView => ({ state });


describe("elapsedLabel", () => {
  it("counts minutes and seconds, then hours", () => {
    expect(elapsedLabel(0, 7_000)).toBe("0:07");
    expect(elapsedLabel(0, 724_000)).toBe("12:04");
    expect(elapsedLabel(0, 3_729_000)).toBe("1:02:09");
  });

  it("never goes negative on a clock skew", () => {
    expect(elapsedLabel(10_000, 0)).toBe("0:00");
  });
});

describe("recordAction", () => {
  it("starts when idle, stops while recording, and waits otherwise", () => {
    expect(recordAction(view("idle"))).toBe("start");
    expect(recordAction(view("recording"))).toBe("stop");
    expect(recordAction(view("starting"))).toBe("wait");
    expect(recordAction(null)).toBe("wait");
  });
});
