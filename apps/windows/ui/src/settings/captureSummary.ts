import type { CaptureStatus } from "../generated/types";

/** One line for the Capture card: what capture is doing right now. */
export function captureSummary(s: CaptureStatus, now: Date = new Date()): string {
  if (!s.running) return `Not running${s.lastError ? `: ${s.lastError}` : ""}`;
  if (!s.enabled) return "Off";
  if (s.paused) return `Paused: ${s.paused}`;
  if (s.lastStoredAtMs == null) return "Capturing; nothing stored yet";
  const secs = Math.max(0, Math.round((now.getTime() - s.lastStoredAtMs) / 1000));
  const ago = secs < 60 ? `${secs}s ago` : secs < 3600 ? `${Math.round(secs / 60)} min ago` : `${Math.round(secs / 3600)} h ago`;
  return `Capturing · last from ${s.lastApp ?? "an app"} ${ago}`;
}
