// What the redesigned Home shows, decided from data Today already loads plus the day's memory blocks.
// Pure: nothing here touches the DOM or the shell. (docs/ui-redesign/screens/home.md)

import type { DayBlock } from "../../api";
import type { MeetingRow, ScheduleEvent, ScheduleView } from "../../generated/types";
import { timeLabel } from "../../library/rows";
import { t } from "../../l10n";
import type { WIconName } from "../../system";
import type { CaptureState } from "../../shell/redesign/captureModel";
import { pauseEndLabel } from "../../shell/redesign/captureModel";

const MINUTE = 60_000;

// --- the next meeting (the hero) -----------------------------------------------------------------

/** "in 9 min", "in 2 hr", "in 5 hr 8 min"; "now" once it has started. Written for assistive technology as it reads. */
export function soonLabel(startMs: number, nowMs: number): string {
  if (nowMs >= startMs) return t("now");
  const minutes = Math.ceil((startMs - nowMs) / MINUTE);
  if (minutes < 60) return t("in {minutes} min", { minutes });
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? t("in {hours} hr", { hours }) : t("in {hours} hr {minutes} min", { hours, minutes: rest });
}

export type Hero =
  /** A meeting is recording. `meetingId` is the one the shell calls most relevant, when it knows. */
  | { kind: "live"; meetingId: number | null; title: string; since: number | null }
  | { kind: "next"; event: ScheduleEvent }
  /** No calendar is connected and nothing is scheduled. */
  | { kind: "noCalendar" }
  /** A calendar is connected and the day has no meetings at all. */
  | { kind: "noMeetings" }
  /** The day had meetings and none is left. */
  | { kind: "none" };

export interface HeroInput {
  now: number;
  schedule: ScheduleView | null;
  /** The meeting the shell says matters now (the recording one), if any. */
  relevantMeeting: number | null;
  /** Null while unknown; false when no calendar account is connected. */
  calendarConnected: boolean | null;
  /** When the recording began, from the recording status, if known. */
  recordingSince?: number | null;
}

/** The first event still ahead with no recording. A meeting in progress that nobody recorded counts: its Start is
 *  still the useful action. */
export function nextEvent(schedule: ScheduleView | null, now: number): ScheduleEvent | null {
  return schedule?.events.find((e) => e.recordedMeetingId == null && e.endUnixMs > now) ?? null;
}

export function heroOf(input: HeroInput): Hero {
  const { schedule, now } = input;
  if (schedule?.recording) {
    const id = input.relevantMeeting;
    const event = schedule.events.find((e) => id != null && e.recordedMeetingId === id) ?? schedule.events.find((e) => e.recordedMeetingId != null && e.startUnixMs <= now && now < e.endUnixMs);
    const held = schedule.earlierToday.find((m) => m.id === id);
    return { kind: "live", meetingId: id ?? event?.recordedMeetingId ?? null, title: event?.title ?? held?.title ?? t("Recording"), since: input.recordingSince ?? held?.tsUnixMs ?? event?.startUnixMs ?? null };
  }
  const event = nextEvent(schedule, now);
  if (event) return { kind: "next", event };
  const any = (schedule?.events.length ?? 0) > 0 || (schedule?.earlierToday.length ?? 0) > 0;
  if (!any && input.calendarConnected === false) return { kind: "noCalendar" };
  if (!any) return { kind: "noMeetings" };
  return { kind: "none" };
}

// --- the time line --------------------------------------------------------------------------------

export interface MeetingStop {
  kind: "meeting";
  key: string;
  at: number;
  end: number;
  node: "past" | "upcoming" | "recording";
  title: string;
  timeText: string;
  /** Names, for the speaker chips. */
  attendees: string[];
  /** "in 9 min" for what is ahead; null once over. */
  soon: string | null;
  action: "open" | "start" | null;
  meetingId: number | null;
  eventId: string | null;
  /** "10:31 AM, Northwind security review, in 9 minutes": the stop's name for assistive technology. */
  name: string;
}

export interface MemoryStop {
  kind: "memory";
  key: string;
  at: number;
  end: number;
  block: DayBlock;
}

export interface NowStop {
  kind: "now";
  key: "now";
  at: number;
}

