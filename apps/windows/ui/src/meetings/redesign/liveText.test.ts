import { describe, expect, it } from "vitest";
import { startLine } from "./liveText";

const t = (key: string, values?: Record<string, string | number>) => key.replace(/\{(\w+)\}/g, (_, k) => String(values?.[k]));
const at = (h: number, m = 0, day = 18) => new Date(2026, 10, day, h, m).getTime();

describe("startLine", () => {
  it("says Today with the time and lists the attendees", () => {
    expect(startLine(at(10), at(10, 14), "en-US", ["Sam Okafor", "Priya Shah"], t)).toBe("Today, 10:00 AM. Sam Okafor, Priya Shah.");
  });
  it("is only the time when nobody is listed", () => {
    expect(startLine(at(10), at(10, 14), "en-US", [], t)).toBe("Today, 10:00 AM.");
  });
  it("names the date for a meeting that began on another day", () => {
    expect(startLine(at(23, 30, 17), at(0, 20), "en-US", [], t)).toContain("Nov 17");
  });
});
