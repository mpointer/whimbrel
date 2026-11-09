import type { TopicSortEstimate, TopicStepsView } from "../generated/types";
import { t } from "../l10n";
import { formatUsd, type SortPrompt } from "./topicsModel";

/** One-click names to add: core's suggestions, then the Profile name, minus names already listed
 *  (ignoring case); at most 4, as on the Mac. */
export function nameChips(names: string[], suggested: string[], profileName: string): string[] {
  const seen = new Set(names.map((n) => n.toLowerCase()));
  const out: string[] = [];
  for (const raw of [...suggested, profileName]) {
    const name = raw.trim();
    if (!name || seen.has(name.toLowerCase())) continue;
    seen.add(name.toLowerCase());
    out.push(name);
    if (out.length === 4) break;
  }
  return out;
}

/** The step switch's row while Start hasn't been pressed: the same flow as topic sorting. */
export function stepsPrompt(view: TopicStepsView, estimate: TopicSortEstimate | null, failed: boolean): SortPrompt {
  if (!view.enabled || view.started || !view.hasKey || view.localOnly) return { kind: "none" };
  if (failed) return { kind: "failed" };
  if (!estimate) return { kind: "loading" };
  if (estimate.indexing || estimate.sources === 0) return { kind: "sorting" };
  const text = t("About {count} items, about {usd}. Start?", { count: estimate.sources, usd: formatUsd(estimate.usd) });
  return { kind: "estimate", text, tokens: estimate.tokens };
}

/** "Finding steps needs a local model": only when the local model is the one that should run. */
export const needsLocalModel = (view: TopicStepsView): boolean => view.noModel && !view.enabled;
