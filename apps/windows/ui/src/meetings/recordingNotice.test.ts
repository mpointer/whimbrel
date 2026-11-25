import { describe, expect, it, vi } from "vitest";

vi.mock("../api", () => ({}));
const { liveNoticeShown, meetingsNoticeFor, noticeText, noticeVisible } = await import("./recordingNotice");

const on = { enabled: true, text: "" };

describe("the recording notice", () => {
  it("shows only with the setting on, for a meeting this PC records, until Done for it", () => {
    expect(noticeVisible(null, 7, null)).toBe(false);
    expect(noticeVisible({ enabled: false, text: "" }, 7, null)).toBe(false);
    expect(noticeVisible(on, null, null)).toBe(false);
    expect(noticeVisible(on, 7, null)).toBe(true);
    expect(noticeVisible(on, 7, 7)).toBe(false);
    expect(noticeVisible(on, 8, 7)).toBe(true);
  });

  it("stays on the Meetings screen whichever meeting is selected, and is never drawn twice", () => {
    // Recording meeting 7 with another meeting, or none, selected: the Meetings screen draws it.
    expect(meetingsNoticeFor(7, 3, on, null)).toBe(7);
    expect(meetingsNoticeFor(7, null, on, null)).toBe(7);
    // The recorded meeting is the selected one: its live screen draws it, so the Meetings screen does not.
    expect(meetingsNoticeFor(7, 7, on, null)).toBeNull();
    // Nothing recording, the setting off, not yet read, or Done pressed for this meeting: nothing.
    expect(meetingsNoticeFor(null, 3, on, null)).toBeNull();
    expect(meetingsNoticeFor(7, 3, { enabled: false, text: "" }, null)).toBeNull();
    expect(meetingsNoticeFor(7, 3, null, null)).toBeNull();
    expect(meetingsNoticeFor(7, 3, on, 7)).toBeNull();
    // Done for an earlier meeting does not hide it for the next one.
    expect(meetingsNoticeFor(8, 3, on, 7)).toBe(8);
  });

  it("is drawn by a live screen only for the meeting this PC is recording", () => {
    expect(liveNoticeShown(7, 7, on, null)).toBe(true);
    // A live screen open on another meeting while 7 records: not there, the Meetings screen has it.
    expect(liveNoticeShown(3, 7, on, null)).toBe(false);
    expect(meetingsNoticeFor(7, 3, on, null)).toBe(7);
    // No recording id known yet: the screen's own meeting decides, as before.
    expect(liveNoticeShown(7, null, on, null)).toBe(true);
    expect(liveNoticeShown(7, undefined, on, 7)).toBe(false);
    expect(liveNoticeShown(null, 7, on, null)).toBe(false);
  });

  it("is on the Meetings screen exactly once, whatever is selected", () => {
    for (const selected of [null, 3, 7]) {
      const onMeetings = meetingsNoticeFor(7, selected, on, null) != null ? 1 : 0;
      const onLive = selected != null && liveNoticeShown(selected, 7, on, null) ? 1 : 0;
      expect(onMeetings + onLive, `selected ${selected}`).toBe(1);
    }
  });

  it("uses the custom wording, else the default", () => {
    expect(noticeText("Recording for my notes.")).toBe("Recording for my notes.");
    expect(noticeText("  ")).toBe("I'm recording this meeting with Whimbrel, on my own computer, for my notes.");
  });
});
