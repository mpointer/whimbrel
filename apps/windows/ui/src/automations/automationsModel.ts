import type { ActionView, ApprovalView, HostView, ModelOptionView, RoutineKind, RoutineView, RunOutcome, RunView } from "../generated/types";

// What the Automations tab says and decides, apart from the component.

/** What a routine does, and, where it calls a model, what that model is shown. */
const SENDS_TO_MODEL = "Uses the AI you chose in Settings, so a hosted model sees it unless Local-only is on.";

export const DESCRIPTIONS: Record<RoutineKind, string> = {
  dailyBrief: `A morning summary of yesterday, and a look at today's calendar. It reads your captures and meeting summaries. ${SENDS_TO_MODEL}`,
  focusTime: "Proposes a block of focus time on your calendar. Nothing is added until you approve it.",
  eodRecap: `Emails you a recap of your day. It reads your captures and meeting summaries. Nothing is sent until you approve it. ${SENDS_TO_MODEL}`,
  weeklyDigest: `Emails you a digest of your week. Nothing is sent until you approve it. ${SENDS_TO_MODEL}`,
  replyPriorities: "Ranks the email threads that are waiting for a reply from you.",
  meetingPrep: "A short brief shortly before each calendar meeting.",
};

export const NO_ROUTINES = "Your routines appear a few seconds after Whimbrel starts.";
export const NO_RUNS = "Nothing has run yet.";

const WHEN: Intl.DateTimeFormatOptions = { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" };

/** A fixed locale, so the same text on every PC and in tests. */
export const formatWhen = (unixMs: number): string => new Date(unixMs).toLocaleString("en-US", WHEN);

export const outcomeLabel = (outcome: RunOutcome): string =>
  ({ success: "Completed", failed: "Failed", pending: "Waiting for approval", rejected: "Rejected" })[outcome];

/** "Never run", or when it last ran and how it went. */
export function lastRunLine(routine: Pick<RoutineView, "lastRun">): string {
  const run = routine.lastRun;
  if (!run) return "Never run";
  return `Last ran ${formatWhen(run.ranAtUnixMs)}: ${outcomeLabel(run.outcome).toLowerCase()}`;
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

export const MODEL_PICKER_NOTE = "Which model this routine runs on. Model usage in Settings shows what each run used, so a routine set to the cheapest model can be judged on a week of output.";

/** Meeting Prep runs before each meeting, so it has no time of day. */
export const hasSchedule = (routine: Pick<RoutineView, "kind">): boolean => routine.kind !== "meetingPrep";

/** What to say about a routine that cannot run yet, or null when it can. */
export const unavailableNote = (routine: Pick<RoutineView, "unavailable">): string | null => routine.unavailable ?? null;

/** A history row's one-line heading, ending " · from <device>" when another device made the run. */
export const runHeading = (run: Pick<RunView, "routineName" | "ranAtUnixMs" | "originLabel">): string =>
  `${run.routineName}, ${formatWhen(run.ranAtUnixMs)}` + (run.originLabel ? ` · from ${run.originLabel}` : "");

/** Whether the schedule box holds something worth saving: a changed, complete `HH:MM`. */
export const scheduleChanged = (saved: string, typed: string): boolean => /^\d{2}:\d{2}$/.test(typed) && typed !== saved;

// --- approvals ----------------------------------------------------------------------------------

export const NO_APPROVALS = "Nothing is waiting for your approval.";

const DAY: Intl.DateTimeFormatOptions = { weekday: "short", month: "short", day: "numeric" };
const CLOCK: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };

/** "Thu, Oct 8, 9:00 AM to 11:00 AM", or with both days when it crosses midnight. */
export function formatRange(startMs: number, endMs: number): string {
  const start = new Date(startMs);
  const end = new Date(endMs);
  const startDay = start.toLocaleDateString("en-US", DAY);
  const startClock = start.toLocaleTimeString("en-US", CLOCK);
  const endClock = end.toLocaleTimeString("en-US", CLOCK);
  if (start.toDateString() === end.toDateString()) return `${startDay}, ${startClock} to ${endClock}`;
  return `${startDay}, ${startClock} to ${end.toLocaleDateString("en-US", DAY)}, ${endClock}`;
}

/** What approving would do, in one line. */
export const actionHeadline = (action: ActionView): string =>
  action.kind === "createCalendarEvent" ? `Add “${action.title}” to your calendar` : `Send “${action.subject}” to ${action.to}`;

/** The rest of what the user is agreeing to: when the event is, or the whole email. */
export const actionDetail = (action: ActionView): string =>
  action.kind === "createCalendarEvent" ? formatRange(action.startUnixMs, action.endUnixMs) : action.body;

/** Approve works only for a draft that could be read and that has an account to be done with. */
export const canApprove = (approval: Pick<ApprovalView, "action" | "blocked">): boolean => approval.action != null && approval.blocked == null;

/** Which account it will use, or why it cannot be done yet. */
export function accountLine(approval: Pick<ApprovalView, "account" | "blocked" | "action">): string | null {
  if (approval.action == null) return "This draft couldn't be read. You can reject it.";
  if (approval.blocked != null) return approval.blocked;
  return approval.account ? `Using ${approval.account}` : null;
}

/** Only with more than one account is there a choice to make. */
export const needsAccountPicker = (approval: Pick<ApprovalView, "accounts">): boolean => approval.accounts.length > 1;

export const waitingLabel = (count: number): string => (count === 1 ? "1 waiting for approval" : `${count} waiting for approval`);

/** What switching auto-approve on means, said next to the switch. */
export function autoApproveNote(kind: RoutineKind): string {
  const doing = kind === "focusTime" ? "adds the event to your calendar" : "sends the email";
  return `When on, this routine ${doing} without asking you first. You can still see what it did in History.`;
}

// --- which device runs routines -------------------------------------------------------------------

/** The picker's first option, which is also what an unset choice means. */
export const HOST_ANY = "Any device (default)";

/** Shown instead of a choice while there are no other devices to choose between. */
export const NO_OTHER_DEVICES = "Your other devices appear here once sync is set up and they are approved.";

/** The picker can only choose between devices when there is more than this PC. */
export const canChooseHost = (view: Pick<HostView, "options">): boolean => view.options.length >= 2;
