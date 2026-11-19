import { t } from "../l10n";
import type { DeviceView, RoutineKind, RunOutcome } from "../generated/types";

// "Run in the cloud" in the Automations tab, and Whimbrel Cloud in Settings → Sync & Devices
// (docs/superpowers/specs/2026-11-15-cloud-routine-runner-design.md). The words and rules live here, apart from
// the components, as the Mac keeps them in CloudRoutineModel.swift (`CloudRoutineCopy`, `CloudFreshness`).
//
// The shell passes these shapes as it builds them (`commands::routines::CloudRoutinesView`,
// `syncing::controller::RunnerRegistration`). They are not in the generated schema, so they are written by hand
// here, in the shell's snake_case.

/** What core says about one template and the cloud, and this PC's settings for it. */
export interface CloudRoutineInfo {
  /** The stored template name: `daily_brief`, `eod_recap`, ... */
  template: string;
  /** Whether core lets it run in the cloud (`routines::cloud_eligible`). */
  eligible: boolean;
  /** The row types the runner may read for it (`routines::grant_for`). */
  grant: string[];
  /** Those row types in plain words, as a list ("meetings, transcripts and client settings"). */
  grant_list: string;
  /** Core's own sentence about the grant. */
  grant_sentence: string;
  /** The Google Calendar line for a routine that reads calendar events; empty otherwise. */
  google_calendar_limit: string;
  /** Set to run in the cloud. */
  cloud: boolean;
  /** The ready-by time ("HH:MM"), once chosen here or on another device. */
  ready_by: string | null;
}

/** `routines_cloud`: what the toggles need. */
export interface CloudRoutinesView {
  routines: CloudRoutineInfo[];
  /** The runner's last round (unix ms), from its synced heartbeat. */
  runner_last_run_ms: number | null;
  sync_ready: boolean;
  local_only: boolean;
  /** A hosted provider's key is saved here with "Sync to devices" on. */
  has_synced_key: boolean;
  /** The account's devices as last fetched; empty until the first fetch. */
  devices: DeviceView[];
  /** The hidden switch (`cloudRoutines.enabled` in settings.json, off unless set by hand) that shows the cloud UI
   *  before a Whimbrel Cloud is on the account. */
  cloud_ui_enabled: boolean;
}

/** `sync_register_runner`'s outcome. */
export type RunnerRegistration =
  | { status: "added"; provision_bundle: string; approver_public_key: string; worker_url: string }
  | { status: "not_a_key" }
  | { status: "not_available" }
  | { status: "already_added" }
  | { status: "sync_off" }
  | { status: "failed"; message: string };

/** The stored template name of a routine the tab lists. */
export function templateOf(kind: RoutineKind): string {
  switch (kind) {
    case "dailyBrief":
      return "daily_brief";
    case "focusTime":
      return "focus_time";
    case "eodRecap":
      return "eod_recap";
    case "weeklyDigest":
      return "weekly_digest";
    case "replyPriorities":
      return "reply_priorities";
    case "meetingPrep":
      return "meeting_prep";
  }
}

/** Core's answer for a routine, if the view has it. */
export const infoFor = (view: CloudRoutinesView | null, kind: RoutineKind): CloudRoutineInfo | null =>
  view?.routines.find((r) => r.template === templateOf(kind)) ?? null;

// --- the toggle -----------------------------------------------------------------------------------

/** Only the templates core lets run in the cloud get the toggle (never Reply Priorities, Focus Time or Away
 *  follow-up). */
export const showsToggle = (info: CloudRoutineInfo | null | undefined): boolean => info?.eligible === true;

export const toggleLabel = (): string => t("Run in the cloud");

// --- who sees any of it (author-only phase, docs/RELEASING.md "Cloud routine runner gate") ----------------------

/** An approved Whimbrel Cloud is on the account. */
export const runnerOnAccount = (devices: Pick<DeviceView, "platform" | "status">[]): boolean => !runnerMissing(devices);

