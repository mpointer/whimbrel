import { describe, expect, it } from "vitest";
import { footer, hasMore, nextLimit, PAGE_SIZE } from "./meetingsPaging";

describe("the Meetings list's pages", () => {
  it("offers older meetings until every meeting is loaded", () => {
    expect(PAGE_SIZE).toBe(200);
    expect(nextLimit(200)).toBe(400);
    expect(hasMore(200, 200, 273)).toBe(true);
    expect(hasMore(273, 400, 273)).toBe(false);
    expect(hasMore(40, 200, 40)).toBe(false);
  });

  it("without a count, asks for more only after a full page", () => {
    expect(hasMore(200, 200, null)).toBe(true);
    expect(hasMore(120, 200, null)).toBe(false);
  });

  it("says how many it is showing", () => {
    expect(footer(200, 273, true)).toBe("Showing 200 of 273 meetings");
    expect(footer(273, 273, false)).toBe("Showing all 273 meetings");
    expect(footer(40, 40, false)).toBeNull();
    expect(footer(200, null, true)).toBeNull();
  });
});
