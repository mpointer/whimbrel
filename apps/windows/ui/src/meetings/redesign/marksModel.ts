// "Mark this moment": what a mark is labeled with and how the list reads. Pure. The stored time is the offset from the start of
// the recording in centiseconds on the clock live transcript lines and alerts use (`clockOffsetCs`), which includes paused time;
// the list shows the clock time of day for it.

import type { MeetingMarkerView } from "../../generated/types";

/** The longest label kept (core cuts at 200 characters; the screen shows the line as written). */
export const LABEL_CHARS = 200;

/** The notes line being written: the line the caret is on, or the last line when the caret is unknown. Blank is `null`. */
export function markLabel(notes: string, caret: number | null): string | null {
  const at = caret == null ? notes.length : Math.min(Math.max(caret, 0), notes.length);
  const start = notes.lastIndexOf("\n", at - 1) + 1;
  const end = notes.indexOf("\n", at);
  const line = notes.slice(start, end === -1 ? notes.length : end).trim();
  // A caret on a fresh empty line means the line just above is the one being written.
  if (!line && at > 0 && notes[at - 1] === "\n") return markLabel(notes, at - 1);
  return line ? line.slice(0, LABEL_CHARS) : null;
}

/** The marks earliest first, ties by id, as core lists them. */
export function sortedMarks(marks: readonly MeetingMarkerView[]): MeetingMarkerView[] {
  return [...marks].sort((a, b) => a.atCs - b.atCs || a.id - b.id);
}

/** The wall-clock moment a mark points at: the recording's start plus its offset. */
export function markClockMs(startedUnixMs: number, atCs: number): number {
  return startedUnixMs + atCs * 10;
}
