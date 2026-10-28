// What the six Today layouts show and which of their actions are on, decided once (the Mac's `TodayRows`
// and `TodayContent`). The layouts style these rows and wire the actions; they do not re-derive them.
// Pure: nothing here touches the DOM.

import type { DeviceCount, MeetingRow, NextMeetingView, ScheduleEvent, ScheduleView, ThreadRow, TodayOverview, TodayRecap } from "../generated/types";
import { dateTimeLabel, timeLabel } from "../library/rows";
import { WHIMBREL } from "../skin";
import { attendeesLine, deviceLine, greeting, relativeStart, rowAction } from "./todayModel";

// --- sections ---------------------------------------------------------------------------------

/** The parts of Today a layout can place, in the standard order (the Mac's `TodaySection.allCases`). */
export type SectionId = "greeting" | "composer" | "upNext" | "schedule" | "earlier" | "threads" | "recap" | "devices";

export const SECTIONS: readonly SectionId[] = ["greeting", "composer", "upNext", "schedule", "earlier", "threads", "recap", "devices"];

export type ArchetypeId = "standard" | "terminal" | "glass" | "field" | "almanac" | "reference";

export const ARCHETYPE_IDS: readonly ArchetypeId[] = ["standard", "terminal", "glass", "field", "almanac", "reference"];

/** An archetype id from a manifest, or Standard for one this build does not know. */
export const archetypeOf = (id: string | undefined): ArchetypeId => ARCHETYPE_IDS.find((a) => a === id) ?? "standard";

/**
 * The sections each layout draws itself, in the order it draws them. Anything else that is non-empty
 * follows as a plain list (`leftovers`). Taken from the Mac's layout files; a test reads those files and
 * compares. Standard is Windows' own Today, which has no separate up next and no Earlier list.
 */
export const PLACED: Record<ArchetypeId, readonly SectionId[]> = {
  standard: ["greeting", "composer", "schedule", "recap", "threads", "devices"],
  terminal: ["greeting", "composer", "upNext", "schedule", "earlier", "threads", "recap", "devices"],
  glass: ["greeting", "composer", "upNext", "schedule", "recap"],
  field: ["greeting", "composer", "upNext", "schedule", "earlier"],
  almanac: ["greeting", "composer", "upNext", "schedule", "earlier", "threads", "recap", "devices"],
  // The infobox (up next) comes before the composer; Contents lists them in `REFERENCE_CONTENTS` order.
  reference: ["greeting", "upNext", "composer", "schedule", "earlier", "threads", "recap", "devices"],
};

/** The order of Reference's Contents box (the Mac's `entries`). */
export const REFERENCE_CONTENTS: readonly SectionId[] = ["composer", "upNext", "schedule", "earlier", "threads", "recap", "devices"];

// --- voice ------------------------------------------------------------------------------------

export type VoiceKey =
  | "greetingMorning"
  | "greetingAfternoon"
  | "greetingEvening"
  | "upNextHeader"
  | "upcomingHeader"
  | "earlierHeader"
  | "recapHeader"
  | "emptyDay"
  | "emptyEarlier"
  | "askPlaceholder"
  | "tagline";

/** A voice string with `{name}` filled in. A key the skin leaves out reads as Whimbrel's. */
export function voiceText(voice: Record<string, string>, key: VoiceKey, name = ""): string {
  const text = voice[key] ?? WHIMBREL.skin.voice[key] ?? "";
  return text.split("{name}").join(name);
}

/** Today's heading in the skin's voice: the date over "Good morning, <first name>" worded by the skin. With
 *  no usable name the date is the title, as in `greeting`. */
export function greetingFor(name: string, now: Date, voice: Record<string, string>): { kicker: string | null; title: string } {
  const plain = greeting(name, now);
  if (plain.kicker === null) return plain;
  const hour = now.getHours();
  const key = hour < 12 ? "greetingMorning" : hour < 17 ? "greetingAfternoon" : "greetingEvening";
  return { kicker: plain.kicker, title: voiceText(voice, key, name.trim().split(/\s+/)[0]) };
}

// --- what Today knows -------------------------------------------------------------------------

/** The Daily Brief, the digest, or why neither is there. */
export type RecapModel =
  | { kind: "brief"; text: string; origin: string | null }
  | { kind: "digest"; text: string }
  | { kind: "failed"; message: string; retryEnabled: boolean }
  | { kind: "writing" };

