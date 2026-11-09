/** How often the live transcript pane reloads while a meeting records. */
export const LIVE_POLL_MS = 5_000;

/** A live line's time from the start of the recording: "4:05", or "1:02:09" past an hour. */
export function liveClock(startCs: number): string {
  const total = Math.max(0, Math.floor(startCs / 100));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/**
 * Whether the pane should follow new lines down: only when the reader is already at (or
 * within a line of) the bottom, so scrolling back to reread isn't yanked away.
 */
export function followsNewLines(box: { scrollTop: number; clientHeight: number; scrollHeight: number }): boolean {
  return box.scrollHeight - box.scrollTop - box.clientHeight < 40;
}
