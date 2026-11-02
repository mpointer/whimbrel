// What an import did, as one line above the Meetings list (the Mac's `ImportSummary`, W-1.6).

import type { ImportView } from "../generated/types";
import { t } from "../l10n";

export function importSummary(view: ImportView): string {
  const parts: string[] = [];
  if (view.meetings > 0) parts.push(t("Imported {count} meetings.", { count: view.meetings }));
  if (view.notes > 0) parts.push(t("Imported {count} notes.", { count: view.notes }));
  if (view.skipped > 0) parts.push(t("Skipped {count} already here.", { count: view.skipped }));
  if (view.overlapping > 0) parts.push(t("Skipped {count} you recorded here.", { count: view.overlapping }));
  if (view.failed.length > 0) parts.push(t("Couldn't read {count} files.", { count: view.failed.length }));
  if (view.meetings > view.summaryLimit) parts.push(t("Summarizing the newest {count}.", { count: view.summaryLimit }));
  return parts.length === 0 ? t("Nothing to import.") : parts.join(" ");
}
