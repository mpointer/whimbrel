import { describe, expect, it } from "vitest";
import type { CaptureStatus } from "../generated/types";
import { captureSummary } from "./captureSummary";

const base: CaptureStatus = { enabled: true, running: true, stored: 0, unchanged: 0, excluded: 0, ocrRuns: 0 };
const now = new Date(1_757_520_000_000);

describe("captureSummary", () => {
  it("reports off, paused and not-running states first", () => {
    expect(captureSummary({ ...base, enabled: false }, now)).toBe("Off");
    expect(captureSummary({ ...base, paused: "the screen is locked" }, now)).toBe("Paused: the screen is locked");
    expect(captureSummary({ ...base, running: false, lastError: "UI Automation: failed" }, now)).toBe(
      "Not running: UI Automation: failed",
    );
  });

  it("says when and from where the last capture was stored", () => {
    expect(captureSummary(base, now)).toBe("Capturing; nothing stored yet");
    expect(captureSummary({ ...base, lastStoredAtMs: now.getTime() - 42_000, lastApp: "msedge.exe" }, now)).toBe(
      "Capturing · last from msedge.exe 42s ago",
    );
    expect(captureSummary({ ...base, lastStoredAtMs: now.getTime() - 5 * 60_000, lastApp: "code.exe" }, now)).toBe(
      "Capturing · last from code.exe 5 min ago",
    );
  });
});
