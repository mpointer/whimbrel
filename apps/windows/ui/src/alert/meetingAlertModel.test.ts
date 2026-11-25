import { describe, expect, it } from "vitest";
import { dismissWords } from "./meetingAlertModel";

const t = (s: string) => s;

describe("the meeting alert's way out", () => {
  it("is Not now until a countdown runs, with no tooltip", () => {
    expect(dismissWords(false, t)).toEqual({ label: "Not now", help: undefined });
  });

  it("is one control, Don’t record, while the countdown runs, and says what it does", () => {
    const words = dismissWords(true, t);
    expect(words.label).toBe("Don’t record");
    expect(words.help).toBe("Stops the countdown. Whimbrel will not ask about this meeting again.");
  });
});
