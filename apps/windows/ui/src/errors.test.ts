import { describe, expect, it } from "vitest";
import { describeError, isCommandError } from "./errors";

describe("describeError", () => {
  it("names the user cap with both amounts", () => {
    const text = describeError({ kind: "spend_cap", scope: "user", spent_cents: 205, cap_cents: 200 });
    expect(text).toContain("205¢ of 200¢");
    expect(text).toContain("Settings");
  });

  it("distinguishes the app-wide cap", () => {
    expect(describeError({ kind: "spend_cap", scope: "global", spent_cents: 5000, cap_cents: 5000 })).toContain(
      "app-wide",
    );
  });

  it("uses the message for other kinds", () => {
    expect(describeError({ kind: "store", message: "the store actor has stopped" })).toBe(
      "the store actor has stopped",
    );
  });
});

describe("isCommandError", () => {
  it("recognizes tagged errors only", () => {
    expect(isCommandError({ kind: "provider_failure" })).toBe(true);
    expect(isCommandError("plain string")).toBe(false);
    expect(isCommandError(null)).toBe(false);
  });
});
