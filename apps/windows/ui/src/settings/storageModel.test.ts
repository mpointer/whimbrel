import { describe, expect, it } from "vitest";
import type { FreeUpOptions, StorageView } from "../generated/types";
import { compactLine, footprintLines, freedLine, freeUpRows, lowDiskLines } from "./storageModel";

const GB = 1_000_000_000;

describe("storage model", () => {
  it("says the total and each part", () => {
    const view: StorageView = {
      footprint: { database: 2 * GB, recordings: GB, models: 700_000_000, other: 40_000_000, total: 3_740_000_000 },
      dbFreeBytes: 0,
      lowDisks: [],
      days: [30, 90, 180],
      defaultDays: 90,
    };
    const [total, parts] = footprintLines(view);
    expect(total).toContain("on this PC");
    expect(parts).toContain("Database");
    expect(parts).toContain("Speech models");
  });

  it("warns once per low drive", () => {
    const lines = lowDiskLines([{ volume: "C:\\", freeBytes: 2 * GB }]);
    expect(lines).toHaveLength(1);
    expect(lines[0]).toContain("C:\\");
  });

  it("says what a compaction did", () => {
    expect(compactLine({ ran: true, reason: "ran", needBytes: null, fileBytesBefore: 3 * GB, fileBytesAfter: 2 * GB, spacePending: false })).toContain("Compacted:");
    expect(compactLine({ ran: true, reason: "ran", needBytes: null, fileBytesBefore: 3 * GB, fileBytesAfter: 3 * GB, spacePending: true })).toContain("returns to the disk shortly");
    expect(compactLine({ ran: false, reason: "not_worth_it", needBytes: null, fileBytesBefore: GB, fileBytesAfter: GB, spacePending: false })).toBe("Nothing to compact yet.");
    expect(compactLine({ ran: false, reason: "not_enough_disk", needBytes: 3 * GB, fileBytesBefore: 3 * GB, fileBytesAfter: 3 * GB, spacePending: false })).toContain("Not enough free disk");
  });

  it("hides what frees nothing and asks before deleting", () => {
    const none: FreeUpOptions = { days: 90, compactBytes: null, recordingsBytes: null, capturesCount: null, capturesBytes: null };
    expect(freeUpRows(none)).toEqual([]);
    const all: FreeUpOptions = { days: 30, compactBytes: 300_000_000, recordingsBytes: GB, capturesCount: 1200, capturesBytes: 50_000_000 };
    const rows = freeUpRows(all);
    expect(rows.map((r) => r.kind)).toEqual(["compact", "recordings", "captures"]);
    expect(rows[0].confirm).toBeNull();
    expect(rows[1].detail).toContain("30 days");
    expect(rows[1].confirm).toContain("30 days");
    expect(rows[2].detail).toMatch(/1,?200 screen captures/);
    expect(rows[2].confirm).toContain("other devices");
  });

  it("says what was freed only when something was", () => {
    expect(freedLine(5 * GB, 4 * GB)).toContain("Freed");
    expect(freedLine(4 * GB, 4 * GB)).toBeNull();
  });
});
