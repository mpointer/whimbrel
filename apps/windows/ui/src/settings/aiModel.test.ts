import { describe, expect, it } from "vitest";
import type { ProviderInfoView, RoutingSettings, UsageRowView, UsageView } from "../generated/types";
import {
  capToDollars,
  costLine,
  dataTermsLine,
  formatCents,
  groupUsageByDay,
  hostedNotice,
  keyCheckLine,
  keyLine,
  modelField,
  modelOptions,
  parseCapDollars,
  showsOnRamp,
  usageCost,
  usageTokens,
  usageTotalsLine,
} from "./aiModel";

const routing: RoutingSettings = {
  preferredProvider: "anthropic",
  ollamaModel: "gemma4:12b",
  anthropicModel: "claude-haiku-4-5",
  openaiModel: "gpt-4o-mini",
  googleModel: "gemini-2.5-flash",
  openrouterModel: "",
  localOnly: false,
  cheapFirst: true,
  useLocalModel: true,
};

const google: ProviderInfoView = {
  id: "google",
  name: "Google Gemini",
  keyPageUrl: "https://aistudio.google.com/apikey",
  policyUrl: "https://ai.google.dev/gemini-api/terms",
  keyPrefix: "AIza",
  dataTerms: { text: "On Google's free tier, prompts may be used.", checked: "2026-10-29" },
  cheapModel: "gemini-2.0-flash",
};

const usage: UsageView = { todayCents: 3, last30DaysCents: 37, monthToDateCents: 12, byProvider: [{ provider: "google", cents: 37 }] };

describe("the on-ramp", () => {
  it("says what small tasks use and the user's own figure once there is one", () => {
    expect(costLine(google, null)).toBe("Small tasks use gemini-2.0-flash. Most people spend under a dollar a month at this setting.");
    expect(costLine(google, usage)).toBe("Small tasks use gemini-2.0-flash. In the last 30 days you spent $0.37 on Google Gemini.");
    expect(costLine({ ...google, id: "openrouter", name: "OpenRouter", cheapModel: "" }, usage)).toBe("In the last 30 days you spent $0.00 on OpenRouter.");
    expect(dataTermsLine(google)).toBe("On Google's free tier, prompts may be used. (Checked 2026-10-29.)");
    expect(dataTermsLine({ ...google, dataTerms: null })).toBeNull();
  });

  it("turns a key check into its line", () => {
    expect(keyCheckLine({ status: "valid", message: "Saved. 2 models available.", models: ["a", "b"] })).toEqual({ text: "Saved. 2 models available.", isError: false });
    expect(keyCheckLine({ status: "refused", message: "no", models: [] }).isError).toBe(true);
    expect(keyCheckLine({ status: "unreachable", message: "offline", models: [] }).isError).toBe(false);
  });

  it("shows the card only with no hosted key and no local model to call", () => {
    const none = [{ provider: "anthropic" as const, masked: null }];
    expect(showsOnRamp(routing, none, false)).toBe(true);
    expect(showsOnRamp(routing, none, true)).toBe(false);
    expect(showsOnRamp({ ...routing, useLocalModel: false }, none, true)).toBe(true);
    expect(showsOnRamp(routing, [{ provider: "anthropic", masked: "••••••••1234" }], false)).toBe(false);
  });

  it("notes an OpenRouter default without a pick", () => {
    const keys = [{ provider: "openrouter" as const, masked: "••••••••1234" }];
    expect(hostedNotice({ ...routing, preferredProvider: "openrouter" }, keys)).toMatch(/no model picked/);
    expect(hostedNotice({ ...routing, preferredProvider: "openrouter", openrouterModel: "vendor/model:free" }, keys)).toBeNull();
  });
});

describe("model usage", () => {
  const row = (over: Partial<UsageRowView> = {}): UsageRowView => ({
    tsUnixMs: new Date(2026, 9, 8, 12, 0).getTime(),
    task: "ask",
    provider: "openai",
    model: "gpt-4o-mini",
    inputTokens: 100,
    outputTokens: 20,
    estimatedCostCents: 1,
    local: false,
    ...over,
  });

  it("reads totals, tokens and a local call's cost", () => {
    expect(usageTotalsLine(usage)).toBe("Today $0.03, last 30 days $0.37, this month $0.12. Local calls are not metered.");
    expect(usageTokens(row())).toBe("100 in, 20 out");
    expect(usageTokens(row({ inputTokens: null, outputTokens: null }))).toBe("");
    expect(usageCost(row())).toBe("$0.01");
    expect(usageCost(row({ local: true, estimatedCostCents: 0 }))).toBe("local");
  });

  it("groups rows by day in arrival order", () => {
    const noon = new Date(2026, 9, 8, 12, 0).getTime();
    const day = 24 * 60 * 60 * 1000;
    const groups = groupUsageByDay([row({ tsUnixMs: noon }), row({ tsUnixMs: noon - 60_000 }), row({ tsUnixMs: noon - day }), row({ tsUnixMs: noon - 2 * day })]);
    expect(groups.map((g) => g.rows.length)).toEqual([2, 1, 1]);
  });
});

describe("caps", () => {
  it("reads blank as the default, 0 as no cap, and dollars as cents", () => {
    expect(parseCapDollars("")).toBeNull();
    expect(parseCapDollars("  ")).toBeNull();
    expect(parseCapDollars("0")).toBe(0);
    expect(parseCapDollars("$2.50")).toBe(250);
    expect(parseCapDollars("10")).toBe(1000);
    expect(parseCapDollars("0.07")).toBe(7);
  });

  it("rejects what isn't an amount", () => {
    for (const bad of ["-1", "abc", "1.234", "1,5"]) expect(parseCapDollars(bad)).toBeUndefined();
  });

  it("round-trips through the text field", () => {
    expect(capToDollars(null)).toBe("");
    expect(capToDollars(undefined)).toBe("");
    expect(capToDollars(250)).toBe("2.50");
    expect(parseCapDollars(capToDollars(1999))).toBe(1999);
    expect(formatCents(1234)).toBe("$12.34");
  });
});

describe("keys", () => {
  it("shows only the masked form", () => {
    expect(keyLine({ masked: "••••••••1234" })).toBe("Saved · ••••••••1234");
    expect(keyLine({ masked: null })).toBe("Not set");
    expect(keyLine({})).toBe("Not set");
  });

  it("warns when the preferred hosted provider has no key, or local-only overrides it", () => {
    expect(hostedNotice(routing, [{ provider: "anthropic", masked: "••••••••1234" }])).toBeNull();
    expect(hostedNotice(routing, [{ provider: "anthropic", masked: null }])).toBe(
      "Anthropic has no API key, so requests fall back to Ollama.",
    );
    expect(hostedNotice({ ...routing, preferredProvider: "ollama" }, [])).toBeNull();
    expect(hostedNotice({ ...routing, localOnly: true }, [])).toMatch(/^Local-only mode is on/);
  });
});

it("keeps the saved model selectable when the provider's list lacks it", () => {
  const list = ["gemini-2.5-flash", "gemini-2.5-pro"];
  expect(modelOptions(list, "gemini-2.5-pro")).toEqual(list);
  expect(modelOptions(list, "gemini-1.5-flash")).toEqual(["gemini-1.5-flash", ...list]);
  expect(modelOptions(list, "  ")).toEqual(list);
});

it("maps each provider to its model field", () => {
  expect(modelField("openai")).toBe("openaiModel");
  expect(routing[modelField("google")]).toBe("gemini-2.5-flash");
});
