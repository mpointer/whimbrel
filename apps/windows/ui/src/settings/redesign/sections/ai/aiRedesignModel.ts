// What the redesigned AI section decides, apart from drawing it. The money rules come from aiModel.ts
// (parseCapDollars, capToDollars); this file maps them onto the number rows and words the section uses.

import { t } from "../../../../l10n";
import type { UsageRowView } from "../../../../generated/types";
import type { Provider } from "../../../../generated/types";
import { formatDollars, groupUsageByDay, parseCapDollars, providerLabel, usageCost, usageTime, usageTokens } from "../../../aiModel";

/** A daily cap is one of three things: the built-in cap (nothing stored), a dollar amount, or off (0). */
export type CapMode = "builtin" | "custom" | "off";

/** What the stored cap is. `null` or missing is the built-in cap, 0 is off, anything else is an amount. */
export function capModeOf(cents: number | null | undefined): CapMode {
  if (cents === null || cents === undefined) return "builtin";
  return cents === 0 ? "off" : "custom";
}

/** The value a mode writes at once: the built-in cap is `null`, off is 0. An amount waits for the number row. */
export function capForMode(mode: CapMode): number | null | undefined {
  if (mode === "builtin") return null;
  if (mode === "off") return 0;
  return undefined;
}

/** A number typed in dollars, as cents; `undefined` when it is not an amount of money (more than two decimals). Same rule as the old card. */
export function dollarsToCents(dollars: number): number | null | undefined {
  if (!Number.isFinite(dollars) || dollars < 0) return undefined;
  return parseCapDollars(String(dollars));
}

/** What a monthly budget stores for a typed amount: dollars, or `null` for none (0 is none). `undefined`: not an amount. */
export function budgetForAmount(dollars: number): { dollars: number | null; cents: number | null } | undefined {
  const cents = dollarsToCents(dollars);
  if (cents === undefined) return undefined;
  return { dollars: cents ? cents / 100 : null, cents: cents || null };
}

/** What turning a daily cap off means, said plainly before it is done. */
export const capOffWords = () => t("With the cap off there will be no daily limit on spending for these hosted providers.");
export const budgetOffTitle = (provider: string) => t("Remove the {provider} budget?", { provider });
export const budgetOffWords = (provider: string) => t("With no budget there will be no monthly limit on spending for {provider}.", { provider });

/** A paid provider's month for the progress row: the amount against the budget (the row's own label says "This month"), and whether it is over. */
export function monthBudget(spentCents: number, budgetCents: number | null | undefined): { value: string; fill: number | null; over: boolean; none: string } {
  const none = t("This month: {spent}", { spent: formatDollars(spentCents) });
  if (!budgetCents || budgetCents <= 0) return { value: formatDollars(spentCents), fill: null, over: false, none };
  const percent = Math.floor((spentCents * 100) / budgetCents);
  return {
    value: t("{spent} of {budget} ({percent}%)", { spent: formatDollars(spentCents), budget: formatDollars(budgetCents), percent: String(percent) }),
    fill: Math.min(1, spentCents / budgetCents),
    over: spentCents >= budgetCents,
    none,
  };
}

export const capInvalidMessage = () => t("A daily cap is a dollar amount, like 2.50, with at most two decimals.");
export const budgetInvalidMessage = () => t("A monthly budget is a dollar amount, like 25.00, with at most two decimals.");

/** What the chosen provider does with a request, said truthfully: only a model running on this PC keeps it on this PC. */
export function providerPrivacyLine(provider: Provider): string {
  if (provider === "ollama") return t("Ollama is free. A model that runs on this PC keeps everything on this PC.");
  if (provider === "whimbrel") return t("Your request goes through Whimbrel's server to Anthropic. Whimbrel keeps a record of tokens used, not what you asked.");
  return t("{provider} is a hosted service: what you ask is sent to it, not kept on this PC.", { provider: providerLabel(provider) });
}

/** The on-ramp's words. Says what the app does without a model in the redesign's names, and what the chosen provider does with a request. */
export function onRampWords(provider: Provider = "ollama") {
  return {
    title: t("Pick where AI runs."),
    body: t("Whimbrel works without any AI model: recording, saving your screen text, transcripts and search still work. Chat, summaries and routines need one."),
    note: [
      providerPrivacyLine(provider),
      t("Gemini's free tier is free, and Google may use prompts to improve its products."),
      t("Or paste any Anthropic, OpenAI, Google or OpenRouter key below."),
    ].join(" "),
  };
}

/** One call of the usage list, as a labelled item: the time and task, then what ran and what it cost. */
export function usageItems(rows: UsageRowView[], locale: string) {
  return groupUsageByDay(rows, locale).map((group) => ({
    day: group.day,
    items: group.rows.map((row, i) => ({
      key: `${row.tsUnixMs}-${row.provider}-${row.model}-${row.task}-${i}`,
      label: `${usageTime(row.tsUnixMs, locale)} ${row.task}`,
      value: [`${row.provider} ${row.model}`, usageTokens(row), usageCost(row)].filter(Boolean).join(", "),
      mono: true,
    })),
  }));
}

/** Names the redesigned copy must not show. */
export const BANNED_AI_PHRASES = [/AI & routing/i, /memory database/i, /Activity capture/i, /Capture & Privacy/i, /Never delete/i, /Keep full quality forever/i];

export const hasBannedPhrase = (text: string): boolean => BANNED_AI_PHRASES.some((p) => p.test(text));
