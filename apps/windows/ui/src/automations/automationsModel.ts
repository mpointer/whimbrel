import { t } from "../l10n";
import type { ActionView, ApprovalView, HostView, ModelOptionView, RoutineKind, RoutineView, RunOutcome, RunView } from "../generated/types";

// What the Automations tab says and decides, apart from the component.

/** What a routine does, and, where it calls a model, what that model is shown. Whole sentences per
 *  routine, so each language words the hosted-model note in context. */
export function description(kind: RoutineKind): string {
  switch (kind) {
    case "dailyBrief":
      return t("A morning summary of yesterday, and a look at today's calendar. It reads your captures and meeting summaries. Uses the AI you chose in Settings, so a hosted model sees it unless Local-only is on.");
    case "focusTime":
      return t("Proposes a block of focus time on your calendar. Nothing is added until you approve it.");
    case "eodRecap":
      return t("Emails you a recap of your day. It reads your captures and meeting summaries. Nothing is sent until you approve it. Uses the AI you chose in Settings, so a hosted model sees it unless Local-only is on.");
    case "weeklyDigest":
      return t("Emails you a digest of your week. Nothing is sent until you approve it. Uses the AI you chose in Settings, so a hosted model sees it unless Local-only is on.");
    case "replyPriorities":
      return t("Ranks the email threads that are waiting for a reply from you.");
    case "meetingPrep":
      return t("A short brief shortly before each calendar meeting.");
  }
}

export const noRoutines = (): string => t("Your routines appear a few seconds after Whimbrel starts.");
export const noRuns = (): string => t("Nothing has run yet.");

const WHEN: Intl.DateTimeFormatOptions = { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" };

/** In the UI's locale; tests pass a fixed one, so the same text on every PC. */
export const formatWhen = (unixMs: number, locale: string): string => new Date(unixMs).toLocaleString(locale, WHEN);

export const outcomeLabel = (outcome: RunOutcome): string =>
  ({ success: t("Completed"), failed: t("Failed"), pending: t("Waiting for approval"), rejected: t("Rejected") })[outcome];

/** "Never run", or when it last ran and how it went. */
export function lastRunLine(routine: Pick<RoutineView, "lastRun">, locale: string): string {
  const run = routine.lastRun;
  if (!run) return t("Never run");
  const when = formatWhen(run.ranAtUnixMs, locale);
  switch (run.outcome) {
    case "success":
      return t("Last ran {when}: completed", { when });
    case "failed":
      return t("Last ran {when}: failed", { when });
    case "pending":
      return t("Last ran {when}: waiting for approval", { when });
    case "rejected":
      return t("Last ran {when}: rejected", { when });
  }
}

/** Whether the on/off switch may be used. A routine that cannot run here can still be switched off. */
export const canToggle = (routine: Pick<RoutineView, "enabled" | "unavailable">): boolean =>
  routine.unavailable == null || routine.enabled;

/** The Model picker's current value: the saved preference when it is still offered, else Default. */
export const modelPrefValue = (routine: Pick<RoutineView, "modelPref">, options: ModelOptionView[]): string =>
  options.some((o) => o.value === routine.modelPref) ? routine.modelPref : "default";

/** Whether a routine shows the Model picker: it calls a model through its own preference. */
export const hasModelPicker = (routine: Pick<RoutineView, "callsModel" | "unavailable">): boolean =>
  routine.callsModel && routine.unavailable == null;

export const modelPickerNote = (): string =>
  t("Which model this routine runs on. Model usage in Settings shows what each run used, so a routine set to the cheapest model can be judged on a week of output.");

/** Meeting Prep runs before each meeting, so it has no time of day. */
export const hasSchedule = (routine: Pick<RoutineView, "kind">): boolean => routine.kind !== "meetingPrep";

/** What to say about a routine that cannot run yet, or null when it can. */
export const unavailableNote = (routine: Pick<RoutineView, "unavailable">): string | null => routine.unavailable ?? null;

/** A history row's one-line heading, ending " · from <device>" when another device made the run. */
export function runHeading(run: Pick<RunView, "routineName" | "ranAtUnixMs" | "originLabel">, locale: string): string {
  const name = run.routineName;
  const when = formatWhen(run.ranAtUnixMs, locale);
  return run.originLabel ? t("{name}, {when} · from {device}", { name, when, device: run.originLabel }) : t("{name}, {when}", { name, when });
}

/** Whether the schedule box holds something worth saving: a changed, complete `HH:MM`. */
export const scheduleChanged = (saved: string, typed: string): boolean => /^\d{2}:\d{2}$/.test(typed) && typed !== saved;

// --- approvals ----------------------------------------------------------------------------------

export const noApprovals = (): string => t("Nothing is waiting for your approval.");

const DAY: Intl.DateTimeFormatOptions = { weekday: "short", month: "short", day: "numeric" };
const CLOCK: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };

