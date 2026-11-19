import { describe, expect, it } from "vitest";
import { ASK_WAIT_MS, askOutcome } from "./askHandoff";

describe("askOutcome", () => {
  it("waits, then keeps the text when nobody took the question in time", () => {
    expect(askOutcome(0, false)).toBe("waiting");
    expect(askOutcome(ASK_WAIT_MS - 1, false)).toBe("waiting");
    expect(askOutcome(ASK_WAIT_MS, false)).toBe("kept");
  });
  it("is delivered as soon as the main window acknowledges, early or at the limit", () => {
    expect(askOutcome(100, true)).toBe("delivered");
    expect(askOutcome(ASK_WAIT_MS + 1, true)).toBe("delivered");
  });
});
