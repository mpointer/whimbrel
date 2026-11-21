// Speaker lanes above the transcript (redesign flag on only): one lane per speaker, a span for each line, the marked
// moments as ticks. Pure. The span rule is the Mac's (apps/macos WLaneMath): a stored end wins; otherwise a line runs until
// the next line starts, at most 30 seconds; the last line is drawn 5 seconds.

import type { MeetingMarkerView, TranscriptLineView } from "../../generated/types";
import { lineSpeaker, offsetClock } from "./noteModel";

/** The longest a line with no stored end runs, in centiseconds (30 seconds). */
export const CAP_CS = 3000;
/** How long the last line is drawn, in centiseconds (5 seconds). */
export const LAST_LINE_CS = 500;

export function spanEnd(startCs: number, storedEndCs: number | null | undefined, nextStartCs: number | null): number {
  if (storedEndCs != null && storedEndCs > startCs) return storedEndCs;
  if (nextStartCs == null) return startCs + LAST_LINE_CS;
  return Math.max(startCs, Math.min(nextStartCs, startCs + CAP_CS));
}

export interface LaneSpan {
  id: number;
  startCs: number;
  endCs: number;
  speaker: string;
  time: string;
}

export interface Lane {
  key: string;
  speaker: string;
  spans: LaneSpan[];
}

export interface LaneTick {
  id: number;
  atCs: number;
  label: string;
}

export interface LaneModel {
  lanes: Lane[];
  /** The meeting's length on the lanes' clock: where the last span or mark ends. */
  endCs: number;
  ticks: LaneTick[];
}

/** Lanes in the order each speaker first speaks. Lines are taken in time order, ties by id. */
export function buildLanes(lines: readonly TranscriptLineView[], marks: readonly MeetingMarkerView[], markLabel: (m: MeetingMarkerView) => string): LaneModel {
  const sorted = [...lines].sort((a, b) => a.startCs - b.startCs || a.id - b.id);
  const lanes = new Map<string, Lane>();
  let endCs = 0;
  sorted.forEach((line, i) => {
    const next = i + 1 < sorted.length ? sorted[i + 1].startCs : null;
    const end = spanEnd(line.startCs, line.endCs, next);
    endCs = Math.max(endCs, end);
    const speaker = lineSpeaker(line);
    const lane = lanes.get(line.speaker) ?? { key: line.speaker, speaker, spans: [] };
    lanes.set(line.speaker, lane);
    lane.spans.push({ id: line.id, startCs: line.startCs, endCs: end, speaker, time: offsetClock(line.startCs) });
  });
  const ticks = marks.map((m) => ({ id: m.id, atCs: m.atCs, label: markLabel(m) }));
  for (const tick of ticks) endCs = Math.max(endCs, tick.atCs);
  return { lanes: [...lanes.values()], endCs: Math.max(endCs, 1), ticks };
}

/** Left or right as a percentage of the lanes' width. */
export function percent(cs: number, endCs: number): number {
  return Math.min(100, Math.max(0, (cs / Math.max(endCs, 1)) * 100));
}

const flat = (model: LaneModel): LaneSpan[] => model.lanes.flatMap((l) => l.spans).sort((a, b) => a.startCs - b.startCs || a.id - b.id);

/** The span `delta` places along in time (left and right arrows); stays at the ends. With none current: the first, or the last going back. */
export function neighborSpan(model: LaneModel, current: number | null, delta: number): number | null {
  const all = flat(model);
  if (all.length === 0) return null;
  const at = all.findIndex((s) => s.id === current);
  if (at < 0) return (delta >= 0 ? all[0] : all[all.length - 1]).id;
  return all[Math.min(Math.max(at + delta, 0), all.length - 1)].id;
}

/** The span in the lane `delta` lanes away (up and down arrows) that starts nearest the current one; stays at the ends. */
export function otherLaneSpan(model: LaneModel, current: number | null, delta: number): number | null {
  const all = flat(model);
  const from = all.find((s) => s.id === current);
  if (!from) return all.length ? all[0].id : null;
  const laneAt = model.lanes.findIndex((l) => l.spans.some((s) => s.id === current));
  const target = model.lanes[Math.min(Math.max(laneAt + delta, 0), model.lanes.length - 1)];
  let best = target.spans[0];
  for (const s of target.spans) if (Math.abs(s.startCs - from.startCs) < Math.abs(best.startCs - from.startCs)) best = s;
  return best.id;
}

/** How many transcript rows the list has drawn: a window that only grows, so rows already on screen never move. */
export interface RowRange {
  from: number;
  to: number;
}

export const ROW_CHUNK = 80;

/** The window to start with: round a line, or the top of the transcript. */
export function rangeAround(index: number | null, total: number): RowRange {
  if (index == null) return { from: 0, to: Math.min(total, ROW_CHUNK) };
  return { from: Math.max(0, index - ROW_CHUNK / 2), to: Math.min(total, index + ROW_CHUNK) };
}

export function growRange(range: RowRange, total: number, side: "up" | "down"): RowRange {
  return side === "up" ? { from: Math.max(0, range.from - ROW_CHUNK), to: range.to } : { from: range.from, to: Math.min(total, range.to + ROW_CHUNK) };
}

/** The range widened to hold an index (a jump to a line already above or below the window). */
export function rangeHolding(range: RowRange, index: number, total: number): RowRange {
  if (index >= range.from && index < range.to) return range;
  return rangeAround(index, total);
}