/** "Thu, Oct 8, 9:00 AM to 11:00 AM", or with both days when it crosses midnight. */
export function formatRange(startMs: number, endMs: number, locale: string): string {
  const start = new Date(startMs);
  const end = new Date(endMs);
  const startDay = start.toLocaleDateString(locale, DAY);
  const startClock = start.toLocaleTimeString(locale, CLOCK);
  const endClock = end.toLocaleTimeString(locale, CLOCK);
  if (start.toDateString() === end.toDateString()) return t("{day}, {start} to {end}", { day: startDay, start: startClock, end: endClock });
  return t("{startDay}, {start} to {endDay}, {end}", { startDay, start: startClock, endDay: end.toLocaleDateString(locale, DAY), end: endClock });
}

/** What approving would do, in one line. */
export const actionHeadline = (action: ActionView): string =>
  action.kind === "createCalendarEvent"
    ? t("Add “{title}” to your calendar", { title: action.title })
    : t("Send “{subject}” to {to}", { subject: action.subject, to: action.to });

/** The rest of what the user is agreeing to: when the event is, or the whole email. */
export const actionDetail = (action: ActionView, locale: string): string =>
  action.kind === "createCalendarEvent" ? formatRange(action.startUnixMs, action.endUnixMs, locale) : action.body;

/** Approve works only for a draft that could be read and that has an account to be done with. */
export const canApprove = (approval: Pick<ApprovalView, "action" | "blocked">): boolean => approval.action != null && approval.blocked == null;

/** Which account it will use, or why it cannot be done yet. */
export function accountLine(approval: Pick<ApprovalView, "account" | "blocked" | "action">): string | null {
  if (approval.action == null) return t("This draft couldn't be read. You can reject it.");
  if (approval.blocked != null) return approval.blocked;
  return approval.account ? t("Using {account}", { account: approval.account }) : null;
}

/** Only with more than one account is there a choice to make. */
export const needsAccountPicker = (approval: Pick<ApprovalView, "accounts">): boolean => approval.accounts.length > 1;

export const waitingLabel = (count: number): string => t("{count} waiting for approval", { count });

/** What switching auto-approve on means, said next to the switch. */
export function autoApproveNote(kind: RoutineKind): string {
  return kind === "focusTime"
    ? t("When on, this routine adds the event to your calendar without asking you first. You can still see what it did in History.")
    : t("When on, this routine sends the email without asking you first. You can still see what it did in History.");
}

// --- which device runs routines -------------------------------------------------------------------

/** The picker's first option, which is also what an unset choice means. */
export const hostAny = (): string => t("Any device (default)");

/** Shown instead of a choice while there are no other devices to choose between. */
export const noOtherDevices = (): string => t("Your other devices appear here once sync is set up and they are approved.");

/**
 * Whether routines are set to run on another device. The backend only captions a chosen device
 * that isn't this PC, so a selection with a caption is one.
 */
export const runsElsewhere = (view: Pick<HostView, "selected" | "caption"> | null): boolean =>
  !!view && view.selected !== "" && view.caption != null;

/**
 * The Routines card's opening line. "Runs on this PC" would contradict a choice of another device;
 * then the picker's caption says where they run, and this keeps only the approval rule.
 */
export const routinesIntro = (view: Pick<HostView, "selected" | "caption"> | null): string =>
  runsElsewhere(view)
    ? t("A routine that would send or add something waits for your approval first, unless you turn on approving it automatically for that routine.")
    : t("Routines run on this PC while Whimbrel is running. A routine that would send or add something waits for your approval first, unless you turn on approving it automatically for that routine.");

/** The picker can only choose between devices when there is more than this PC. */
export const canChooseHost = (view: Pick<HostView, "options">): boolean => view.options.length >= 2;

/** How often, and for how long after the page opens, the picker re-checks while it has nothing to choose. */
export const HOST_FAST_POLL_MS = 3_000;
export const HOST_FAST_POLL_FOR_MS = 60_000;

/**
 * Whether to re-read the picker soon: there is nothing to choose between yet (the device list
 * may still be arriving after launch or sign-in) and the page opened under a minute ago.
 */
export const fastHostPoll = (view: Pick<HostView, "options"> | null, openForMs: number): boolean =>
  openForMs < HOST_FAST_POLL_FOR_MS && (view === null || !canChooseHost(view));
