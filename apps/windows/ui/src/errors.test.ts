import { afterEach, describe, expect, it } from "vitest";
import { describeError, isCommandError } from "./errors";
import { setLanguage } from "./l10n";
import catalogText from "./l10n/Localizable.xcstrings?raw";

const catalogKeys = Object.keys((JSON.parse(catalogText) as { strings: Record<string, unknown> }).strings);

describe("describeError", () => {
  afterEach(() => setLanguage("en"));

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

  it("keeps a large amount free of separators", () => {
    expect(describeError({ kind: "spend_cap", scope: "user", spent_cents: 1487, cap_cents: 1000 })).toContain("1487¢ of 1000¢");
  });

  it("renders each sentence from the catalog, so it can be translated", () => {
    const sentences = [
      "You've reached today's spend cap ({spent}¢ of {cap}¢). It resets at midnight UTC, or raise it in Settings.",
      "Whimbrel's app-wide spend limit for today is reached ({spent}¢ of {cap}¢). Try again tomorrow.",
      "No model provider could answer. Check that Ollama is running or that a hosted provider has a key in Settings.",
    ];
    for (const key of sentences) expect(catalogKeys).toContain(key);
    expect(describeError({ kind: "spend_cap", scope: "user", spent_cents: 5, cap_cents: 9 })).toBe(
      sentences[0].replace("{spent}", "5").replace("{cap}", "9"),
    );
    expect(describeError({ kind: "provider_failure" })).toBe(sentences[2]);
  });

  it("falls back to English in a language without the translation yet", () => {
    // A language Whimbrel doesn't ship, so it has no translation (German now does).
    setLanguage("ja");
    expect(describeError({ kind: "spend_cap", scope: "global", spent_cents: 5, cap_cents: 5 })).toContain("(5¢ of 5¢)");
  });

  it("uses the message for other kinds, which the shell already rendered in the UI language", () => {
    expect(describeError({ kind: "store", message: "the store actor has stopped" })).toBe(
      "the store actor has stopped",
    );
  });
});

describe("a provider failure says why, when core knows", () => {
  it("names the missing Ollama model and how to get it", () => {
    expect(describeError({ kind: "provider_failure", reason: { kind: "ollama_model_missing", model: "gemma4:12b" } })).toBe(
      "Ollama doesn't have the model gemma4:12b. Pick one you have in Settings → AI, or run: ollama pull gemma4:12b",
    );
  });

  it("says Ollama isn't running, nothing is set up, a key was refused, or a provider is busy", () => {
    expect(describeError({ kind: "provider_failure", reason: { kind: "ollama_not_running" } })).toMatch(/^Ollama isn't running/);
    expect(describeError({ kind: "provider_failure", reason: { kind: "no_provider_configured" } })).toBe(
      "No AI provider is set up. Add one in Settings → AI.",
    );
    expect(describeError({ kind: "provider_failure", reason: { kind: "unauthorized", provider: "openai" } })).toBe(
      "OpenAI rejected the API key. Check it in Settings → AI.",
    );
    expect(describeError({ kind: "provider_failure", reason: { kind: "rate_limited", provider: "anthropic" } })).toBe(
      "Anthropic is busy or out of credit. Try again shortly.",
    );
    expect(describeError({ kind: "provider_failure", reason: { kind: "credit_exhausted" } })).toBe(
      "Whimbrel AI credit is used up. Add credit in Settings.",
    );
  });

  it("says a local model that didn't finish was too much for the computer, and what to do", () => {
    const local = /^The local model \(Ollama\) didn't finish\./;
    expect(describeError({ kind: "provider_failure", reason: { kind: "other", provider: "ollama", status: null } })).toMatch(local);
    expect(describeError({ kind: "provider_failure", reason: { kind: "network", provider: "ollama" } })).toMatch(local);
    expect(describeError({ kind: "provider_failure", reason: { kind: "other", provider: "ollama", status: 500 } })).not.toMatch(local);
  });

  it("keeps the general line for a network blip", () => {
    expect(describeError({ kind: "provider_failure", reason: { kind: "network", provider: "google" } })).toBe(
      "No model provider could answer. Check that Ollama is running or that a hosted provider has a key in Settings.",
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
