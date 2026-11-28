import { describe, expect, it } from "vitest";
import {
  dateTimeLabel,
  dayLabel,
  durationLabel,
  fullDateTimeLabel,
  spanLabel,
} from "./rows";

const now = new Date(2026, 8, 10, 15, 0); // Sep 10 2026, 15:00 local


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

});



