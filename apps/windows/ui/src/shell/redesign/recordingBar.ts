/** Whole seconds since a recording began, never negative. */
export function elapsedSeconds(startedMs: number, nowMs: number): number {
  return Math.max(0, Math.floor((nowMs - startedMs) / 1000));
}
