import { t } from "../l10n";
import type { KeyCheckView, PromptSurface, Provider, ProviderInfoView, ProviderKey, RoutingSettings, UsageRowView, UsageView } from "../generated/types";

/** The provider picker's choices, labelled in the current language. */
export const providers = (): { id: Provider; label: string }[] => [
  { id: "ollama", label: t("Ollama (on this PC)") },
  { id: "anthropic", label: "Anthropic" },
  { id: "openai", label: "OpenAI" },
  { id: "google", label: "Google" },
  { id: "openrouter", label: "OpenRouter" },
  { id: "whimbrel", label: "Whimbrel AI" },
];

export const providerLabel = (p: Provider) => providers().find((x) => x.id === p)!.label;

/** The Mac's prompt-library surfaces, in its order. */
export const surfaces = (): { id: PromptSurface; label: string }[] => [
  { id: "ask", label: t("Ask") },
  { id: "meetingSummary", label: t("Meeting summaries") },
  { id: "dailyDigest", label: t("Daily digest") },
  { id: "weeklyDigest", label: t("Weekly digest") },
  { id: "episodeSummary", label: t("Episode summaries") },
  { id: "threadSummary", label: t("Thread summaries") },
];

type ModelField = "ollamaModel" | "anthropicModel" | "openaiModel" | "googleModel" | "openrouterModel" | "whimbrelModel";

/** The `RoutingSettings` field holding `provider`'s default model. */
export const modelField = (provider: Provider): ModelField => `${provider}Model`;

/**
 * The model picker's choices: the provider's list, with the saved model first if the
 * list doesn't have it (retired, or typed by hand), so loading a list never silently
 * changes what's saved.
 */
/** The Whimbrel AI model used until the user picks one (the Mac's `whimbrelDefault`). */
export const WHIMBREL_DEFAULT_MODEL = "claude-sonnet-5-5";

/**
 * The model a Whimbrel AI pick snaps to when `options` doesn't hold it: the default when offered,
 * else the first option (the Mac's `whimbrelFallback`). `null` when `current` is offered.
 */
export function whimbrelSnap(options: string[], current: string): string | null {
  if (options.length === 0 || options.includes(current)) return null;
  return options.includes(WHIMBREL_DEFAULT_MODEL) ? WHIMBREL_DEFAULT_MODEL : options[0];
}

export function modelOptions(list: string[], current: string): string[] {
  const c = current.trim();
  return c && !list.includes(c) ? [c, ...list] : list;
}

/**
 * A cap typed in dollars, as the settings file stores it (cents). Blank is `null`: the
 * built-in default cap. `0` means no cap (Mac semantics). `undefined`: not an amount.
 */
export function parseCapDollars(text: string): number | null | undefined {
  const v = text.trim().replace(/^\$/, "");
  if (v === "") return null;
  if (!/^\d+(\.\d{1,2})?$/.test(v)) return undefined;
  return Math.round(Number(v) * 100);
}

export function capToDollars(cents: number | null | undefined): string {
  return cents === null || cents === undefined ? "" : (cents / 100).toFixed(2);
}

export const formatCents = (cents: number) => `$${(cents / 100).toFixed(2)}`;

/** What a key row says. The shell sends only the masked form; the key never reaches the UI. */
export function keyLine(key: Pick<ProviderKey, "masked">): string {
  return key.masked ? t("Saved · {key}", { key: key.masked }) : t("Not set");
}

/** Why hosted providers won't be used right now, if they won't. `whimbrelSignedIn`: sync is ready,
 * which Whimbrel AI needs in place of a key. */
export function hostedNotice(routing: RoutingSettings, keys: ProviderKey[], whimbrelSignedIn = false): string | null {
  if (routing.localOnly) return t("Local-only mode is on: every request goes to Ollama on this PC.");
  if (routing.preferredProvider === "ollama") return null;
  if (routing.preferredProvider === "whimbrel") return whimbrelSignedIn ? null : t("Sign in to sync to use Whimbrel AI");
  const key = keys.find((k) => k.provider === routing.preferredProvider);
  if (!key?.masked) return t("{provider} has no API key, so requests fall back to Ollama.", { provider: providerLabel(routing.preferredProvider) });
  if (routing.preferredProvider === "openrouter" && !routing.openrouterModel.trim()) {
    return t("OpenRouter has no model picked yet, so requests fall back to Ollama. Pick one below.");
  }
  return null;
}

export const formatDollars = (cents: number) => `$${(cents / 100).toFixed(2)}`;

/**
 * A hosted provider said it doesn't have the model the user chose: the choice is kept, and the
 * provider's current default (from core) answers meanwhile.
 */
export const modelWarningLine = (w: { provider: Provider; model: string; using: string }): string =>
  t("{model} isn't available to your {provider} key; using {using} for now.", {
    model: w.model,
    provider: providerLabel(w.provider),
    using: w.using,
  });

/** A chosen Ollama model that isn't installed, and what Whimbrel uses meanwhile (the Mac's #581). */
export const ollamaWarningLine = (w: { model: string; fallback: string }): string =>
  t("{model} isn't installed. Whimbrel is using {fallback} until you install it or choose another model.", w);

/**
 * The cost line under a key row: the light-task model, then the user's own figure once
 * there is one, else the one general claim doc 16 makes. The Mac's `costLine`.
 */
export function costLine(info: ProviderInfoView, usage: UsageView | null | undefined): string {
  const parts: string[] = [];
  if (info.cheapModel) parts.push(t("Small tasks use {model}.", { model: info.cheapModel }));
  if (usage && usage.last30DaysCents > 0) {
    const cents = usage.byProvider.find((p) => p.provider === info.id)?.cents ?? 0;
    parts.push(t("In the last 30 days you spent {amount} on {provider}.", { amount: formatDollars(cents), provider: info.name }));
  } else {
    parts.push(t("Most people spend under a dollar a month at this setting."));
  }
  return parts.join(" ");
}

