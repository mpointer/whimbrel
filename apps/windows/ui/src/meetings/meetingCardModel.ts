// One model for every meeting card and row (docs/superpowers/specs/2026-11-10-meeting-lists-design.md, the
// Mac's MeetingCardModel): Today's timeline cards, the Meetings list rows and the skinned layouts' chips all
// decide what a meeting shows and where a click goes here, so every list agrees. Pure: no React, no calls.

import type { MeetingRow, NextMeetingView, ScheduleEvent, TopicRefView } from "../generated/types";
import { timeLabel, shortDate } from "../library/rows";
import { t } from "../l10n";
import { relativeStart } from "../today/todayModel";

/** Where clicking a card goes. `none` means the card is inert: no hover, no chevron. */
export type MeetingCardTarget =
  | { kind: "meeting"; id: number }
  | { kind: "brief"; key: string }
  | { kind: "topic"; id: number }
  | { kind: "none" };

/** One chip on a card. Each opens its own item: Prep the brief, Topic the topic page, the rest the meeting. */
export type MeetingChip =
  | { kind: "prep"; key: string }
  | { kind: "summary" }
  | { kind: "actionItems"; count: number }
  | { kind: "topic"; id: number; name: string };

export interface MeetingCardModel {
  title: string;
  /** "9:00 AM to 9:45 AM", or just the start when there is no end or it isn't after the start. */
  timeRange: string;
  /** Just the start, for the Meetings row's time column. */
  startText: string;
  durationMinutes: number | null;
  /** Attendees plus you; null for a recorded meeting with no attendees. */
  peopleCount: number | null;
  isPast: boolean;
  /** Started and not over: a filled accent dot, after the Now marker. */
  isRunning: boolean;
  chips: MeetingChip[];
  target: MeetingCardTarget;
  /** "In 30m" on the next upcoming card only; null elsewhere. */
  hint: string | null;
  /** Windows' quiet extra line on a recorded meeting ("Recording", "No summary yet"); null when neither applies. */
  statusLine: string | null;
}

// --- copy ---------------------------------------------------------------------------------------

export function peopleText(n: number): string {
  return n === 1 ? t("1 person") : t("{count} people", { count: n });
}

export function actionItemsText(n: number): string {
  return n === 1 ? t("1 action item") : t("{count} action items", { count: n });
}

export function durationText(minutes: number): string {
  return t("{count} min", { count: minutes });
}

export function chipLabel(chip: MeetingChip): string {
  switch (chip.kind) {
    case "prep":
      return t("Prep ready");
    case "summary":
      return t("Summary");
    case "actionItems":
      return actionItemsText(chip.count);
    case "topic":
      return chip.name;
  }
}

/** The card's second line: "9:00 AM to 9:45 AM · 3 people · In 30m". */
export function subtitle(model: MeetingCardModel): string {
  const parts = [model.timeRange];
  if (model.peopleCount != null) parts.push(peopleText(model.peopleCount));
  if (model.hint) parts.push(model.hint);
  return parts.join(" · ");
}

/** What a screen reader says for the card's click target: title and time, then the extras. */
export function spokenLabel(model: MeetingCardModel): string {
  const parts = [model.title, model.timeRange];
  if (model.peopleCount != null) parts.push(peopleText(model.peopleCount));
  if (model.hint) parts.push(model.hint);
  if (model.statusLine) parts.push(model.statusLine);
  return parts.join(", ");
}

// --- times --------------------------------------------------------------------------------------

/** Whole minutes from start to end, or null when there is no end or it isn't after the start. */
export function durationMinutes(startMs: number, endMs: number | null | undefined): number | null {
  if (endMs == null) return null;
  const minutes = Math.round((endMs - startMs) / 60_000);
  return minutes > 0 ? minutes : null;
}

export function timeRange(startMs: number, endMs: number | null | undefined, locale: string): string {
  const start = timeLabel(startMs, locale);
  if (endMs == null || durationMinutes(startMs, endMs) == null) return start;
  return t("{start} to {end}", { start, end: timeLabel(endMs, locale) });
}

// --- building -----------------------------------------------------------------------------------

