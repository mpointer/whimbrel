import { describe, expect, it } from "vitest";
import type { MeetingMarkerView } from "../../generated/types";
import { markClockMs, markLabel, sortedMarks } from "./marksModel";

const mark = (id: number, atCs: number): MeetingMarkerView => ({ id, meetingId: 1, atCs, createdUnixMs: 0, label: null });

describe("markLabel", () => {
  const notes = "sam wants usage tier\nannual discount 15 or 20?\npriya: check churn";
  it("takes the line the caret is on", () => {
    expect(markLabel(notes, 25)).toBe("annual discount 15 or 20?");
    expect(markLabel(notes, 0)).toBe("sam wants usage tier");
  });
  it("takes the last line when the caret is at the end or unknown", () => {
    expect(markLabel(notes, notes.length)).toBe("priya: check churn");
    expect(markLabel(notes, null)).toBe("priya: check churn");
  });
  it("is the line just written when the caret sits on a fresh empty line", () => {
    expect(markLabel("a thought\n", 10)).toBe("a thought");
  });
  it("is null for blank notes", () => {
    expect(markLabel("", null)).toBeNull();
    expect(markLabel("   \n  ", 3)).toBeNull();
  });
  it("is cut to 200 characters", () => {
    expect(markLabel("x".repeat(300), null)).toHaveLength(200);
  });
});

describe("the list", () => {
  it("orders by time then id", () => {
    expect(sortedMarks([mark(3, 500), mark(1, 900), mark(2, 500)]).map((m) => m.id)).toEqual([2, 3, 1]);
  });
  it("points at the start plus the offset, paused time included", () => {
    expect(markClockMs(1_000_000, 85_000)).toBe(1_000_000 + 850_000);
  });
});
