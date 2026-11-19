import { describe, expect, it } from "vitest";
import { setThreadParts, threadAnchorId } from "./outline";

describe("thread parts", () => {
  it("names the anchor each part scrolls to", () => expect(threadAnchorId("lastWeek")).toBe("th-part-lastWeek"));
  it("accepts a list and an empty list without a DOM", () => {
    setThreadParts([{ id: "today", label: "Today" }]);
    setThreadParts([]);
    expect(true).toBe(true);
  });
});