/** The recap as the layouts show it: the Daily Brief, else the digest, else why there is none. */
export function recapModel(overview: Pick<TodayOverview, "briefRecap">, recap: TodayRecap | null, writing: boolean, error: string | null): RecapModel | null {
  if (overview.briefRecap) return { kind: "brief", text: overview.briefRecap.summary, origin: overview.briefRecap.from?.label ?? null };
  if (recap) return { kind: "digest", text: recap.summary };
  if (error) return { kind: "failed", message: error, retryEnabled: !writing };
  return writing ? { kind: "writing" } : null;
}

export interface TodayModel {
  /** Milliseconds; the clock the countdowns are measured against. */
  now: number;
  /** The Profile name; empty when there is none. */
  name: string;
  schedule: ScheduleView | null;
  next: NextMeetingView | null;
  /** Meetings from before today, newest first, as many as Today lists. */
  earlier: MeetingRow[];
  openLoops: ThreadRow[];
  recap: RecapModel | null;
  devices: DeviceCount[];
}

// --- rows -------------------------------------------------------------------------------------

export interface EventRow {
  id: string;
  startMs: number;
  timeText: string;
  title: string;
  relative: string | null;
  attendees: string | null;
  /** Only the up-next row has prep state. */
  prep: "ready" | "none" | null;
  /** Start for an event not yet recorded (off while recording), Open once it has a recording. */
  action: "start" | "open" | null;
  recordedMeetingId: number | null;
}

export interface HeldRow {
  id: number;
  startMs: number;
  timeText: string;
  title: string;
  /** Empty on Windows today; the Mac says "Transcribing…" while a meeting is processed. */
  subtitle: string;
}

export type ScheduleItem = { kind: "event"; row: EventRow } | { kind: "separator" } | { kind: "held"; row: HeldRow };

/** The label between the upcoming and the already-held meetings in a schedule (Standard's wording). */
export const HELD_SEPARATOR = "Earlier today";

function eventRow(e: ScheduleEvent, model: TodayModel, recording: boolean, isUpNext: boolean): EventRow {
  const prep = isUpNext ? (model.next?.eventId === e.eventId && model.next.brief ? "ready" : "none") : null;
  return {
    id: e.eventId,
    startMs: e.startUnixMs,
    timeText: timeLabel(e.startUnixMs),
    title: e.title,
    relative: relativeStart(e.startUnixMs, e.endUnixMs, model.now),
    attendees: attendeesLine(e.attendees),
    prep,
    action: rowAction(e, recording),
    recordedMeetingId: e.recordedMeetingId ?? null,
  };
}

const heldRow = (m: MeetingRow, text: (ms: number) => string): HeldRow => ({ id: m.id, startMs: m.tsUnixMs, timeText: text(m.tsUnixMs), title: m.title, subtitle: "" });

/** The first of today's events still ahead that has no recording: the one the day is waiting for. */
export function upNext(model: TodayModel): EventRow | null {
  const schedule = model.schedule;
  const first = schedule?.events.find((e) => e.recordedMeetingId == null);
  return schedule && first ? eventRow(first, model, schedule.recording, true) : null;
}

/** Today's schedule, each list in ascending time order. With `upNextPlaced` the up-next event is left out
 *  of `upcoming` (the layout shows it separately). `held` is the meetings already held today. */
export function schedule(model: TodayModel, upNextPlaced: boolean): { upcoming: EventRow[]; held: HeldRow[] } {
  const s = model.schedule;
  if (!s) return { upcoming: [], held: [] };
  const skip = upNextPlaced ? upNext(model)?.id : undefined;
  const upcoming = s.events
    .filter((e) => e.eventId !== skip)
    .map((e) => eventRow(e, model, s.recording, false))
    .sort((a, b) => a.startMs - b.startMs);
  const held = s.earlierToday.map((m) => heldRow(m, timeLabel)).sort((a, b) => a.startMs - b.startMs);
  return { upcoming, held };
}

/** `schedule`'s two lists as one sequence, with the separator only when both have rows. */
export function scheduleItems(rows: { upcoming: EventRow[]; held: HeldRow[] }): ScheduleItem[] {
  return [
    ...rows.upcoming.map((row): ScheduleItem => ({ kind: "event", row })),
    ...(rows.upcoming.length > 0 && rows.held.length > 0 ? [{ kind: "separator" } as const] : []),
    ...rows.held.map((row): ScheduleItem => ({ kind: "held", row })),
  ];
}

/** The meetings from earlier days, as they arrive (newest first), each with its date and time. */
export function earlierRows(model: TodayModel, now: Date = new Date(model.now)): HeldRow[] {
  return model.earlier.map((m) => heldRow(m, (ms) => dateTimeLabel(ms, now)));
}

