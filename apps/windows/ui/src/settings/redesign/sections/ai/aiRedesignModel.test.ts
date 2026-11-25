import { describe, expect, it } from "vitest";
import type { RoutingSettings, UsageRowView } from "../../../../generated/types";
import { capToDollars, hostedNotice, keyCheckLine, parseCapDollars, thisMonth, usageCost, usageTokens } from "../../../aiModel";
import { budgetForAmount, capForMode, capModeOf, dollarsToCents, hasBannedPhrase, onRampWords, usageItems } from "./aiRedesignModel";

describe("a daily cap, in modes", () => {
  it("reads nothing stored as the built-in cap, 0 as off and any amount as an amount", () => {
    expect(capModeOf(null)).toBe("builtin");
    expect(capModeOf(undefined)).toBe("builtin");
    expect(capModeOf(0)).toBe("off");
    expect(capModeOf(250)).toBe("custom");
  });
  it("writes the built-in cap as null and off as 0; an amount waits for the number", () => {
    expect(capForMode("builtin")).toBeNull();
    expect(capForMode("off")).toBe(0);
    expect(capForMode("custom")).toBeUndefined();
  });
  it("takes dollars with at most two decimals, the old rule", () => {
    expect(dollarsToCents(2.5)).toBe(250);
    expect(dollarsToCents(0)).toBe(0);
    expect(dollarsToCents(12)).toBe(1200);
    expect(dollarsToCents(1.234)).toBeUndefined();
    expect(dollarsToCents(1e-7)).toBeUndefined();
    expect(dollarsToCents(-1)).toBeUndefined();
    expect(dollarsToCents(Number.NaN)).toBeUndefined();
    // The same answers the old card's parser gives for the same text.
    for (const text of ["2.50", "0", "12", "0.07", "3.456", "1e2"]) {
      const n = Number(text);
      expect(dollarsToCents(n)).toBe(parseCapDollars(String(n)));
    }
  });
});

describe("a monthly budget", () => {
  it("stores dollars, and 0 means no budget", () => {
    expect(budgetForAmount(25)).toEqual({ dollars: 25, cents: 2500 });
    expect(budgetForAmount(12.5)).toEqual({ dollars: 12.5, cents: 1250 });
    expect(budgetForAmount(0)).toEqual({ dollars: null, cents: null });
    expect(budgetForAmount(3.333)).toBeUndefined();
  });
  it("shows the saved budget as dollars, as the old card did", () => {
    expect(capToDollars(1250)).toBe("12.50");
    expect(thisMonth(250, 1000).text).toBe("This month: $2.50 of $10.00 (25%)");
    expect(thisMonth(250, null).fill).toBeNull();
  });
});

describe("the old lines this section still uses", () => {
  const routing = { localOnly: true, preferredProvider: "openai", openrouterModel: "" } as RoutingSettings;
  it("says why hosted providers will not be used", () => {
    expect(hostedNotice(routing, [])).toBe("Local-only mode is on: every request goes to Ollama on this PC.");
    expect(hostedNotice({ ...routing, localOnly: false }, [{ provider: "openai", masked: null, syncEnabled: false }])).toBe("OpenAI has no API key, so requests fall back to Ollama.");
  });
  it("reports a refused key as a problem and a good one as plain", () => {
    expect(keyCheckLine({ message: "Key works", models: [], status: "valid" }).isError).toBe(false);
    expect(keyCheckLine({ message: "Refused", models: [], status: "refused" }).isError).toBe(true);
  });
  it("shows a local call as local, never a zero", () => {
    expect(usageCost({ local: true, estimatedCostCents: 0 })).toBe("local");
    expect(usageTokens({ inputTokens: null, outputTokens: null })).toBe("");
  });
});

describe("usage items", () => {
  const row = (tsUnixMs: number, over: Partial<UsageRowView> = {}): UsageRowView =>
    ({ tsUnixMs, provider: "openai", model: "m1", task: "chat", local: false, estimatedCostCents: 12, inputTokens: 10, outputTokens: 5, ...over }) as UsageRowView;
  it("groups by day and gives every call a unique key, a label and one line of what ran", () => {
    const day = Date.UTC(2026, 10, 20, 12);
    const groups = usageItems([row(day), row(day), row(day - 3 * 86_400_000, { local: true })], "en-US");
    expect(groups).toHaveLength(2);
    expect(new Set(groups[0].items.map((i) => i.key)).size).toBe(2);
    expect(groups[0].items[0].value).toBe("openai m1, 10 in, 5 out, $0.12");
    expect(groups[1].items[0].value).toContain("local");
  });
});

describe("redesigned wording", () => {
  it("has no banned name in the on-ramp", () => {
    const w = onRampWords();
    for (const line of [w.title, w.body, w.note]) expect(hasBannedPhrase(line)).toBe(false);
  });
  it("says recording still works without a model (the old line's capture, in the new words)", () => {
    expect(onRampWords().body).toBe(
      "Whimbrel works without any AI model: recording, saving your screen text, transcripts and search still work. Chat, summaries and routines need one.",
    );
  });
  it("says nothing leaves this PC only for a model that runs here; a hosted provider is said to receive the request", () => {
    expect(onRampWords("ollama").note).toMatch(/^Ollama is free\. A model that runs on this PC keeps everything on this PC\./);
    for (const hosted of ["anthropic", "openai", "google", "openrouter"] as const) {
      const note = onRampWords(hosted).note;
      expect(note, hosted).not.toContain("nothing leaves");
      expect(note, hosted).not.toContain("keeps everything on this PC");
      expect(note, hosted).toContain("is a hosted service: what you ask is sent to it, not kept on this PC.");
    }
    const whimbrel = onRampWords("whimbrel").note;
    expect(whimbrel).toContain("goes through Whimbrel's server to Anthropic");
    expect(whimbrel).not.toContain("nothing leaves");
  });
  it("catches the banned names", () => {
    for (const bad of ["AI & routing", "your memory database", "Activity capture", "Capture & Privacy", "Never delete", "Keep full quality forever"]) expect(hasBannedPhrase(bad)).toBe(true);
  });
});
