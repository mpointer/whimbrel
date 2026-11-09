import { describe, expect, it } from "vitest";
import { followsNewLines, liveClock } from "./liveTranscriptModel";

describe("liveClock", () => {
  it("reads minutes and seconds, and hours past an hour", () => {
    expect(liveClock(0)).toBe("0:00");
    expect(liveClock(24_550)).toBe("4:05");
    expect(liveClock(372_900)).toBe("1:02:09");
    expect(liveClock(-50)).toBe("0:00");
  });
});

describe("followsNewLines", () => {
  it("follows at the bottom and stays put when scrolled back", () => {
    expect(followsNewLines({ scrollTop: 600, clientHeight: 200, scrollHeight: 800 })).toBe(true);
    expect(followsNewLines({ scrollTop: 300, clientHeight: 200, scrollHeight: 800 })).toBe(false);
  });
});
