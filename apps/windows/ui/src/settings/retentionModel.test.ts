import { describe, expect, it } from "vitest";
import type { RetentionPreview, RetentionSettings } from "../generated/types";
import { noticeLines, nothingToDelete } from "./retentionModel";

const settings: RetentionSettings = { captureDays: 30, meetingDays: 90, conversationDays: 30 };
const preview: RetentionPreview = { captures: 487, meetings: 1, conversations: 12, orphans: 2, orphanBytes: 34_200_000 };

describe("noticeLines", () => {
  it("names each window and what is past it", () => {
    expect(noticeLines(settings, preview)).toEqual([
      { label: "Screen text older than 30 days", count: "487 items" },
      { label: "Meetings and recordings older than 90 days", count: "1 meeting" },
      { label: "Chats older than 30 days", count: "12 chats" },
      { label: "Recordings no meeting uses any more", count: "2 files (34.2 MB)" },
    ]);
  });

  it("leaves out windows kept forever and a clean recordings folder", () => {
    const lines = noticeLines({ captureDays: null, meetingDays: 90, conversationDays: null }, { ...preview, orphans: 0 });
    expect(lines.map((l) => l.label)).toEqual(["Meetings and recordings older than 90 days"]);
  });

  it("still lists a window with nothing past it, so the user sees the rule", () => {
    const lines = noticeLines(settings, { captures: 0, meetings: 0, conversations: 0, orphans: 0, orphanBytes: 0 });
    expect(lines).toHaveLength(3);
    expect(lines[0].count).toBe("0 items");
    expect(nothingToDelete({ captures: 0, meetings: 0, conversations: 0, orphans: 0, orphanBytes: 0 })).toBe(true);
    expect(nothingToDelete(preview)).toBe(false);
  });
});
