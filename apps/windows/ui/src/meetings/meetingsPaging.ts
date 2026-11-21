import { t } from "../l10n";

/**
 * The Meetings list loads a page at a time, as on the Mac: the newest PAGE_SIZE, then PAGE_SIZE
 * more each time the end of the list is reached. It used to stop at the newest 200 (Nov 20: 273
 * meetings, everything before Sep 17 could not be opened).
 */
export const PAGE_SIZE = 200;

export function nextLimit(current: number): number {
  return current + PAGE_SIZE;
}

/** True when the store holds meetings the list has not loaded. With no total (the count failed), only a full page means more. */
export function hasMore(loaded: number, limit: number, total: number | null): boolean {
  if (total == null) return loaded >= limit;
  return total > loaded;
}

/** The line under the list; null while it all fits on the first page, or with no total to show. */
export function footer(loaded: number, total: number | null, more: boolean): string | null {
  if (more) return total == null ? null : t("Showing {loaded} of {total} meetings", { loaded, total });
  return loaded > PAGE_SIZE ? t("Showing all {count} meetings", { count: loaded }) : null;
}
