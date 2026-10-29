import type { KeyCheckView, PromptSurface, Provider, ProviderInfoView, ProviderKey, RoutingSettings, UsageRowView, UsageView } from "../generated/types";

export const PROVIDERS: { id: Provider; label: string }[] = [
  { id: "ollama", label: "Ollama (on this PC)" },
  { id: "anthropic", label: "Anthropic" },
  { id: "openai", label: "OpenAI" },
  { id: "google", label: "Google" },
  { id: "openrouter", label: "OpenRouter" },
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

type ModelField = "ollamaModel" | "anthropicModel" | "openaiModel" | "googleModel" | "openrouterModel";

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
export function keyLine(key: Pick<ProviderKey, "masked">): string {
  return key.masked ? `Saved · ${key.masked}` : "Not set";
}

/** Why hosted providers won't be used right now, if they won't. */
export function hostedNotice(routing: RoutingSettings, keys: ProviderKey[]): string | null {
  if (routing.localOnly) return "Local-only mode is on: every request goes to Ollama on this PC.";
  if (routing.preferredProvider === "ollama") return null;
  const key = keys.find((k) => k.provider === routing.preferredProvider);
  if (!key?.masked) return `${providerLabel(routing.preferredProvider)} has no API key, so requests fall back to Ollama.`;
  if (routing.preferredProvider === "openrouter" && !routing.openrouterModel.trim()) {
    return "OpenRouter has no model picked yet, so requests fall back to Ollama. Pick one below.";
  }
  return null;
}

export const formatDollars = (cents: number) => `$${(cents / 100).toFixed(2)}`;

/**
 * The cost line under a key row: the light-task model, then the user's own figure once
 * there is one, else the one general claim doc 16 makes. The Mac's `costLine`.
 */
export function costLine(info: ProviderInfoView, usage: UsageView | null | undefined): string {
  const parts: string[] = [];
  if (info.cheapModel) parts.push(`Small tasks use ${info.cheapModel}.`);
  if (usage && usage.last30DaysCents > 0) {
    const cents = usage.byProvider.find((p) => p.provider === info.id)?.cents ?? 0;
    parts.push(`In the last 30 days you spent ${formatDollars(cents)} on ${info.name}.`);
  } else {
    parts.push("Most people spend under a dollar a month at this setting.");
  }
  return parts.join(" ");
}

/** The dated data-terms note, where the provider's free tier may use prompts. */
export const dataTermsLine = (info: ProviderInfoView): string | null =>
  info.dataTerms ? `${info.dataTerms.text} (Checked ${info.dataTerms.checked}.)` : null;

/** The one line a key check leaves under the row, and whether it is a problem to act on. */
export function keyCheckLine(check: KeyCheckView): { text: string; isError: boolean } {
  return { text: check.message, isError: check.status === "refused" || check.status === "other" };
}

/** The totals line of the Model usage section. */
export const usageTotalsLine = (usage: UsageView): string =>
  `Today ${formatDollars(usage.todayCents)}, last 30 days ${formatDollars(usage.last30DaysCents)}, this month ${formatDollars(usage.monthToDateCents)}. Local calls are not metered.`;

/** A usage row's cost cell: "local" for a local call, never a zero. */
export const usageCost = (row: Pick<UsageRowView, "local" | "estimatedCostCents">): string =>
  row.local ? "local" : formatDollars(row.estimatedCostCents);

export const usageTokens = (row: Pick<UsageRowView, "inputTokens" | "outputTokens">): string =>
  row.inputTokens == null || row.outputTokens == null ? "" : `${row.inputTokens} in, ${row.outputTokens} out`;

const DAY: Intl.DateTimeFormatOptions = { month: "short", day: "numeric" };
const CLOCK: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };
export const usageDay = (unixMs: number): string => new Date(unixMs).toLocaleDateString("en-US", DAY);
export const usageTime = (unixMs: number): string => new Date(unixMs).toLocaleTimeString("en-US", CLOCK);

/** Rows grouped by local calendar day, in arrival order (newest first). */
export function groupUsageByDay(rows: UsageRowView[]): { day: string; rows: UsageRowView[] }[] {
  const groups: { day: string; rows: UsageRowView[] }[] = [];
  for (const row of rows) {
    const day = usageDay(row.tsUnixMs);
    const last = groups[groups.length - 1];
    if (last && last.day === day) last.rows.push(row);
    else groups.push({ day, rows: [row] });
  }
  return groups;
}

/**
 * Whether the on-ramp card shows: no hosted key is saved and Ollama did not answer (or
 * local is off), so chat, summaries and routines have nothing to call.
 */
export const showsOnRamp = (routing: RoutingSettings, keys: ProviderKey[], ollamaUp: boolean): boolean =>
  !(routing.useLocalModel && ollamaUp) && !keys.some((k) => k.masked);
