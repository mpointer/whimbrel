import { describe, expect, it, vi } from "vitest";

vi.mock("../api", () => ({}));
const { noticeText, noticeVisible } = await import("./recordingNotice");

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

  it("uses the custom wording, else the default", () => {
    expect(noticeText("Recording for my notes.")).toBe("Recording for my notes.");
    expect(noticeText("  ")).toBe("I'm recording this meeting with Whimbrel, on my own computer, for my notes.");
  });
});
