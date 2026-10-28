import type { PromptSurface, Provider, ProviderKey, RoutingSettings } from "../generated/types";

export const PROVIDERS: { id: Provider; label: string }[] = [
  { id: "ollama", label: "Ollama (on this PC)" },
  { id: "anthropic", label: "Anthropic" },
  { id: "openai", label: "OpenAI" },
  { id: "google", label: "Google" },
];

export const providerLabel = (p: Provider) => PROVIDERS.find((x) => x.id === p)!.label;

/** The Mac's prompt-library surfaces, in its order. */
export const SURFACES: { id: PromptSurface; label: string }[] = [
  { id: "ask", label: "Ask" },
  { id: "meetingSummary", label: "Meeting summaries" },
  { id: "dailyDigest", label: "Daily digest" },
  { id: "weeklyDigest", label: "Weekly digest" },
  { id: "episodeSummary", label: "Episode summaries" },
  { id: "threadSummary", label: "Thread summaries" },
];

type ModelField = "ollamaModel" | "anthropicModel" | "openaiModel" | "googleModel";

/** The `RoutingSettings` field holding `provider`'s default model. */
export const modelField = (provider: Provider): ModelField => `${provider}Model`;

/**
 * The model picker's choices: the provider's list, with the saved model first if the
 * list doesn't have it (retired, or typed by hand), so loading a list never silently
 * changes what's saved.
 */
export function modelOptions(list: string[], current: string): string[] {
  const c = current.trim();
  return c && !list.includes(c) ? [c, ...list] : list;
}

/**
 * A cap typed in dollars, as the settings file stores it (cents). Blank is `null`: the
 * built-in default cap. `0` means no cap (Mac semantics). `undefined`: not an amount.
 */
export function parseCapDollars(text: string): number | null | undefined {
  const t = text.trim().replace(/^\$/, "");
  if (t === "") return null;
  if (!/^\d+(\.\d{1,2})?$/.test(t)) return undefined;
  return Math.round(Number(t) * 100);
}

export function capToDollars(cents: number | null | undefined): string {
  return cents === null || cents === undefined ? "" : (cents / 100).toFixed(2);
}

export const formatCents = (cents: number) => `$${(cents / 100).toFixed(2)}`;

/** What a key row says. The shell sends only the masked form; the key never reaches the UI. */
export function keyLine(key: ProviderKey): string {
  return key.masked ? `Saved · ${key.masked}` : "Not set";
}

/** Why hosted providers won't be used right now, if they won't. */
export function hostedNotice(routing: RoutingSettings, keys: ProviderKey[]): string | null {
  if (routing.localOnly) return "Local-only mode is on: every request goes to Ollama on this PC.";
  if (routing.preferredProvider === "ollama") return null;
  const key = keys.find((k) => k.provider === routing.preferredProvider);
  return key?.masked ? null : `${providerLabel(routing.preferredProvider)} has no API key, so requests fall back to Ollama.`;
}
