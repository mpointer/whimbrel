import { describe, expect, it } from "vitest";
import { importSummary } from "./importSummary";

const none = { meetings: 0, notes: 0, skipped: 0, overlapping: 0, failed: [], summaryLimit: 25 };

describe("the import line", () => {
  it("says what the import did, in the Mac's order", () => {
    expect(importSummary({ ...none, meetings: 3, skipped: 1, failed: ["a.vtt"] })).toBe(
      "Imported 3 meetings. Skipped 1 already here. Couldn't read 1 file.",
    );
    expect(importSummary({ ...none, notes: 12, overlapping: 2 })).toBe("Imported 12 notes. Skipped 2 you recorded here.");
    expect(importSummary({ ...none, meetings: 1 })).toBe("Imported 1 meeting.");
  });

  it("names the summary limit only when an import goes past it", () => {
    expect(importSummary({ ...none, meetings: 25 })).toBe("Imported 25 meetings.");
    expect(importSummary({ ...none, meetings: 226 })).toBe("Imported 226 meetings. Summarizing the newest 25.");
  });

  it("says so when there was nothing to bring in", () => {
    expect(importSummary(none)).toBe("Nothing to import.");
  });
});
