import { describe, expect, it } from "vitest";
import type { LibraryRow } from "../generated/types";
import {
  dateTimeLabel,
  dayLabel,
  durationLabel,
  formatOffset,
  fullDateTimeLabel,
  groupByDay,
  headline,
  rowKey,
  rowWhen,
  spanLabel,
} from "./rows";

const device = { deviceId: "k", label: "This PC", thisDevice: true };
const now = new Date(2026, 8, 10, 15, 0); // Sep 10 2026, 15:00 local

const capture = (id: number, ts: number, source: string | null = "uia"): LibraryRow => ({
  kind: "capture",
  id,
  tsUnixMs: ts,
  app: "code.exe",
  title: "main.rs - Whimbrel",
  snippet: "fn main()",
  source,
  device,
});

describe("dayLabel", () => {
  it("says Today and Yesterday by calendar day, not 24-hour windows", () => {
    expect(dayLabel(new Date(2026, 8, 10, 0, 5).getTime(), now)).toBe("Today");
    expect(dayLabel(new Date(2026, 8, 9, 23, 55).getTime(), now)).toBe("Yesterday");
    expect(dayLabel(new Date(2026, 8, 7, 12).getTime(), now)).not.toMatch(/Today|Yesterday/);
  });
});

describe("date and time labels", () => {
  const at = (d: number, h: number, m: number) => new Date(2026, 8, d, h, m).getTime(); // Sep 2026, local
  const en = "en-US";

  it("dates every row and detail, even today's", () => {
    expect(dateTimeLabel(at(10, 14, 5), now, en)).toBe("Thu, Sep 10 · 2:05 PM");
    expect(fullDateTimeLabel(at(10, 14, 5), en)).toBe("Thursday, September 10, 2026 · 2:05 PM");
    expect(dateTimeLabel(new Date(2025, 11, 31, 9, 0).getTime(), now, en)).toBe("Wed, Dec 31, 2025 · 9:00 AM");
  });

  it("shows a span's range and length, dating both ends across midnight", () => {
    expect(spanLabel(at(10, 14, 5), at(10, 14, 50), { now, locale: en })).toBe("Thu, Sep 10 · 2:05–2:50 PM (45 min)");
    expect(spanLabel(at(10, 11, 30), at(10, 12, 45), { now, locale: en })).toBe("Thu, Sep 10 · 11:30 AM–12:45 PM (1 h 15 min)");
    expect(spanLabel(at(9, 23, 50), at(10, 0, 30), { now, locale: en })).toBe("Wed, Sep 9 · 11:50 PM – Thu, Sep 10 · 12:30 AM (40 min)");
    expect(spanLabel(at(10, 14, 5), at(10, 14, 50), { full: true, now, locale: en })).toBe(
      "Thursday, September 10, 2026 · 2:05–2:50 PM (45 min)",
    );
    expect(spanLabel(at(10, 14, 5), null, { now, locale: en })).toBe("Thu, Sep 10 · 2:05 PM");
    expect(durationLabel(2 * 3_600_000)).toBe("2 h");
  });

  it("dates each kind of Library row", () => {
    expect(rowWhen(capture(1, at(10, 14, 5)), now, en)).toBe("Thu, Sep 10 · 2:05 PM");
    const meeting: LibraryRow = {
      kind: "meeting",
      id: 2,
      tsUnixMs: at(10, 14, 5),
      endUnixMs: at(10, 14, 50),
      title: "Standup",
      status: "summarized",
      trigger: "manual",
      hasSummary: true,
      device,
    };
    expect(rowWhen(meeting, now, en)).toBe("Thu, Sep 10 · 2:05–2:50 PM (45 min)");
    expect(rowWhen({ kind: "transcript", id: 3, meetingId: 2, meetingTitle: "M", startCs: 0, speaker: "A", snippet: "" }, now, en)).toBeNull();
  });
});

describe("formatOffset", () => {
  it("formats centiseconds as m:ss or h:mm:ss", () => {
    expect(formatOffset(0)).toBe("0:00");
    expect(formatOffset(6_150)).toBe("1:01");
    expect(formatOffset(372_000)).toBe("1:02:00");
  });
});

describe("headline", () => {
  it("labels each kind", () => {
    expect(headline(capture(1, 0))).toEqual({ title: "main.rs - Whimbrel", meta: "code.exe" });
    expect(headline(capture(1, 0, "ocr")).meta).toBe("code.exe · OCR");
    expect(
      headline({
        kind: "episode",
        id: 2,
        tsUnixMs: 0,
        endTsUnixMs: 1,
        app: "msedge.exe",
        title: null,
        itemCount: 1,
        status: "pending",
        appHidden: false,
        summary: null,
        hidden: true,
        device,
      }),
    ).toEqual({ title: "msedge.exe", meta: "msedge.exe · 1 capture · hidden" });
    expect(
      headline({ kind: "transcript", id: 3, meetingId: 9, meetingTitle: "Standup", startCs: 6_150, speaker: "Me", snippet: "hi" }),
    ).toEqual({ title: "Standup", meta: "Me · 1:01" });
  });
});

describe("groupByDay", () => {
  it("groups consecutive rows by day and keeps transcript hits together", () => {
    const today = new Date(2026, 8, 10, 9).getTime();
    const yesterday = new Date(2026, 8, 9, 9).getTime();
    const rows: LibraryRow[] = [
      capture(1, today),
      capture(2, today - 1000),
      capture(3, yesterday),
      { kind: "transcript", id: 4, meetingId: 1, meetingTitle: "M", startCs: 0, speaker: "A", snippet: "" },
    ];
    expect(groupByDay(rows, now).map((g) => [g.day, g.rows.map(rowKey)])).toEqual([
      ["Today", ["capture-1", "capture-2"]],
      ["Yesterday", ["capture-3"]],
      ["Transcript matches", ["transcript-4"]],
    ]);
  });
});
