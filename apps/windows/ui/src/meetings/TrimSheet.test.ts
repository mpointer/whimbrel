import { describe, expect, it } from "vitest";
import { hmm, keepMs, removesLine } from "./TrimSheet";

const MIN = 60_000;

describe("the trim sheet", () => {
  it("reads hours and minutes as a keep time", () => {
    expect(keepMs("1", "05")).toBe(65 * MIN);
    expect(keepMs("", "30")).toBe(30 * MIN);
    expect(keepMs("0", "60")).toBeNull();
    expect(keepMs("a", "1")).toBeNull();
    expect(keepMs("1.5", "0")).toBeNull();
  });

  it("says what goes, and nothing when nothing would", () => {
    expect(removesLine(120 * MIN, 65 * MIN)).toBe("Removes 0:55 after 1:05");
    expect(removesLine(38 * 60 * MIN, 30 * MIN)).toBe("Removes 37:30 after 0:30");
    expect(removesLine(120 * MIN, 120 * MIN)).toBeNull();
    expect(removesLine(120 * MIN, 0)).toBeNull();
    expect(removesLine(120 * MIN, null)).toBeNull();
  });

  it("writes h:mm", () => {
    expect(hmm(5 * MIN + 59_000)).toBe("0:05");
    expect(hmm(26 * 60 * MIN)).toBe("26:00");
  });
});