/** The Windows meta a recorded meeting already showed: recording now, or done without a summary yet. */
export function statusLine(m: Pick<MeetingRow, "status" | "hasSummary">): string | null {
  if (m.status === "recording") return t("Recording");
  if (m.status === "completed" && !m.hasSummary) return t("No summary yet");
  return null;
}

function chipsFor(prepKey: string | null | undefined, summary: boolean, actionItems: number, topic: TopicRefView | null | undefined): MeetingChip[] {
  const chips: MeetingChip[] = [];
  if (prepKey) chips.push({ kind: "prep", key: prepKey });
  if (summary) chips.push({ kind: "summary" });
  if (actionItems > 0) chips.push({ kind: "actionItems", count: actionItems });
  if (topic) chips.push({ kind: "topic", id: topic.topicId, name: topic.name });
  return chips;
}

/**
 * A calendar event on Today. One already recorded opens its meeting and carries its summary chips; one not
 * recorded opens its brief, else its topic page, else nothing.
 */
export function forEvent(e: ScheduleEvent, topic: TopicRefView | null | undefined, now: number, locale: string): MeetingCardModel {
  const recorded = e.recordedMeetingId ?? null;
  const prepKey = e.prepKey ?? null;
  const chips = recorded != null ? chipsFor(prepKey, !!e.hasSummary, e.actionItemCount ?? 0, topic) : chipsFor(prepKey, false, 0, topic);
  const target: MeetingCardTarget =
    recorded != null
      ? { kind: "meeting", id: recorded }
      : prepKey
        ? { kind: "brief", key: prepKey }
        : topic
          ? { kind: "topic", id: topic.topicId }
          : { kind: "none" };
  return {
    title: e.title,
    timeRange: timeRange(e.startUnixMs, e.endUnixMs, locale),
    startText: timeLabel(e.startUnixMs, locale),
    durationMinutes: durationMinutes(e.startUnixMs, e.endUnixMs),
    peopleCount: e.attendees.length + 1,
    isPast: e.endUnixMs <= now,
    isRunning: e.startUnixMs <= now && e.endUnixMs > now,
    chips,
    target,
    hint: null,
    statusLine: null,
  };
}

/** A recorded meeting: it always opens the meeting. A missing end (still recording) leaves the duration out. */
export function forRecorded(m: MeetingRow, prepKey: string | null | undefined, topic: TopicRefView | null | undefined, locale: string): MeetingCardModel {
  const attendees = m.attendeeCount ?? 0;
  return {
    title: m.title,
    timeRange: timeRange(m.tsUnixMs, m.endUnixMs, locale),
    startText: timeLabel(m.tsUnixMs, locale),
    durationMinutes: durationMinutes(m.tsUnixMs, m.endUnixMs),
    peopleCount: attendees > 0 ? attendees + 1 : null,
    isPast: true,
    isRunning: false,
    chips: chipsFor(prepKey, m.hasSummary, m.actionItemCount ?? 0, topic),
    target: { kind: "meeting", id: m.id },
    hint: null,
    statusLine: statusLine(m),
  };
}

/** Where a chip goes: Prep to its brief, Topic to its page, Summary and action items to the meeting (when
 *  there is one; an unrecorded event has neither chip). */
export function chipTarget(chip: MeetingChip, card: MeetingCardTarget): MeetingCardTarget {
  switch (chip.kind) {
    case "prep":
      return { kind: "brief", key: chip.key };
    case "topic":
      return { kind: "topic", id: chip.id };
    default:
      return card.kind === "meeting" ? card : { kind: "none" };
  }
}

// --- Today's timeline ---------------------------------------------------------------------------

export interface TimelineEntry {
  key: string;
  startMs: number;
  /** When it ends; a held meeting with no end counts as ended at its start. */
  endMs: number;
  model: MeetingCardModel;
  /** The calendar event, for Start and the brief toggle; null for a meeting already held. */
  event: ScheduleEvent | null;
  /** The event the next-meeting brief is about: its card carries Open brief / Hide brief or "No prep yet". */
  isNext: boolean;
}

/**
 * Today's schedule as one start-ascending list: the events not over yet, and the meetings held earlier today.
 * Only the first event still ahead gets the "In 30m" hint, even while another one is running.
 */