/** How many meetings from earlier days Today lists. */
export const EARLIER_LIMIT = 5;

/** The meetings from before today (the local day), newest first, as many as Today lists. */
export function earlierMeetings(rows: MeetingRow[], now: number): MeetingRow[] {
  const dayStart = new Date(now).setHours(0, 0, 0, 0);
  return rows
    .filter((m) => m.tsUnixMs < dayStart)
    .sort((a, b) => b.tsUnixMs - a.tsUnixMs)
    .slice(0, EARLIER_LIMIT);
}

/** "capture line · N other devices": this PC's activity, then how many others reported. */
export function devicesLine(devices: DeviceCount[]): string {
  const mine = devices.find((d) => d.device.thisDevice);
  const others = devices.filter((d) => !d.device.thisDevice).length;
  let line = mine ? deviceLine(mine) : "Nothing yet today";
  if (others > 0) line += ` · ${others} other device${others === 1 ? "" : "s"}`;
  return line;
}

/** This PC's chip label, or the first device's when this one did not report. */
export const thisDeviceLabel = (devices: DeviceCount[]): string | null => (devices.find((d) => d.device.thisDevice) ?? devices[0])?.device.label ?? null;

// --- which sections have something to show ----------------------------------------------------

/** The sections a layout would have something to put in, in the standard order. */
export function nonEmptySections(model: TodayModel): SectionId[] {
  const rows = schedule(model, false);
  return SECTIONS.filter((section) => {
    switch (section) {
      case "greeting":
      case "composer":
      case "devices":
        return true;
      case "upNext":
        return upNext(model) !== null;
      case "schedule":
        return rows.upcoming.length > 0 || rows.held.length > 0;
      case "earlier":
        return model.earlier.length > 0;
      case "threads":
        return model.openLoops.length > 0;
      case "recap":
        return model.recap !== null;
    }
  });
}

/** What an archetype leaves to the plain list after it: non-empty and not placed. The schedule never repeats
 *  the up-next event, so with nothing after it and nothing held it has nothing to draw. */
export function leftovers(archetype: ArchetypeId, model: TodayModel): SectionId[] {
  const placed = PLACED[archetype];
  return nonEmptySections(model).filter((section) => {
    if (placed.includes(section)) return false;
    if (section === "schedule") {
      const rest = schedule(model, true);
      return rest.upcoming.length > 0 || rest.held.length > 0;
    }
    return true;
  });
}

// --- the Field pitch --------------------------------------------------------------------------

/** The numbers behind the pitch drawing, shared with the contrast test (core/skins `FIELD_*_OPACITY`). */
export const FIELD_STRIPE_OPACITY = 0.1;
export const FIELD_LINE_OPACITY = 0.45;

export const FIELD = { markerDiameter: 24, minGap: 44, labelGap: 6, minLabel: 96, maxMarkers: 8 } as const;

export interface FieldSlot {
  centerX: number;
  /** Leading edge of the marker and of its label. */
  labelX: number;
  labelWidth: number;
  /** 0 puts the label above the line, 1 below it. */
  row: 0 | 1;
}

/**
 * Where the markers and their labels go along the pitch (the Mac's `FieldGeometry.slots`). Markers sit on
 * the centre line in start order, spaced by time but never closer than `minGap`. Labels alternate above and
 * below the line, so a label can run to the next label in its own row, two markers on: the widths never
 * overlap, whatever the times.
 */
export function fieldSlots(starts: number[], width: number): FieldSlot[] {
  const n = starts.length;
  if (n === 0) return [];
  const radius = FIELD.markerDiameter / 2;
  const lo = radius;
  const hi = Math.max(lo, width - FIELD.minLabel);
  const gap = n > 1 ? Math.min(FIELD.minGap, (hi - lo) / (n - 1)) : 0;
  const first = starts[0];
  const span = starts[n - 1] - first;
  // By time when the times differ, evenly when they do not.
  const x = starts.map((start, i) => {
    const fraction = span > 0 ? (start - first) / span : n > 1 ? i / (n - 1) : 0;
    return lo + fraction * (hi - lo);
  });
  for (let i = 1; i < n; i++) x[i] = Math.max(x[i], x[i - 1] + gap);
  x[n - 1] = Math.min(x[n - 1], hi);
  for (let i = n - 2; i >= 0; i--) x[i] = Math.min(x[i], x[i + 1] - gap);
  return x.map((centerX, i) => {
    const labelX = centerX - radius;
    const end = i + 2 < n ? x[i + 2] - radius - FIELD.labelGap : width;
    return { centerX, labelX, labelWidth: Math.max(0, end - labelX), row: (i % 2) as 0 | 1 };
  });
}