/** Whether a routine shows "Run in the cloud" (with its ready-by time and Updated/Late line): with the hidden switch
 *  on, or once a Whimbrel Cloud is on the account. A routine already set to run in the cloud keeps it too, so it can
 *  always be turned off. With the switch off, no runner and nothing set, nothing new shows. */
export const showsCloudSection = (view: Pick<CloudRoutinesView, "cloud_ui_enabled" | "devices">, on: boolean): boolean =>
  view.cloud_ui_enabled || on || runnerOnAccount(view.devices);

/** "Add Whimbrel Cloud…" only with the hidden switch on. */
export const showsAddRunner = (view: Pick<CloudRoutinesView, "cloud_ui_enabled"> | null): boolean => view?.cloud_ui_enabled === true;

/** The binding sentence, with core's list of what the grant reads, then core's Google Calendar line where it
 *  applies. The host's secret store holds the key at all times, not only during a run, so the sentence says that
 *  first. */
export function toggleCopy(info: Pick<CloudRoutineInfo, "grant_list" | "google_calendar_limit">): string {
  const sentence = t("Whimbrel Cloud holds your sync key. While this runs it can read your {list}. It keeps nothing after the run.", {
    list: info.grant_list,
  });
  return info.google_calendar_limit ? `${sentence} ${info.google_calendar_limit}` : sentence;
}

/** What the cloud cannot undo or see. */
export const disclosure = (): string =>
  t(
    "Results already made are not recalled if you later delete a client. On the cloud, exclusion matches by name, alias and contact address; items you filed by hand are not known to the cloud.",
  );

export const readyByLabel = (): string => t("Ready by");

/** The ready-by time syncs for the runner, which starts early enough to be ready by it; here it decides when a
 *  run is late and, with no result, when this PC stands in. */
export const readyByNote = (): string =>
  t(
    "Whimbrel Cloud starts early enough to have the result ready by this time. If no result has arrived by then, Automations shows the run as late, with the last result.",
  );

/** Exactly "HH:MM" on a 24-hour clock. */
export function isHhmm(text: string): boolean {
  const m = /^(\d{2}):(\d{2})$/.exec(text);
  return m !== null && Number(m[1]) <= 23 && Number(m[2]) <= 59;
}

/** Whether a routine can be late: only the daily ones that are on. The Weekly Digest runs one day a week and
 *  Meeting Prep runs before each meeting, so they only show "Updated". */
export const canBeLate = (template: string, enabled: boolean): boolean =>
  enabled && (template === "daily_brief" || template === "eod_recap");

/** The ready-by time when none is chosen: an hour after the routine's own time, since the result then has to
 *  sync. Null when the time does not parse. */
