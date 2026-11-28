import { describe, expect, it } from "vitest";
import type { MeetingRecordingView } from "../generated/types";
import { elapsedLabel, pausedMeetingId, recordAction, recordingCardShown, recordingLine } from "./recordingModel";

const view = (state: MeetingRecordingView["state"]): MeetingRecordingView => ({ state, paused: false, pausedTotalMs: 0, resuming: false });


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

describe("recordingCardShown", () => {
  const paused: MeetingRecordingView = { ...view("recording"), startedUnixMs: 1_000, paused: true, pausedSinceUnixMs: 5_000 };
  const processing: MeetingRecordingView = { ...view("idle"), processing: { meetingId: 1, label: "Transcribing" } as MeetingRecordingView["processing"] };

  // The real-PC test: the old card ("Recording · 0:45", a status) sat above the new live screen, so a screen reader heard two
  // recording timers, and it still said Recording while the new bar said Paused.
  it("gives way while a meeting starts, records, pauses or is processed: the bar and the live screen own that", () => {
    for (const v of [view("starting"), view("recording"), paused, processing]) expect(recordingCardShown(v)).toBe(false);
  });

  it("still offers Record now when nothing records", () => {
    expect(recordingCardShown(view("idle"))).toBe(true);
  });

  it("waits for the first status, so it never flashes over a recording", () => {
    expect(recordingCardShown(null)).toBe(false);
  });
});

// The recording notice stays on the Meetings screen, drawn once: the live screen draws it for the meeting being recorded, and
// the Meetings screen draws it above the list when another meeting, or none, is selected (`meetingsNoticeFor`). The Record
// now card never draws it.
describe("the recording notice is drawn once on the Meetings screen", () => {
  it("draws the Meetings screen's own notice from `meetingsNoticeFor`, and the Record now card draws none", async () => {
    const meetingsView = (await import("./MeetingsView.tsx?raw")).default;
    expect(meetingsView).toMatch(/meetingsNoticeFor\(recordingId, selected, noticeState\.settings, noticeState\.dismissed\)/);
    expect(meetingsView).toMatch(/<WConsentBanner/);
    const card = (await import("./RecordingCard.tsx?raw")).default;
    expect(card).not.toMatch(/RecordingNoticeCard/);
  });

  it("draws the notice from the open meeting only, in the one place the live meeting has for it", async () => {
    const workspace = (await import("./Workspace.tsx?raw")).default;
    expect(workspace.match(/<RecordingNoticeCard/g)).toHaveLength(1);
  });
});

describe("a paused recording reads Paused, not Recording", () => {
  const rec = (over: Partial<MeetingRecordingView> = {}): MeetingRecordingView => ({ ...view("recording"), meetingId: 6, startedUnixMs: 0, ...over });
  it("names the paused meeting, and only while it is paused", () => {
    expect(pausedMeetingId(null)).toBeNull();
    expect(pausedMeetingId(view("idle"))).toBeNull();
    expect(pausedMeetingId(rec())).toBeNull();
    expect(pausedMeetingId(rec({ paused: true }))).toBe(6);
    expect(pausedMeetingId(rec({ paused: true, meetingId: null }))).toBeNull();
    // A status that is not a recording any more is never paused, whatever a stale flag says.
    expect(pausedMeetingId({ ...view("idle"), paused: true, meetingId: 6 })).toBeNull();
  });
  it("the card and the workspace line say Recording paused while paused, Recording with the time otherwise", () => {
    expect(recordingLine(false, 0, 7_000)).toBe("Recording · 0:07");
    expect(recordingLine(true, 0, 7_000)).toBe("Recording paused");
  });
});
