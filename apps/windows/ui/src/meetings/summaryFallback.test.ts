import { describe, expect, it } from "vitest";
import { fallbackAction, fallbackLine } from "./summaryFallback";

describe("the summary fallback notice", () => {
  it("says who wrote the summary and why, in one sentence per cause", () => {
    expect(fallbackLine({ used: "ollama", model: "gemma4:12b", reason: "not_signed_in" })).toBe(
      "Summarized on this PC with gemma4:12b because you were not signed in to Whimbrel.",
    );
    expect(fallbackLine({ used: "local", model: "qwen", reason: "credit_exhausted" })).toMatch(/on this PC.*credit or budget ran out/);
    expect(fallbackLine({ used: "anthropic", model: "m", reason: "model_unavailable" })).toBe(
      "Summarized with m because Whimbrel does not offer the chosen model.",
    );
    for (const reason of ["timeout", "network", "local_only", "rate_limited", "something_new"]) {
      expect(fallbackLine({ used: "openai", model: "m", reason })).toBe("Summarized with m because Whimbrel was not available.");
    }
  });

  it("offers sign-in only while signed out", () => {
    expect(fallbackAction(false)).toBe("signIn");
    expect(fallbackAction(true)).toBe("resummarize");
  });
});