export function defaultReadyBy(schedule: string): string | null {
  if (!isHhmm(schedule)) return null;
  const [h, m] = schedule.split(":").map(Number);
  const minutes = Math.min(h * 60 + m + 60, 23 * 60 + 59);
  return `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
}

/** Why "Run in the cloud" cannot be turned on, or null when it can, in this order: sync off, the routine off,
 *  local-only, no synced key. The runner reaches this PC's settings only through sync; turning it on for a routine
 *  that is off here would sync `enabled=false` and switch the routine off on every device; and the runner has no
 *  model of its own, only a synced provider key. */
export function blockedReason(syncReady: boolean, routineEnabled: boolean, localOnly: boolean, hasSyncedKey: boolean): string | null {
  if (!syncReady) return t("Turn on sync in Settings → Sync & Devices first: Whimbrel Cloud gets this routine's settings through sync.");
  if (!routineEnabled) return t("Turn this routine on first.");
  if (localOnly) return t("Local-only mode is on, so Whimbrel Cloud can't use a hosted model for this routine.");
  if (!hasSyncedKey) {
    return t("Whimbrel Cloud uses a provider key you sync. First turn on Sync to devices for a hosted provider's key in Settings → AI.");
  }
  return null;
}

/** Whimbrel Cloud: the account's device with platform `cloud`. */
export const isRunner = (device: Pick<DeviceView, "platform">): boolean => device.platform === "cloud";

/** No approved Whimbrel Cloud is on the account. An empty list (sync off, or not loaded yet) counts as none: the
 *  note then says what happens rather than staying silent. A removed one counts as none too. */
export const runnerMissing = (devices: Pick<DeviceView, "platform" | "status">[]): boolean =>
  !devices.some((d) => isRunner(d) && d.status === "approved");

/** The computer stands in 3 hours after the ready-by time, but by 23:30 that day; with no ready-by time, 3 hours after
 *  the routine's own time (core `tick_cloud`, `DEVICE_STAND_IN_CAP_HHMM`). */
export const noRunner = (): string =>
  t(
    "Whimbrel Cloud is not on your account yet, so this routine runs on your computer 3 hours after its ready-by time or at 23:30, whichever is earlier (3 hours after its own time if it has no ready-by time). Add Whimbrel Cloud in Settings → Sync & Devices.",
  );

/** Meeting Prep has no ready-by time: the computer prepares a meeting itself 5 minutes before it starts when no brief
 *  has synced in (core `PREP_CLOUD_DEVICE_BEFORE_START_MS`). */
export const noRunnerPrep = (): string =>
  t("Whimbrel Cloud is not on your account yet, so your computer prepares each meeting 5 minutes before it starts. Add Whimbrel Cloud in Settings → Sync & Devices.");

/** The one line under the toggle that says why it is off-limits or why the routine will not run in the cloud.
 *  A blocked reason (sync off, local-only, no synced key) wins whether the toggle is on or off: while on, the
 *  routine then runs on this computer after the grace, and the reason says why. Only with nothing blocking does an
 *  on toggle say that no Whimbrel Cloud is on the account. */
export function statusNote(on: boolean, blocked: string | null, missing: boolean, template = ""): string | null {
  if (blocked !== null) return blocked;
  if (!(on && missing)) return null;
  return template === "meeting_prep" ? noRunnerPrep() : noRunner();
}

/** Turning the toggle on needs nothing to block it; turning it off is always allowed. */
export const toggleDisabled = (on: boolean, blocked: string | null): boolean => !on && blocked !== null;

// --- how fresh the last result is -----------------------------------------------------------------

/** One routine run, as the freshness line needs it. */
export interface CloudRunStamp {
  ranAtUnixMs: number;
  outcome: RunOutcome;
}

export type CloudFreshness =
  /** No result yet, and not late. */
  | { kind: "none" }
  /** The newest good result. */
  | { kind: "updated"; at: number }
  /** Today's run is missing past the ready-by time: the last good result (if any) shows instead. */
  | { kind: "late"; lastGood: number | null };

const sameDay = (a: Date, b: Date): boolean =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/** The ready-by "HH:MM" on `day`'s local date, or null when it does not parse. */
export function readyByTime(hhmm: string, day: Date): Date | null {
  if (!isHhmm(hhmm)) return null;
  const [h, m] = hhmm.split(":").map(Number);
  return new Date(day.getFullYear(), day.getMonth(), day.getDate(), h, m, 0, 0);
}

/** The freshness of a routine whose runs are `runs` (any order), at `now`. A failed run (one that did not finish,
 *  or one the runner left for this computer) is never a result. `readyBy` "" means the routine cannot be late. */
export function freshness(runs: CloudRunStamp[], readyBy: string, now: Date): CloudFreshness {
  const good = runs.filter((r) => r.outcome !== "failed").map((r) => r.ranAtUnixMs);
  const lastGood = good.length > 0 ? Math.max(...good) : null;
  if (lastGood !== null && sameDay(new Date(lastGood), now)) return { kind: "updated", at: lastGood };
  const due = readyByTime(readyBy, now);
  if (due !== null && now.getTime() >= due.getTime()) return { kind: "late", lastGood };
  return lastGood !== null ? { kind: "updated", at: lastGood } : { kind: "none" };
}

/** The time alone for today ("6:58 AM"), else the day and time. */
export function when(unixMs: number, now: Date, locale: string): string {
  const at = new Date(unixMs);
  return sameDay(at, now)
    ? at.toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" })
    : at.toLocaleString(locale, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

export function freshnessLabel(f: CloudFreshness, now: Date, locale: string): string | null {
  switch (f.kind) {
    case "none":
      return null;
    case "updated":
      return t("Updated {time}", { time: when(f.at, now, locale) });
    case "late":
      return f.lastGood === null
        ? t("Late: no result yet")
        : t("Late: showing the last result from {time}", { time: when(f.lastGood, now, locale) });
  }
}

/** The freshness line for one routine, from the runs the tab has (all routines', newest first or not). Only an
 *  enabled daily routine can be late, against its ready-by time (chosen, else the default). */
export function routineFreshnessLabel(
  routine: { id: number; schedule: string; enabled: boolean },
  info: Pick<CloudRoutineInfo, "template" | "ready_by">,
  runs: (CloudRunStamp & { routineId: number })[],
  now: Date,
  locale: string,
): string | null {
  const readyBy = canBeLate(info.template, routine.enabled) ? shownReadyBy(routine.schedule, info.ready_by) : "";
  return freshnessLabel(freshness(runs.filter((r) => r.routineId === routine.id), readyBy, now), now, locale);
}

/** The ready-by time the picker shows: the chosen one, else an hour after the routine's time. */
export const shownReadyBy = (schedule: string, chosen: string | null): string => chosen ?? defaultReadyBy(schedule) ?? schedule;

// --- Settings → Sync & Devices --------------------------------------------------------------------

/** The runner's name on the account. */
export const runnerName = (): string => t("Whimbrel Cloud");
export const addRunner = (): string => t("Add Whimbrel Cloud…");
export const addRunnerPrompt = (): string =>
  t("On the host that will run your routines, run whimbrel-cloud-runner init and paste the public key it prints here.");
export const publicKeyLabel = (): string => t("Public key");
export const removeRunnerTitle = (): string => t("Remove Whimbrel Cloud?");
/** Honest about what removal does: relay access ends; the copy of the sync key it held is not recalled. */
export const removeRunnerMessage = (): string =>
  t(
    "Whimbrel Cloud loses its access to your sync at once. Routines still set to run in the cloud then run on your computer 3 hours after their ready-by time or at 23:30, whichever is earlier (3 hours after their own time if they have no ready-by time, and Meeting Prep 5 minutes before each meeting), until you turn Run in the cloud off. To use Whimbrel Cloud again, run its setup again and add it again. Like a lost laptop, the copy of your sync key it held is not recalled.",
  );
/** What the host's secret store needs from this screen, by name (`whimbrel-cloud-runner` reads these). */
export const provisionInstructions = (): string =>
  t(
    "Add these to Whimbrel Cloud's secret store, next to RUNNER_DEVICE_SECRET_KEY from its setup. The provision value carries your sync key: paste it only there. It is shown once.",
  );

/** "Last ran <day and time>" for the runner's row, from its heartbeat. */
export const lastRanLine = (unixMs: number, locale: string): string =>
  t("Last ran {time}", {
    time: new Date(unixMs).toLocaleString(locale, { month: "short", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit" }),
  });

/** Why "Add Whimbrel Cloud" did not register the runner, in words; null when it did. */
export function registrationError(r: RunnerRegistration): string | null {
  switch (r.status) {
    case "added":
      return null;
    case "not_a_key":
      return t("That is not a Whimbrel Cloud key. Paste the one line that whimbrel-cloud-runner init printed.");
    case "not_available":
      return t("Whimbrel Cloud is not available for this account yet.");
    case "already_added":
      return t("This Whimbrel Cloud is already on your account. To add it again, remove it from the list, run its setup again and paste the new key.");
    case "sync_off":
      return t("Turn on sync on this PC first.");
    case "failed":
      return r.message;
  }
}

/** The values the runner's host needs, by the names its secret store uses. Shown once; never stored. */
export function provisionRows(r: Extract<RunnerRegistration, { status: "added" }>): [string, string][] {
  return [
    ["RUNNER_PROVISION", r.provision_bundle],
    ["RUNNER_APPROVER_PUBLIC_KEY", r.approver_public_key],
    ["RUNNER_WORKER_URL", r.worker_url],
  ];
}
