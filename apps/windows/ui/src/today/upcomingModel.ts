import { t } from "../l10n";
import type { UpcomingDay } from "../generated/types";

/** Days with meetings shown before "Show the rest of the week" (the design's 2). */
export const FIRST_DAYS = 2;

/** "Tomorrow", else "Wed, Nov 11" in the app's locale. */
export function dayHeading(day: Pick<UpcomingDay, "date" | "tomorrow">, locale: string): string {
  if (day.tomorrow) return t("Tomorrow");
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day.date);
  if (!m) return day.date;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).toLocaleDateString(locale, { weekday: "short", month: "short", day: "numeric" });
}

/** The days to draw, and whether a "Show the rest of the week" toggle is needed. */
export function visibleDays(days: UpcomingDay[], expanded: boolean): { shown: UpcomingDay[]; more: boolean } {
  const more = days.length > FIRST_DAYS;
  return { shown: expanded || !more ? days : days.slice(0, FIRST_DAYS), more };
}

/** "3 invited": the other attendees (core leaves the user out, as the Mac does); nothing at 0. */
export function invitedLine(attendees: string[]): string | null {
  return attendees.length > 0 ? t("{count} invited", { count: attendees.length }) : null;
}

const EXPANDED_KEY = "whimbrel.upcomingExpanded";

export function readExpanded(): boolean {
  try {
    return localStorage.getItem(EXPANDED_KEY) === "1";
  } catch {
    return false;
  }
}

export function writeExpanded(expanded: boolean): void {
  try {
    localStorage.setItem(EXPANDED_KEY, expanded ? "1" : "0");
  } catch {
    // Not remembered this time.
  }
}
