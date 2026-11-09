// Settings → Topics and the TypeSafe sorting row in Settings → AI (port of PR 631: the Mac's
// TopicsSettingsPane and AIProvidersSettingsPane). English only until translation is asked for.

import type { TopicSeedView, TopicSortEstimate, TopicSortView } from "../generated/types";
import { t } from "../l10n";

/** Settings → Topics and the estimate row reload this often (the Mac's 30 s). */
export const TOPICS_REFRESH_MS = 30_000;

export const TOPIC_KINDS = ["customer", "product", "project", "subject"] as const;
export type TopicKind = (typeof TOPIC_KINDS)[number];

export function kindLabel(kind: string): string {
  switch (kind) {
    case "customer":
      return t("Customer");
    case "product":
      return t("Product");
    case "project":
      return t("Project");
    case "subject":
      return t("Subject");
    default:
      return kind;
  }
}

/** "Acme, ACME Corp , " → ["Acme", "ACME Corp"]: the editor's comma-separated aliases. */
export function parseAliases(text: string): string[] {
  return text
    .split(",")
    .map((a) => a.trim())
    .filter((a) => a.length > 0);
}

/** The line under a topic's name: its kind, then its other names. */
export function seedDetail(seed: Pick<TopicSeedView, "kind" | "aliases">): string {
  return [seed.kind ? kindLabel(seed.kind) : "", seed.aliases.join(", ")].filter((p) => p.length > 0).join(" · ");
}

/** One plural key, as the Mac's "%lld items". */
export function itemCount(n: number): string {
  return t("{count} items", { count: n });
}

export function formatUsd(usd: number): string {
  return new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" }).format(usd);
}

/** What the sorting row shows under the switch while Start hasn't been pressed. */
export type SortPrompt =
  | { kind: "none" }
  | { kind: "loading" }
  | { kind: "failed" }
  | { kind: "sorting" }
  | { kind: "estimate"; text: string; tokens: number };

/** The Mac's `awaitingStart` branch: nothing unless the switch is on, Start wasn't pressed and
 * judging could run (a key, not Local-only). */
export function sortPrompt(view: TopicSortView, estimate: TopicSortEstimate | null, failed: boolean): SortPrompt {
  if (!view.enabled || view.started || !view.key || view.localOnly) return { kind: "none" };
  if (failed) return { kind: "failed" };
  if (!estimate) return { kind: "loading" };
  if (estimate.indexing || estimate.sources === 0) return { kind: "sorting" };
  const text = t("About {count} items, about {usd}. Start?", { count: estimate.sources, usd: formatUsd(estimate.usd) });
  return { kind: "estimate", text, tokens: estimate.tokens };
}
