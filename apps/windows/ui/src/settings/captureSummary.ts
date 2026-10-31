import type { CaptureStatus } from "../generated/types";
import { t } from "../l10n";

/** One line for the Capture card: what capture is doing right now. */
export function captureSummary(s: CaptureStatus, now: Date = new Date()): string {
  if (!s.running) return s.lastError ? t("Not running: {error}", { error: s.lastError }) : t("Not running");
  if (!s.enabled) return t("Off");
  if (s.paused) return t("Paused: {reason}", { reason: s.paused });
  if (s.lastStoredAtMs == null) return t("Capturing; nothing stored yet");
  const secs = Math.max(0, Math.round((now.getTime() - s.lastStoredAtMs) / 1000));
  const mins = Math.round(secs / 60);
  const hours = Math.round(secs / 3600);
  // One whole line per unit, with and without the app's name, so a language can reorder the parts.
  if (s.lastApp == null) {
    if (secs < 60) return t("Capturing · last from an app {count}s ago", { count: secs });
    if (secs < 3600) return t("Capturing · last from an app {count} min ago", { count: mins });
    return t("Capturing · last from an app {count} h ago", { count: hours });
  }
  const app = s.lastApp;
  if (secs < 60) return t("Capturing · last from {app} {count}s ago", { app, count: secs });
  if (secs < 3600) return t("Capturing · last from {app} {count} min ago", { app, count: mins });
  return t("Capturing · last from {app} {count} h ago", { app, count: hours });
}