export function timeline(
  events: ScheduleEvent[],
  held: MeetingRow[],
  next: NextMeetingView | null,
  chips: { eventTopics: (TopicRefView | null)[]; held: ReadonlyMap<number, HeldChips> },
  now: number,
  locale: string,
): TimelineEntry[] {
  const entries: TimelineEntry[] = events.map((e, i) => ({
    key: `event-${e.eventId}@${e.startUnixMs}`,
    startMs: e.startUnixMs,
    endMs: e.endUnixMs,
    model: forEvent(e, chips.eventTopics[i], now, locale),
    event: e,
    isNext: !!next && next.eventId === e.eventId && e.recordedMeetingId == null,
  }));
  for (const m of held) {
    const extra = chips.held.get(m.id);
    entries.push({
      key: `held-${m.id}`,
      startMs: m.tsUnixMs,
      endMs: m.endUnixMs ?? m.tsUnixMs,
      model: forRecorded(m, extra?.prepKey, extra?.topic, locale),
      event: null,
      isNext: false,
    });
  }
  entries.sort((a, b) => a.startMs - b.startMs);
  const first = entries.find((x) => x.event && x.startMs > now);
  if (first?.event) first.model = { ...first.model, hint: relativeStart(first.startMs, first.endMs, now) };
  return entries;
}

/** Where the Now marker goes in a start-ascending timeline: before the first item that hasn't ended (so a
 *  running meeting is after it). 0 when everything is ahead, `ends.length` when everything is behind. */
export function nowMarkerIndex(ends: number[], now: number): number {
  const i = ends.findIndex((end) => end > now);
  return i === -1 ? ends.length : i;
}

export function nowLabel(now: number, locale: string): string {
  return t("Now · {time}", { time: timeLabel(now, locale) });
}

// --- the Meetings list --------------------------------------------------------------------------

function startOfDay(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** "Today" and "Yesterday" as the language says them. The catalog's "Today" is the tab's name and is not
 *  translated, so the relative day comes from the locale's own data, capitalized for a heading. */
function relativeDay(days: number, locale: string): string {
  const text = new Intl.RelativeTimeFormat(locale, { numeric: "auto" }).format(-days, "day");
  return text.charAt(0).toLocaleUpperCase(locale) + text.slice(1);
}

/** A day heading: Today, Yesterday, the weekday within the last week, then the date. */
export function dayHeading(ms: number, now: number, locale: string): string {
  const days = Math.round((startOfDay(now) - startOfDay(ms)) / 86_400_000);
  if (days === 0 || days === 1) return relativeDay(days, locale);
  if (days > 1 && days < 7) return new Date(ms).toLocaleDateString(locale, { weekday: "long" });
  return shortDate(ms, new Date(now), locale);
}

export interface DayGroup<T> {
  key: string;
  title: string;
  rows: T[];
}

/** Rows (newest first, as the list has them) under one heading per local day, in the same order. */
export function groupByDay<T extends { tsUnixMs: number }>(rows: T[], now: number, locale: string): DayGroup<T>[] {
  const groups: DayGroup<T>[] = [];
  for (const row of rows) {
    const key = String(startOfDay(row.tsUnixMs));
    const last = groups[groups.length - 1];
    if (last && last.key === key) last.rows.push(row);
    else groups.push({ key, title: dayHeading(row.tsUnixMs, now, locale), rows: [row] });
  }
  return groups;
}

// --- the chips lookup ---------------------------------------------------------------------------

/** What one background lookup adds to a recorded meeting's card. */
export interface HeldChips {
  prepKey: string | null;
  topic: TopicRefView | null;
}

/**
 * What decides whether the chips are looked up again: the set of meetings (order does not matter) and which of
 * them have a summary, since a meeting gains its topic when it is processed. The list re-reads every 5 s; the
 * lookup runs only when this changes.
 */
export function chipLookupKey(rows: Pick<MeetingRow, "id" | "hasSummary">[]): string {
  return rows
    .map((r) => `${r.id}${r.hasSummary ? "s" : ""}`)
    .sort()
    .join(",");
}

/** Numbers each lookup so only the newest may write its answer: a slow, stale one is dropped. */
export function latestOnly(): { begin(): number; isLatest(ticket: number): boolean } {
  let seq = 0;
  return {
    begin: () => ++seq,
    isLatest: (ticket) => ticket === seq,
  };
}