export type Stop = MeetingStop | MemoryStop | NowStop;

export interface TimelineInput {
  now: number;
  locale: string;
  schedule: ScheduleView | null;
  /** The event the hero shows; the time line leaves it out. */
  heroEventId: string | null;
  /** The recording meeting the hero shows (live): left out too. */
  heroMeetingId: number | null;
  blocks: DayBlock[];
}

function stopName(time: string, title: string, soon: string | null): string {
  return [time, title, soon].filter((p): p is string => !!p).join(", ");
}

/** Past meetings, memory blocks, the now marker and what is ahead, oldest first. */
export function buildTimeline(input: TimelineInput): Stop[] {
  const { now, locale, schedule } = input;
  const stops: Stop[] = [];
  const recordedIds = new Set<number>();
  for (const e of schedule?.events ?? []) {
    if (e.recordedMeetingId != null) recordedIds.add(e.recordedMeetingId);
    if (e.eventId === input.heroEventId) continue;
    if (e.recordedMeetingId != null && e.recordedMeetingId === input.heroMeetingId) continue;
    const over = e.endUnixMs <= now;
    const timeText = timeLabel(e.startUnixMs, locale);
    const soon = over ? null : soonLabel(e.startUnixMs, now);
    const recording = !over && e.recordedMeetingId != null && !!schedule?.recording;
    stops.push({
      kind: "meeting",
      key: `event-${e.eventId}`,
      at: e.startUnixMs,
      end: e.endUnixMs,
      node: recording ? "recording" : over ? "past" : "upcoming",
      title: e.title,
      timeText,
      attendees: e.attendees.slice(0, 3),
      soon,
      action: e.recordedMeetingId != null ? "open" : over || schedule?.recording ? null : "start",
      meetingId: e.recordedMeetingId ?? null,
      eventId: e.eventId,
      name: stopName(timeText, e.title, soon),
    });
  }
  for (const m of schedule?.earlierToday ?? ([] as MeetingRow[])) {
    if (recordedIds.has(m.id) || m.id === input.heroMeetingId) continue;
    const timeText = timeLabel(m.tsUnixMs, locale);
    stops.push({
      kind: "meeting",
      key: `meeting-${m.id}`,
      at: m.tsUnixMs,
      end: m.endUnixMs ?? m.tsUnixMs,
      node: "past",
      title: m.title,
      timeText,
      attendees: [],
      soon: null,
      action: "open",
      meetingId: m.id,
      eventId: null,
      name: stopName(timeText, m.title, null),
    });
  }
  input.blocks.forEach((block, i) => stops.push({ kind: "memory", key: `block-${block.start_unix_ms}-${i}`, at: block.start_unix_ms, end: block.end_unix_ms, block }));
  stops.push({ kind: "now", key: "now", at: now });
  const rank = (s: Stop) => (s.kind === "meeting" ? 0 : s.kind === "now" ? 1 : 2);
  return stops.sort((a, b) => a.at - b.at || rank(a) - rank(b));
}

/** The `[start, end]` pairs the memory read cuts blocks around: every meeting stop of the day. */
export function meetingIntervals(schedule: ScheduleView | null): [number, number][] {
  const out: [number, number][] = [];
  for (const e of schedule?.events ?? []) out.push([e.startUnixMs, e.endUnixMs]);
  for (const m of schedule?.earlierToday ?? []) out.push([m.tsUnixMs, m.endUnixMs ?? m.tsUnixMs]);
  return out;
}

/** Local midnight at the start of the day `now` is in. */
export function startOfDay(now: number): number {
  return new Date(now).setHours(0, 0, 0, 0);
}

// --- memory blocks -------------------------------------------------------------------------------

const APP_NAMES: Record<string, string> = {
  msedge: "Microsoft Edge",
  chrome: "Google Chrome",
  firefox: "Firefox",
  code: "Visual Studio Code",
  excel: "Excel",
  winword: "Word",
  powerpnt: "PowerPoint",
  outlook: "Outlook",
  olk: "Outlook",
  teams: "Teams",
  slack: "Slack",
  notepad: "Notepad",
  explorer: "File Explorer",
};

