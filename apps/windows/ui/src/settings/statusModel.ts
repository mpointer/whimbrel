import type { AiSettings, DeviceView, HostView, Provider, RetentionSettings, RunView, StatusView, SyncView, UpdateCheck } from "../generated/types";
import { t } from "../l10n";
import { modelField, monthSpent, ollamaWarningLine, providerLabel, thisMonth } from "./aiModel";
import { stateLabel } from "./syncModel";

// What the Settings status page (issue 595, the Mac's StatusOverviewView) says, apart from the component.
// Each builder returns the lines under one row; the page never edits anything.

/** How long the oldest unsent change may wait before the page mentions it (the Mac's SyncTiming). */
export const BACKLOG_STALLED_MS = 60 * 60 * 1000;

/** "5 minutes ago", "yesterday": the Mac's `.relative(presentation: .named)`. */
export function relativeTime(thenMs: number, nowMs: number, locale: string): string {
  const seconds = Math.round((thenMs - nowMs) / 1000);
  const format = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ["day", 86_400],
    ["hour", 3_600],
    ["minute", 60],
  ];
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return format.format(Math.round(seconds / size), unit);
  }
  return format.format(seconds, "second");
}

/** "34.2 MB" or "1.3 GB". */
export function diskSize(bytes: number): string {
  return bytes >= 1_000_000_000 ? `${(bytes / 1_000_000_000).toFixed(1)} GB` : `${(bytes / 1_000_000).toFixed(1)} MB`;
}

/** Whether unsent changes have waited long enough to be worth saying. */
export const backlogStalled = (pending: number, oldestMs: number | null | undefined, nowMs: number): boolean =>
  pending > 0 && oldestMs != null && nowMs - oldestMs >= BACKLOG_STALLED_MS;

export function syncLines(
  view: SyncView | null,
  devices: Pick<DeviceView, "status">[],
  status: Pick<StatusView, "outboxPending" | "outboxOldestMs"> | null,
  nowMs: number,
  locale: string,
): string[] {
  if (!view) return [];
  if (view.state === "notSignedIn" || view.state === "notConfigured") return [t("Sync is off.")];
  // A refused sign-in ("This Google account isn't allowed to sign in") says why, not "setting up".
  if (view.state === "error") return [view.message ?? stateLabel("error")];
  if (view.state !== "ready") return [t("Sync is setting up.")];
  const lines: string[] = [];
  const approved = devices.filter((d) => d.status === "approved").length;
  if (approved > 0) lines.push(t("Devices approved: {count}", { count: approved }));
  if (view.contentError) lines.push(view.contentError);
  else if (view.contentLastSyncedMs != null) lines.push(t("Content last synced {when}", { when: relativeTime(view.contentLastSyncedMs, nowMs, locale) }));
  if (view.settingsError) lines.push(view.settingsError);
  else if (view.settingsLastSyncedMs != null) lines.push(t("Settings last synced {when}", { when: relativeTime(view.settingsLastSyncedMs, nowMs, locale) }));
  if (status && status.outboxOldestMs != null && backlogStalled(status.outboxPending, status.outboxOldestMs, nowMs)) {
    const oldest = new Date(status.outboxOldestMs).toLocaleString(locale, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
    lines.push(t("Changes waiting to send: {count}, the oldest from {when}", { count: status.outboxPending, when: oldest }));
  }
  return lines;
}

/** A provider's plain name: "Ollama", not the picker's "Ollama (on this PC)". */
function providerName(ai: Pick<AiSettings, "providers">, id: Provider): string {
  return ai.providers.find((p) => p.id === id)?.name ?? (id === "ollama" ? "Ollama" : providerLabel(id));
}

export function aiLines(ai: AiSettings | null): string[] {
  if (!ai) return [];
  const provider = ai.routing.preferredProvider;
  const lines = [t("Summaries use {provider}, {model}.", { provider: providerName(ai, provider), model: ai.routing[modelField(provider)] })];
  if (provider === "ollama" && ai.ollamaWarning) lines.push(ollamaWarningLine(ai.ollamaWarning));
  for (const { provider: id, cents } of ai.budgets) {
    const spent = monthSpent(ai.usage, id);
    if (spent <= 0 && !cents) continue;
    lines.push(`${providerName(ai, id)}: ${thisMonth(spent, cents).text}`);
  }
  return lines;
}

export function storageLines(status: Pick<StatusView, "audioBytes" | "imageBytes"> | null, retention: Pick<RetentionSettings, "meetingDays"> | null): string[] {
  const lines: string[] = [];
  if (status) lines.push(t("Audio: {audio}. Images: {images}.", { audio: diskSize(status.audioBytes), images: diskSize(status.imageBytes) }));
  if (retention) {
    lines.push(
      retention.meetingDays == null ? t("Meetings are kept forever.") : t("Meetings are kept for {count} days.", { count: retention.meetingDays }),
    );
  }
  return lines;
}

export function updateLines(version: string | null, ready: string | null, check: UpdateCheck | null): string[] {
  const lines: string[] = [];
  if (version) lines.push(t("Whimbrel {version}", { version }));
  if (ready) {
    lines.push(t("Version {version} is ready. Restart to install it.", { version: ready }));
    return lines;
  }
  switch (check?.state) {
    case "checking":
      lines.push(t("Checking…"));
      break;
    case "downloading":
      lines.push(t("Downloading v{version}…", { version: check.version }));
      break;
    case "upToDate":
      lines.push(t("Up to date."));
      break;
    case "failed":
      lines.push(t("Couldn't check for updates. Try again later."));
      break;
    case "ready":
      lines.push(t("Version {version} is ready. Restart to install it.", { version: check.version }));
      break;
  }
  return lines;
}

export function routineLines(host: Pick<HostView, "selected" | "options"> | null, lastRun: Pick<RunView, "routineName" | "ranAtUnixMs"> | null, nowMs: number, locale: string): string[] {
  const lines: string[] = [];
  if (host) {
    const chosen = host.selected ? host.options.find((o) => o.id === host.selected) : undefined;
    lines.push(chosen ? t("Routines run on {device}.", { device: chosen.label }) : t("Routines run on any device."));
  }
  if (lastRun) lines.push(t("Last run: {name}, {when}", { name: lastRun.routineName, when: relativeTime(lastRun.ranAtUnixMs, nowMs, locale) }));
  return lines;
}
