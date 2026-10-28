import { describe, expect, it } from "vitest";
import type { ApprovalView, HostView, RoutineKind, RoutineView, RunView } from "../generated/types";
import {
  DESCRIPTIONS,
  HOST_ANY,
  NO_APPROVALS,
  NO_OTHER_DEVICES,
  accountLine,
  actionDetail,
  actionHeadline,
  autoApproveNote,
  canApprove,
  canChooseHost,
  canToggle,
  formatRange,
  formatWhen,
  hasSchedule,
  lastRunLine,
  needsAccountPicker,
  outcomeLabel,
  runHeading,
  scheduleChanged,
  unavailableNote,
  waitingLabel,
} from "./automationsModel";

const routine = (over: Partial<RoutineView> = {}): RoutineView => ({
  id: 1,
  name: "Daily Brief",
  kind: "dailyBrief",
  schedule: "08:30",
  enabled: true,
  writes: false,
  autoApprove: false,
  unavailable: null,
  lastRun: null,
  ...over,
});

const run = (over: Partial<RunView> = {}): RunView => ({
  id: 1,
  routineId: 1,
  routineName: "Daily Brief",
  ranAtUnixMs: new Date(2026, 9, 8, 8, 31).getTime(),
  outcome: "success",
  summary: "Yesterday you shipped the Profile.",
  ...over,
});

const KINDS: RoutineKind[] = ["dailyBrief", "focusTime", "eodRecap", "weeklyDigest", "replyPriorities", "meetingPrep"];

describe("descriptions", () => {
  it("cover every routine", () => {
    for (const kind of KINDS) expect(DESCRIPTIONS[kind].length, kind).toBeGreaterThan(20);
  });

  it("say that a write routine waits for approval, never that it acts on its own", () => {
    for (const kind of ["focusTime", "eodRecap", "weeklyDigest"] as const) {
      expect(DESCRIPTIONS[kind], kind).toMatch(/until you approve it/);
    }
  });

  it("say what the model is shown wherever a routine calls one, including that a hosted model may see it", () => {
    for (const kind of ["dailyBrief", "eodRecap", "weeklyDigest"] as const) {
      expect(DESCRIPTIONS[kind], kind).toMatch(/hosted model sees it unless Local-only is on/);
    }
    expect(DESCRIPTIONS.dailyBrief).toMatch(/captures and meeting summaries/);
  });
});

describe("times and outcomes", () => {
  it("formats a time in a fixed locale", () => {
    expect(formatWhen(new Date(2026, 9, 8, 8, 31).getTime())).toBe("Oct 8, 8:31 AM");
    expect(formatWhen(new Date(2026, 9, 8, 17, 5).getTime())).toBe("Oct 8, 5:05 PM");
  });

  it("names each outcome the way a person would", () => {
    expect(outcomeLabel("success")).toBe("Completed");
    expect(outcomeLabel("failed")).toBe("Failed");
    expect(outcomeLabel("pending")).toBe("Waiting for approval");
    expect(outcomeLabel("rejected")).toBe("Rejected");
  });

  it("says when a routine last ran and how it went, or that it never has", () => {
    expect(lastRunLine(routine())).toBe("Never run");
    expect(lastRunLine(routine({ lastRun: run() }))).toBe("Last ran Oct 8, 8:31 AM: completed");
    expect(lastRunLine(routine({ lastRun: run({ outcome: "failed" }) }))).toBe("Last ran Oct 8, 8:31 AM: failed");
    expect(lastRunLine(routine({ lastRun: run({ outcome: "pending" }) }))).toMatch(/waiting for approval$/);
  });

  it("titles a history row with the routine and when", () => {
    expect(runHeading(run())).toBe("Daily Brief, Oct 8, 8:31 AM");
  });

  it("adds where a run came from only when another device made it", () => {
    expect(runHeading(run({ originLabel: "Mike's MacBook Pro" }))).toBe("Daily Brief, Oct 8, 8:31 AM · from Mike's MacBook Pro");
    expect(runHeading(run({ originLabel: null }))).toBe("Daily Brief, Oct 8, 8:31 AM");
    expect(runHeading(run({ originLabel: undefined }))).toBe("Daily Brief, Oct 8, 8:31 AM");
    expect(runHeading(run({ originLabel: "" }))).toBe("Daily Brief, Oct 8, 8:31 AM");
  });
});

describe("what the controls allow", () => {
  it("lets a routine that can run be switched either way", () => {
    expect(canToggle(routine({ enabled: true }))).toBe(true);
    expect(canToggle(routine({ enabled: false }))).toBe(true);
  });

  it("will not let a routine that cannot run here be switched on, but lets one that somehow is on be switched off", () => {
    expect(canToggle(routine({ enabled: false, unavailable: "Not available on Windows yet." }))).toBe(false);
    expect(canToggle(routine({ enabled: true, unavailable: "Not available on Windows yet." }))).toBe(true);
  });

  it("treats a missing reason the same as no reason, so a routine is never locked by an absent field", () => {
    const missing = routine({ enabled: false, unavailable: undefined });
    expect(canToggle(missing)).toBe(true);
    expect(unavailableNote(missing)).toBeNull();
  });

  it("shows why a routine is unavailable, and nothing when it is not", () => {
    expect(unavailableNote(routine({ unavailable: "Needs Gmail connected, which Windows cannot do yet." }))).toBe("Needs Gmail connected, which Windows cannot do yet.");
    expect(unavailableNote(routine())).toBeNull();
  });

  it("gives every routine a time except Meeting Prep, which runs before each meeting", () => {
    for (const kind of KINDS) expect(hasSchedule({ kind }), kind).toBe(kind !== "meetingPrep");
  });

  it("saves a time only when it is a complete, different HH:MM", () => {
    expect(scheduleChanged("08:30", "07:05")).toBe(true);
    expect(scheduleChanged("08:30", "08:30")).toBe(false);
    for (const partial of ["", "7:05", "07:0", "ab:cd", "07:05:00"]) expect(scheduleChanged("08:30", partial), partial).toBe(false);
  });
});

