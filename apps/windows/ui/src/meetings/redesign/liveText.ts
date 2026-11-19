// The note card's "when and who" line: "Today, 10:00 AM. Sam Okafor, Priya Shah." Pure, so the wording is tested.

import { dateTimeLabel, timeLabel } from "../../library/rows";

type Translate = (key: string, values?: Record<string, string | number>) => string;

/** "Today, 10:00 AM" for a start on the same day as `now`, else the date and time; then the attendees when there are any. */
export function startLine(startedMs: number, nowMs: number, locale: string, attendees: readonly string[], t: Translate): string {
  const same = new Date(startedMs).toDateString() === new Date(nowMs).toDateString();
  const when = same ? t("Today, {time}", { time: timeLabel(startedMs, locale) }) : dateTimeLabel(startedMs, new Date(nowMs), locale);
  return attendees.length > 0 ? `${when}. ${attendees.join(", ")}.` : `${when}.`;
}