/**
 * A paid provider's month under its key row: "This month: $12.40 of $25.00 (50%)", or with no
 * budget "This month: $12.40". `fill` is the bar's share (0-1, capped), `over` turns it red at 100%.
 */
export function thisMonth(spentCents: number, budgetCents: number | null | undefined): { text: string; fill: number | null; over: boolean } {
  const spent = formatDollars(spentCents);
  if (!budgetCents || budgetCents <= 0) return { text: t("This month: {spent}", { spent }), fill: null, over: false };
  const percent = Math.floor((spentCents * 100) / budgetCents);
  return {
    text: t("This month: {spent} of {budget} ({percent}%)", { spent, budget: formatDollars(budgetCents), percent: String(percent) }),
    fill: Math.min(1, spentCents / budgetCents),
    over: spentCents >= budgetCents,
  };
}

/** This month's spend for `provider`, other computers included. */
export const monthSpent = (usage: UsageView | null | undefined, provider: Provider): number =>
  usage?.monthByProvider.find((p) => p.provider === provider)?.cents ?? 0;

/** The dated data-terms note, where the provider's free tier may use prompts. */
export const dataTermsLine = (info: ProviderInfoView): string | null =>
  info.dataTerms ? t("{terms} (Checked {date}.)", { terms: info.dataTerms.text, date: info.dataTerms.checked }) : null;

/** The one line a key check leaves under the row, and whether it is a problem to act on. */
export function keyCheckLine(check: KeyCheckView): { text: string; isError: boolean } {
  return { text: check.message, isError: check.status === "refused" || check.status === "other" };
}

/** The totals line of the Model usage section. */
export const usageTotalsLine = (usage: UsageView): string =>
  t("Today {today}, last 30 days {last30}, this month {month}. Local calls are not metered.", {
    today: formatDollars(usage.todayCents),
    last30: formatDollars(usage.last30DaysCents),
    month: formatDollars(usage.monthToDateCents),
  });

/** A usage row's cost cell: "local" for a local call, never a zero. */
export const usageCost = (row: Pick<UsageRowView, "local" | "estimatedCostCents">): string =>
  row.local ? t("local") : formatDollars(row.estimatedCostCents);

export const usageTokens = (row: Pick<UsageRowView, "inputTokens" | "outputTokens">): string =>
  row.inputTokens == null || row.outputTokens == null ? "" : t("{input} in, {output} out", { input: row.inputTokens, output: row.outputTokens });

const DAY: Intl.DateTimeFormatOptions = { month: "short", day: "numeric" };
const CLOCK: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };
export const usageDay = (unixMs: number, locale: string): string => new Date(unixMs).toLocaleDateString(locale, DAY);
export const usageTime = (unixMs: number, locale: string): string => new Date(unixMs).toLocaleTimeString(locale, CLOCK);

/** Rows grouped by local calendar day, in arrival order (newest first). */
export function groupUsageByDay(rows: UsageRowView[], locale: string): { day: string; rows: UsageRowView[] }[] {
  const groups: { day: string; rows: UsageRowView[] }[] = [];
  for (const row of rows) {
    const day = usageDay(row.tsUnixMs, locale);
    const last = groups[groups.length - 1];
    if (last && last.day === day) last.rows.push(row);
    else groups.push({ day, rows: [row] });
  }
  return groups;
}

/**
 * Whether a hosted provider can answer: a key is saved, or Whimbrel AI is the preferred provider and
 * sync is signed in (the Mac's `hasAnyHostedKey`).
 */
export const hasHostedProvider = (routing: RoutingSettings, keys: ProviderKey[], whimbrelSignedIn = false): boolean =>
  keys.some((k) => k.masked) || (routing.preferredProvider === "whimbrel" && whimbrelSignedIn);

/**
 * Whether the on-ramp card shows: no hosted provider can answer and Ollama did not answer (or
 * local is off), so chat, summaries and routines have nothing to call.
 */
export const showsOnRamp = (
  routing: RoutingSettings,
  keys: ProviderKey[],
  ollamaUp: boolean,
  localModelInstalled = false,
  whimbrelSignedIn = false,
): boolean => !(routing.useLocalModel && ollamaUp) && !hasHostedProvider(routing, keys, whimbrelSignedIn) && !localModelInstalled;

/**
 * The on-ramp once it can be decided: null while the Ollama probe or the local model's status is
 * still out. Deciding early drew the on-ramp for a moment on a PC with a local model, then the model
 * picker jumped into its place (2026-11-08, Joe's 0.9.61).
 */
export const onRampOnceKnown = (
  routing: RoutingSettings,
  keys: ProviderKey[],
  ollamaUp: boolean | null,
  localModelInstalled: boolean | null,
  whimbrelSignedIn = false,
): boolean | null => {
  // A hosted provider decides it at once: no probe can bring the on-ramp back.
  if (hasHostedProvider(routing, keys, whimbrelSignedIn)) return false;
  if (ollamaUp === null || localModelInstalled === null) return null;
  return showsOnRamp(routing, keys, ollamaUp, localModelInstalled, whimbrelSignedIn);
};

/** "$7.42" from micro-dollars (1,000,000 = $1), rounded down to the cent (the Mac's `dollars(micros:)`). */
export const formatMicros = (micros: number): string => formatDollars(Math.floor(Math.max(0, micros) / 10_000));

/** The Whimbrel AI card's balance line, once a read has succeeded. */
export const whimbrelBalanceLine = (micros: number): string => t("{amount} left", { amount: formatMicros(micros) });
