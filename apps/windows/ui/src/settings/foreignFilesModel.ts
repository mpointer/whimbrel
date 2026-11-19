import type { ForeignFilesDeleted, ForeignFilesView } from "../generated/types";
import { t } from "../l10n";
import { diskSize } from "./statusModel";

/** Sizes as the card states them: under 1 MB in KB (a lone sidecar would read "0.0 MB" through
 *  diskSize). Decimal units like diskSize and the rest of the app, so 32,768 bytes is 33 KB. */
export function keptSize(bytes: number): string {
  if (bytes <= 0) return "0 KB";
  return bytes < 999_500 ? `${Math.max(1, Math.round(bytes / 1000))} KB` : diskSize(bytes);
}

/** The card shows only while some files are kept aside (Mike, 2026-11-16: never automatic, never an empty row). */
export const showCard = (v: ForeignFilesView | null): boolean => v != null && v.count > 0;

/** "3 files kept aside, 12.0 MB" (the Mac's row). */
export function summaryLine(v: ForeignFilesView): string {
  return v.count === 1
    ? t("1 file kept aside, {size}", { size: keptSize(v.bytes) })
    : t("{count} files kept aside, {size}", { count: v.count, size: keptSize(v.bytes) });
}

/** The confirmation's body: the count and size it deletes, and that it is final. */
export function confirmLine(v: ForeignFilesView): string {
  return v.count === 1
    ? t("This deletes 1 file kept aside ({size}). It cannot be recovered.", { size: keptSize(v.bytes) })
    : t("This deletes {count} files kept aside ({size}). They cannot be recovered.", { count: v.count, size: keptSize(v.bytes) });
}

/** After a delete: what went (said as a status) and what stayed (said as an alert). */
export function resultLines(d: ForeignFilesDeleted): { done: string[]; problems: string[] } {
  const done =
    d.removed === 0
      ? []
      : [
          d.removed === 1
            ? t("Deleted 1 file kept aside ({size}).", { size: keptSize(d.removedBytes) })
            : t("Deleted {count} files kept aside ({size}).", { count: d.removed, size: keptSize(d.removedBytes) }),
        ];
  return { done, problems: d.failed.length === 0 ? [] : [t("These files could not be deleted:"), d.failed.join(", ")] };
}
