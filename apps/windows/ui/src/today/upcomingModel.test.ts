import { describe, expect, it } from "vitest";
import type { UpcomingDay } from "../generated/types";
import { dayHeading, invitedLine, visibleDays } from "./upcomingModel";

const day = (date: string, tomorrow = false): UpcomingDay => ({ date, tomorrow, events: [] });

describe("dayHeading", () => {
  it("says Tomorrow for the first day, else the weekday and date", () => {
    expect(dayHeading(day("2026-11-09", true), "en-US")).toBe("Tomorrow");
    expect(dayHeading(day("2026-11-11"), "en-US")).toBe("Wed, Nov 11");
    expect(dayHeading(day("soon"), "en-US")).toBe("soon");
  });
});

describe("visibleDays", () => {
  const days = [day("2026-11-09", true), day("2026-11-10"), day("2026-11-12")];
  it("shows two days with a toggle until expanded", () => {
    expect(visibleDays(days, false)).toEqual({ shown: days.slice(0, 2), more: true });
    expect(visibleDays(days, true)).toEqual({ shown: days, more: true });
    expect(visibleDays(days.slice(0, 2), false)).toEqual({ shown: days.slice(0, 2), more: false });
    expect(visibleDays([], false)).toEqual({ shown: [], more: false });
  });
});

describe("invitedLine", () => {
  it("counts invitees", () => {
    expect(invitedLine(["a@x.test", "b@x.test", "c@x.test"])).toBe("3 invited");
    expect(invitedLine([])).toBeNull();
  });
});
