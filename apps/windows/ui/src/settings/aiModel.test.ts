import { describe, expect, it } from "vitest";
import type { RoutingSettings } from "../generated/types";
import { capToDollars, formatCents, hostedNotice, keyLine, modelField, modelOptions, parseCapDollars } from "./aiModel";

const routing: RoutingSettings = {
  preferredProvider: "anthropic",
  ollamaModel: "gemma4:12b",
  anthropicModel: "claude-haiku-4-5",
  openaiModel: "gpt-4o-mini",
  googleModel: "gemini-2.5-flash",
  localOnly: false,
};

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
    expect(keyLine({ provider: "anthropic", masked: "••••••••1234" })).toBe("Saved · ••••••••1234");
    expect(keyLine({ provider: "openai", masked: null })).toBe("Not set");
    expect(keyLine({ provider: "google" })).toBe("Not set");
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