export interface FieldMarker {
  id: string;
  startMs: number;
  timeText: string;
  title: string;
  isNext: boolean;
  isHeld: boolean;
  action: { kind: "start"; eventId: string } | { kind: "open"; meetingId: number } | null;
}

/** Today's meetings split into pitch markers and the list under the pitch. */
export interface FieldDay {
  markers: FieldMarker[];
  overflow: { upcoming: EventRow[]; held: HeldRow[] };
  /** "+3 more", or null when everything is on the pitch. */
  moreLabel: string | null;
}

/** Upcoming meetings first (up next onward, ascending), then the latest held ones for the slots left, eight
 *  in all, shown in time order. Everything else is the list under the pitch. Up next stays on the pitch as
 *  the highlighted marker as well as on the scoreboard. */
export function fieldDay(model: TodayModel): FieldDay {
  const rows = schedule(model, false);
  const nextId = upNext(model)?.id;
  const upcoming = rows.upcoming.slice(0, FIELD.maxMarkers);
  const spare = FIELD.maxMarkers - upcoming.length;
  const held = spare > 0 ? rows.held.slice(-spare) : [];
  const markers: FieldMarker[] = [
    ...upcoming.map(
      (e): FieldMarker => ({
        id: e.id,
        startMs: e.startMs,
        timeText: e.timeText,
        title: e.title,
        isNext: e.id === nextId,
        isHeld: false,
        action:
          e.action === "start"
            ? { kind: "start", eventId: e.id }
            : e.action === "open" && e.recordedMeetingId != null
              ? { kind: "open", meetingId: e.recordedMeetingId }
              : null,
      }),
    ),
    ...held.map(
      (m): FieldMarker => ({
        id: `held-${m.id}`,
        startMs: m.startMs,
        timeText: m.timeText,
        title: m.title,
        isNext: false,
        isHeld: true,
        action: { kind: "open", meetingId: m.id },
      }),
    ),
  ].sort((a, b) => a.startMs - b.startMs);
  const overflow = { upcoming: rows.upcoming.slice(FIELD.maxMarkers), held: rows.held.slice(0, rows.held.length - held.length) };
  const hidden = overflow.upcoming.length + overflow.held.length;
  return { markers, overflow, moreLabel: hidden > 0 ? `+${hidden} more` : null };
}

/** What a screen reader says for a marker: the verb its button carries, then the meeting. */
export function markerLabel(m: FieldMarker): string {
  const suffix = m.isNext ? ", up next" : "";
  const verb = m.action?.kind === "start" ? "Start " : m.action?.kind === "open" ? "Open " : "";
  return `${verb}${m.title}, ${m.timeText}${suffix}`;
}

// --- Reference --------------------------------------------------------------------------------

/** The infobox floats right from this content width up, and stacks above Contents below it. */
export const REFERENCE_FLOAT_THRESHOLD = 700;
export const REFERENCE_INFOBOX_WIDTH = 240;

export const infoboxPlacement = (width: number): "floated" | "stacked" => (width >= REFERENCE_FLOAT_THRESHOLD ? "floated" : "stacked");

// --- imagery ----------------------------------------------------------------------------------

export type ImagePlacement = "header" | "background" | "badge";

export interface Imagery {
  asset: string;
  placement: ImagePlacement;
  textFree: boolean;
  /** How strongly the page colour covers a background so every text colour reads at 4.5:1. 1 hides it. */
  scrim: number;
}

/** Imagery marked text-free is never placed under text, and a background is under the text. */
export const isDrawn = (image: Pick<Imagery, "placement" | "textFree">): boolean => !(image.textFree && image.placement === "background");

const PLACEMENTS: readonly string[] = ["header", "background", "badge"];

/** The images of a skin that would be drawn. Until the shell has measured an image (a skin you made), its
 *  scrim is 1, which covers it: an image is never shown below 4.5:1. */
export function imageryOf(
  manifest: { decoration: { imagery: { asset: string; placement: string; textFree: boolean }[] } },
  scrims: Record<string, number> = {},
): Imagery[] {
  return manifest.decoration.imagery
    .filter((i) => PLACEMENTS.includes(i.placement))
    .map((i) => ({ asset: i.asset, placement: i.placement as ImagePlacement, textFree: i.textFree, scrim: scrims[i.asset] ?? 1 }))
    .filter(isDrawn);
}
