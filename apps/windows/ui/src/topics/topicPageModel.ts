import { t } from "../l10n";
import type { SummaryLeadView, TopicListItemView, TopicNextMeetingView, TopicPageView } from "../generated/types";

/** The list's groups, in the Mac's order. A topic with no kind (or an unknown one) is Other. */
export const TOPIC_GROUPS = ["customer", "product", "project", "subject", "other"] as const;
export type TopicGroup = (typeof TOPIC_GROUPS)[number];

export function groupLabel(group: TopicGroup): string {
  return { customer: t("Customer"), product: t("Product"), project: t("Project"), subject: t("Subject"), other: t("Other") }[group];
}

export function groupOf(kind: string | null | undefined): TopicGroup {
  return (TOPIC_GROUPS as readonly string[]).includes(kind ?? "") ? (kind as TopicGroup) : "other";
}

const DAY_MS = 86_400_000;

/** A local `YYYY-MM-DD` as a Date at local midnight; null if it doesn't parse. */
export function parseDay(iso: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])) : null;
}

/** Today as a local `YYYY-MM-DD`. */
export function todayIso(now: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** "Fri, Nov 14" in the app's locale. */
export const dayLabel = (d: Date, locale: string) => d.toLocaleDateString(locale, { weekday: "short", month: "short", day: "numeric" });
/** "Nov 3". */
export const monthDay = (d: Date, locale: string) => d.toLocaleDateString(locale, { month: "short", day: "numeric" });
/** "Tue, Nov 18, 10:00". */
export const dayTime = (ms: number, locale: string) => {
  const d = new Date(ms);
  return `${dayLabel(d, locale)}, ${d.toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" })}`;
};

/** A `YYYY-MM-DD` as a day, or the string itself when it doesn't parse. */
export function dayText(iso: string, locale: string): string {
  const d = parseDay(iso);
  return d ? dayLabel(d, locale) : iso;
}

/** "due Fri, Nov 14", or "overdue" once the day has passed. */
export function dueText(iso: string, locale: string, today: string = todayIso()): string {
  return iso < today ? t("overdue") : t("due {day}", { day: dayText(iso, locale) });
}

function leadText(lead: SummaryLeadView, locale: string, today: string): string {
  if (lead.kind === "date") return `${lead.label} ${dayText(lead.date, locale)}`;
  return lead.dueDate ? `${lead.text} ${dueText(lead.dueDate, locale, today)}` : lead.text;
}

/** "SOW due Fri, Nov 14 · next call Tue, Nov 18, 10:00", from stored facts only; null when there's nothing. */
export function summaryLine(page: Pick<TopicPageView, "lead" | "nextMeeting">, locale: string, today: string = todayIso()): string | null {
  const parts: string[] = [];
  if (page.lead) parts.push(leadText(page.lead, locale, today));
  if (page.nextMeeting) parts.push(t("next call {when}", { when: dayTime(page.nextMeeting.startUnixMs, locale) }));
  return parts.length ? parts.join(" · ") : null;
}

/** "3 maximus.com · 1 you": other attendees by email domain, then the user. */
export function attendeeLine(next: TopicNextMeetingView): string {
  const parts = next.attendeesByOrg.map((o) => `${o.count} ${o.org}`);
  if (next.me > 0) parts.push(t("{n} you", { n: String(next.me) }));
  return parts.join(" · ");
}

/** The list's hint: "call Tue" when the next meeting is within 7 days, "2 steps" for open steps owed. */
export function listHint(entry: TopicListItemView, locale: string, now: number = Date.now()): string | null {
  const parts: string[] = [];
  const next = entry.nextMeetingUnixMs;
  if (next != null && next - now < 7 * DAY_MS) {
    parts.push(t("call {day}", { day: new Date(next).toLocaleDateString(locale, { weekday: "short" }) }));
  }
  // Two plain strings, as on the Mac ("1 step", "2 steps").
  if (entry.openMine > 0) parts.push(entry.openMine === 1 ? t("1 step") : t("{n} steps", { n: String(entry.openMine) }));
  return parts.length ? parts.join(" · ") : null;
}

/** Recent items shown before "More" (core sends up to 20). */
export const RECENT_FIRST = 10;
/** The page reloads this often while it shows, as on the Mac. */
export const TOPICS_REFRESH_MS = 30_000;

/** Which topic to show: the wanted one while it's still listed, else the first. */
export function chooseTopic(list: TopicListItemView[], wanted: number | null): number | null {
  if (wanted != null && list.some((e) => e.topic.topicId === wanted)) return wanted;
  return list[0]?.topic.topicId ?? null;
}
