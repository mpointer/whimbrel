import type {
  BriefRecap,
  DeviceCount,
  PrepBriefView,
  NextMeetingView,
  PrepSourceView,
  ProviderKey,
  RoutingSettings,
  ScheduleEvent,
  ThreadRow,
  TodayOverview,
  TodayRecap,
} from "../generated/types";
import { timeLabel } from "../library/rows";

/**
 * Whether Today should generate a recap now. Generating holds the store for a model
 * call, so it happens once a day on its own (on the first look with nothing cached),
 * plus once more if that first look found an empty day and something has been
 * captured since. A failed attempt waits for Refresh.
 */
export function shouldGenerate(overview: TodayOverview, attemptedDay: string | null, recap: TodayRecap | null): boolean {
  // Today's Daily Brief already answers the question; a digest would cost a model call for nothing.
  if (overview.recap || overview.briefRecap) return false;
  if (attemptedDay !== overview.day) return true;
  return !!recap && recap.day === overview.day && recap.emptyDay && overview.devices.some((d) => d.captures > 0);
}

/** The recap to show once the overview loads: today's cached one, or today's in hand. */
export function recapFor(overview: TodayOverview, current: TodayRecap | null): TodayRecap | null {
  if (overview.recap) return overview.recap;
  return current && current.day === overview.day ? current : null;
}

/** Where a Daily Brief recap came from: another device by name, or this PC's own run with its time. */
export function briefRecapNote(brief: BriefRecap): string {
  return brief.from ? `From ${brief.from.label}` : `Daily Brief, written at ${timeLabel(brief.ranAtUnixMs)}.`;
}

export function recapNote(recap: TodayRecap): string {
  if (recap.emptyDay) return "Nothing captured yet today. The recap writes itself once there is something to summarize.";
  return recap.cached ? "Written earlier today." : "Just written.";
}

/** "1,204 captures · last 2:05 PM", or "Nothing yet today". */
export function deviceLine(d: DeviceCount): string {
  if (d.captures === 0 || d.lastTsUnixMs == null) return "Nothing yet today";
  const n = d.captures.toLocaleString("en-US");
  return `${n} capture${d.captures === 1 ? "" : "s"} · last ${timeLabel(d.lastTsUnixMs)}`;
}

export function threadLine(t: ThreadRow): string {
  const who = t.status === "waiting_on_me" ? "Waiting on you" : "Waiting on them";
  return t.lastMessageFrom ? `${who} · last from ${t.lastMessageFrom}` : who;
}

/** What the next-meeting card says about its brief: when it was made, or why there is none yet. */
export function prepStatus(brief: PrepBriefView | null | undefined): string {
  if (!brief) return "No brief yet. Meeting Prep makes one shortly before the meeting once it is turned on in Automations.";
  return `Prepared at ${timeLabel(brief.generatedAtUnixMs)}.`;
}

/** The sources a claim cites, in the order cited; an id the brief does not list is dropped. */
export function citedSources(brief: PrepBriefView, ids: string[]): PrepSourceView[] {
  return ids.flatMap((id) => brief.sources.filter((s) => s.id === id));
}

const DATE_FORMAT = new Intl.DateTimeFormat("en-US", { weekday: "long", month: "long", day: "numeric" });

/**
 * Today's heading (the Mac's TodayLogic.greeting): the date as a small line above "Good morning,
 * <first word of the Profile name>". With no usable name (empty, blank, or an email address, which
 * is what a sign-in can leave there) the weekday and date are the title and there is no extra line.
 */
export function greeting(name: string, now: Date): { kicker: string | null; title: string } {
  const date = DATE_FORMAT.format(now);
  const first = name.trim().split(/\s+/)[0] ?? "";
  if (first === "" || name.includes("@")) return { kicker: null, title: date };
  const hour = now.getHours();
  const part = hour < 12 ? "morning" : hour < 17 ? "afternoon" : "evening";
  return { kicker: date, title: `Good ${part}, ${first}` };
}

/**
 * Whether Library's Ask can answer now (the Mac's TodayComposer.askAvailable): a local model always
 * can; a hosted provider needs a saved key and local-only mode off.
 */
export function askAvailable(routing: RoutingSettings, keys: ProviderKey[]): boolean {
  if (routing.preferredProvider === "ollama") return true;
  return !routing.localOnly && keys.some((k) => k.provider === routing.preferredProvider && !!k.masked);
}

/** The model Ask will use by default, for the composer's read-only label. */
export function askModelLabel(routing: RoutingSettings): string {
  switch (routing.preferredProvider) {
    case "ollama":
      return routing.ollamaModel;
    case "anthropic":
      return routing.anthropicModel;
    case "openai":
      return routing.openaiModel;
    case "google":
      return routing.googleModel;
  }
}

/** What the composer sends: the trimmed text, or nothing for a blank field. */
export function composerPrompt(text: string): string | null {
  const trimmed = text.trim();
  return trimmed === "" ? null : trimmed;
}

const MINUTE = 60_000;

/**
 * The schedule row's countdown (the Mac's TodayLogic.relativeStart): "Now" while the event is on, "In 9m"
 * (minutes rounded up) or "In 2h" (hours floored) before it, nothing once it is over or when it is more
 * than 12 hours away.
 */
export function relativeStart(startMs: number, endMs: number, nowMs: number): string | null {
  if (nowMs >= endMs) return null;
  if (nowMs >= startMs) return "Now";
  const minutes = Math.ceil((startMs - nowMs) / MINUTE);
  if (minutes > 12 * 60) return null;
  return minutes < 60 ? `In ${minutes}m` : `In ${Math.floor(minutes / 60)}h`;
}

/** Up to two invitees, then "+N" for the rest; nothing for an event with none. */
export function attendeesLine(attendees: string[]): string | null {
  if (attendees.length === 0) return null;
  const shown = attendees.slice(0, 2).join(", ");
  return attendees.length > 2 ? `${shown} +${attendees.length - 2}` : shown;
}

/**
 * What a row offers: Open for an event that has a recording, Start for one that does not (hidden on every
 * row while a meeting is recording), nothing otherwise.
 */
export function rowAction(event: ScheduleEvent, recording: boolean): "open" | "start" | null {
  if (event.recordedMeetingId != null) return "open";
  return recording ? null : "start";
}

/**
 * The prep line, on the next meeting's row only (the one event with prep data): "Prep ready" when its
 * brief exists, "No prep yet" when not. An event already recorded, or any other event, has none.
 */
export function prepLine(event: ScheduleEvent, next: NextMeetingView | null): "ready" | "none" | null {
  if (!next || next.eventId !== event.eventId || event.recordedMeetingId != null) return null;
  return next.brief ? "ready" : "none";
}
