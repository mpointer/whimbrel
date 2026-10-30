import { describe, expect, it } from "vitest";
import { isBusy, updateLine } from "./updateModel";

describe("Check for Updates", () => {
  it("says what the Mac says at each step", () => {
    expect(updateLine({ state: "idle" })).toBeNull();
    expect(updateLine({ state: "checking" })).toBe("Checking…");
    expect(updateLine({ state: "upToDate", version: "0.9.38" })).toBe("Whimbrel is up to date (v0.9.38).");
    expect(updateLine({ state: "downloading", version: "0.9.39" })).toBe("Downloading v0.9.39…");
    expect(updateLine({ state: "ready", version: "0.9.39" })).toBe("v0.9.39 is ready.");
    expect(updateLine({ state: "failed" })).toBe("Couldn't check for updates. Try again later.");
  });

  it("waits while checking or downloading", () => {
    expect(isBusy({ state: "checking" })).toBe(true);
    expect(isBusy({ state: "downloading", version: "1" })).toBe(true);
    expect(isBusy({ state: "ready", version: "1" })).toBe(false);
    expect(isBusy({ state: "failed" })).toBe(false);
  });
});