/** A readable app name from the identity capture stores (`excel.exe`, `MSEdge`, `MSEdge.profile2.Default`). */
export function appName(id: string): string {
  const base = id.split(".")[0] === "" ? id : id.split(".")[0];
  const known = APP_NAMES[base.toLowerCase()];
  if (known) return known;
  return base.charAt(0).toUpperCase() + base.slice(1);
}

const DOTS = ["blue", "green", "purple", "amber"] as const;
export type AppDot = (typeof DOTS)[number];

/** One of four app colors, the same for the same app every time. */
export function appDot(id: string): AppDot {
  let h = 0;
  for (const c of appName(id).toLowerCase()) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return DOTS[h % DOTS.length];
}

export function appIcon(id: string): WIconName {
  const n = appName(id).toLowerCase();
  if (n.includes("excel")) return "table";
  if (n.includes("outlook") || n.includes("mail")) return "mail";
  return "document";
}

export interface BlockItem {
  app: AppDot;
  icon: WIconName;
  name: string;
}

/** The apps and titles of a block as rows: "Pricing v3.xlsx in Excel", then what was left out. Window titles and app
 *  names only; body text is never read. */
export function blockItems(block: DayBlock): BlockItem[] {
  const items: BlockItem[] = [];
  for (const a of block.apps) {
    const app = appName(a.app_bundle_id);
    const dot = appDot(a.app_bundle_id);
    const icon = appIcon(a.app_bundle_id);
    if (a.items.length === 0) items.push({ app: dot, icon, name: app });
    for (const title of a.items) items.push({ app: dot, icon, name: t("{title} in {app}", { title, app }) });
    if (a.more_items > 0) items.push({ app: dot, icon, name: t("{count} more in {app}", { count: a.more_items, app }) });
  }
  if (block.more_apps > 0) items.push({ app: "blue", icon: "apps", name: t("{count} more apps", { count: block.more_apps }) });
  // WMemoryBlock keys its rows by name, so a repeated name would collide.
  const seen = new Map<string, number>();
  return items.map((i) => {
    const n = (seen.get(i.name) ?? 0) + 1;
    seen.set(i.name, n);
    return n === 1 ? i : { ...i, name: `${i.name} (${n})` };
  });
}

export function blockRange(block: DayBlock, locale: string): string {
  return t("{start} to {end}", { start: timeLabel(block.start_unix_ms, locale), end: timeLabel(block.end_unix_ms, locale) });
}

/** How long the quiet stretch before a stop was, for the margin: "22 min". Null under a minute. */
export function minutesLabel(block: DayBlock): string | null {
  const minutes = Math.round((block.end_unix_ms - block.start_unix_ms) / MINUTE);
  return minutes >= 1 ? t("{minutes} min", { minutes }) : null;
}

// --- when there is nothing to draw --------------------------------------------------------------

/** The one line that stands where blocks would be when memory is not capturing, so an empty stretch is never mistaken
 *  for nothing having happened. Null while memory is on. */
export function captureLine(state: CaptureState | null, now: number, locale: string): string | null {
  if (!state || state.kind === "on") return null;
  if (state.kind === "off") return t("Memory is off.");
  return t("Memory is paused until {time}.", { time: pauseEndLabel(state.resumeAt, now, locale) });
}

/** No meetings and no blocks: the day is empty. */
export function dayIsEmpty(stops: Stop[]): boolean {
  return !stops.some((s) => s.kind !== "now");
}

// --- the search filters --------------------------------------------------------------------------

export type DateChoice =
  | { kind: "today" }
  | { kind: "yesterday" }
  | { kind: "last7" }
  /** Local dates as `YYYY-MM-DD`, both included. */
  | { kind: "custom"; from: string; to: string };

export interface SearchFilter {
  /** A person from the attendees of recent meetings, or null for anyone. */
  person: string | null;
  /** The app identity as the store keeps it, or null for any. */
  app: string | null;
  dates: DateChoice | null;
}

export const NO_FILTER: SearchFilter = { person: null, app: null, dates: null };

const DAY = 86_400_000;

function localDay(ymd: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(ymd);
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).getTime() : null;
}