// --- approvals ----------------------------------------------------------------------------------

const approval = (over: Partial<ApprovalView> = {}): ApprovalView => ({
  runId: 7,
  routineName: "Focus Time",
  ranAtUnixMs: 0,
  summary: "Focus 9 to 11",
  action: { kind: "createCalendarEvent", title: "Focus", startUnixMs: new Date(2026, 9, 8, 9, 0).getTime(), endUnixMs: new Date(2026, 9, 8, 11, 0).getTime() },
  accounts: ["me@example.com"],
  account: "me@example.com",
  blocked: null,
  ...over,
});

describe("what an approval says it will do", () => {
  it("names a calendar event and when it is", () => {
    const a = approval().action!;
    expect(actionHeadline(a)).toBe("Add “Focus” to your calendar");
    expect(actionDetail(a)).toBe("Thu, Oct 8, 9:00 AM to 11:00 AM");
  });

  it("names an email's recipient and subject, and shows the whole body", () => {
    const a = { kind: "sendEmail" as const, to: "me@example.com", subject: "Your day", body: "You shipped.\n\nThanks." };
    expect(actionHeadline(a)).toBe("Send “Your day” to me@example.com");
    expect(actionDetail(a)).toBe("You shipped.\n\nThanks.");
  });

  it("gives both days when an event crosses midnight", () => {
    const start = new Date(2026, 9, 8, 23, 0).getTime();
    const end = new Date(2026, 9, 9, 1, 0).getTime();
    expect(formatRange(start, end)).toBe("Thu, Oct 8, 11:00 PM to Fri, Oct 9, 1:00 AM");
  });
});

describe("when Approve is allowed", () => {
  it("allows a readable draft that has an account", () => {
    expect(canApprove(approval())).toBe(true);
  });

  it("does not allow a draft with no account to do it with, and says why", () => {
    const blocked = approval({ account: null, accounts: [], blocked: "Connect a Google Calendar account in Settings to approve this." });
    expect(canApprove(blocked)).toBe(false);
    expect(accountLine(blocked)).toBe("Connect a Google Calendar account in Settings to approve this.");
  });

  it("does not allow a draft that could not be read, and says it can still be rejected", () => {
    const unreadable = approval({ action: null });
    expect(canApprove(unreadable)).toBe(false);
    expect(accountLine(unreadable)).toMatch(/can reject it/);
  });

  it("treats missing fields like absent ones, so an approval is never locked by an omitted field", () => {
    const loose = approval({ blocked: undefined });
    expect(canApprove(loose)).toBe(true);
  });

  it("says which account will be used", () => {
    expect(accountLine(approval())).toBe("Using me@example.com");
  });

  it("offers a choice of account only when there is one to make", () => {
    expect(needsAccountPicker(approval({ accounts: ["a@x.com"] }))).toBe(false);
    expect(needsAccountPicker(approval({ accounts: ["a@x.com", "b@x.com"] }))).toBe(true);
    expect(needsAccountPicker(approval({ accounts: [] }))).toBe(false);
  });
});

describe("the waiting count and auto-approve wording", () => {
  it("says how many are waiting", () => {
    expect(waitingLabel(1)).toBe("1 waiting for approval");
    expect(waitingLabel(3)).toBe("3 waiting for approval");
    expect(NO_APPROVALS).toMatch(/Nothing is waiting/);
  });

  it("says plainly what switching auto-approve on means, per routine", () => {
    expect(autoApproveNote("focusTime")).toBe("When on, this routine adds the event to your calendar without asking you first. You can still see what it did in History.");
    for (const kind of ["eodRecap", "weeklyDigest"] as const) expect(autoApproveNote(kind), kind).toMatch(/sends the email without asking you first/);
  });
});

// --- which device runs routines -------------------------------------------------------------------

const host = (over: Partial<HostView> = {}): HostView => ({ primaryDeviceId: "", selected: "", options: [], caption: null, ...over });

describe("the Routines run on picker", () => {
  it("starts with the Mac's wording for the default", () => {
    expect(HOST_ANY).toBe("Any device (default)");
  });

  it("offers a choice only when there is another device besides this PC", () => {
    expect(canChooseHost(host({ options: [] }))).toBe(false);
    expect(canChooseHost(host({ options: [{ id: "me", label: "This PC" }] }))).toBe(false);
    expect(canChooseHost(host({ options: [{ id: "me", label: "This PC" }, { id: "mac", label: "MacBook" }] }))).toBe(true);
  });

  it("says when there is nobody else to choose", () => {
    expect(NO_OTHER_DEVICES).toMatch(/other devices appear here once sync is set up/);
  });
});