/** `[start, end)` in unix ms for a date choice, in local time. A custom range with an unreadable or backwards date
 *  is null. */
export function dateRange(choice: DateChoice, now: number): { start: number; end: number } | null {
  const today = startOfDay(now);
  const next = (day: number) => new Date(new Date(day).getFullYear(), new Date(day).getMonth(), new Date(day).getDate() + 1).getTime();
  switch (choice.kind) {
    case "today":
      return { start: today, end: next(today) };
    case "yesterday":
      return { start: today - DAY, end: today };
    case "last7":
      return { start: today - 6 * DAY, end: next(today) };
    case "custom": {
      const from = localDay(choice.from);
      const to = localDay(choice.to);
      if (from === null || to === null || to < from) return null;
      return { start: from, end: next(to) };
    }
  }
}

/** What goes to the filtered search: the app and the bounds, or nulls. A filter with no choices is no filter. */
export function searchArgs(filter: SearchFilter, now: number): { person: string | null; app: string | null; startUnixMs: number | null; endUnixMs: number | null } {
  const range = filter.dates ? dateRange(filter.dates, now) : null;
  return { person: filter.person, app: filter.app, startUnixMs: range?.start ?? null, endUnixMs: range?.end ?? null };
}

export const hasFilter = (f: SearchFilter): boolean => f.person !== null || f.app !== null || f.dates !== null;

/** The people a filter can name: attendees of recent meetings, most often seen first, then by name. The user ("Me",
 *  "You") and blanks are left out; the same person in different case is one. */
export function peopleFrom(attendeeLists: string[][]): string[] {
  const seen = new Map<string, { name: string; count: number }>();
  for (const list of attendeeLists)
    for (const raw of list) {
      const name = raw.trim();
      const key = name.toLowerCase();
      if (!name || key === "me" || key === "you") continue;
      const hit = seen.get(key);
      if (hit) hit.count += 1;
      else seen.set(key, { name, count: 1 });
    }
  return [...seen.values()].sort((a, b) => b.count - a.count || a.name.localeCompare(b.name)).map((x) => x.name);
}

export function dateChoiceLabel(choice: DateChoice): string {
  switch (choice.kind) {
    case "today":
      return t("Today");
    case "yesterday":
      return t("Yesterday");
    case "last7":
      return t("Last 7 days");
    case "custom":
      return t("{from} to {to}", { from: choice.from, to: choice.to });
  }
}

// --- the line, newest at the top ---------------------------------------------------------------------

/** The stops split at the now marker and each side ordered newest first: what is ahead (the furthest ahead at the
 *  top, so the next meeting sits just above the marker) and what is behind (the latest just below it). */
export function arrangeTimeline(stops: Stop[]): { ahead: Stop[]; behind: Stop[] } {
  const ordered = [...stops].filter((s) => s.kind !== "now").sort((a, b) => b.at - a.at);
  const nowAt = stops.find((s) => s.kind === "now")?.at ?? 0;
  return { ahead: ordered.filter((s) => s.at > nowAt), behind: ordered.filter((s) => s.at <= nowAt) };
}

export interface EarlierGroup {
  id: "yesterday" | "lastWeek";
  /** The day or the span the group covers, written for the locale ("Mon, Nov 16"). */
  note: string;
  meetings: MeetingRow[];
}

/** Yesterday's meetings and the older ones, from the recent meetings Home already loads (newest first). A group with
 *  no meetings is not there. */
export function earlierGroups(earlier: MeetingRow[], now: number, locale: string): EarlierGroup[] {
  const today = startOfDay(now);
  const yesterday = new Date(new Date(today).getFullYear(), new Date(today).getMonth(), new Date(today).getDate() - 1).getTime();
  const day = (ms: number) => new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" }).format(new Date(ms));
  const y = earlier.filter((m) => m.tsUnixMs >= yesterday && m.tsUnixMs < today);
  const w = earlier.filter((m) => m.tsUnixMs < yesterday);
  const out: EarlierGroup[] = [];
  if (y.length) out.push({ id: "yesterday", note: day(yesterday), meetings: y });
  if (w.length) out.push({ id: "lastWeek", note: day(w[0].tsUnixMs), meetings: w });
  return out;
}
