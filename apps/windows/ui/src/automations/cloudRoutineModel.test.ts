import { describe, expect, it } from "vitest";
import type { DeviceView } from "../generated/types";
import { deviceActions } from "../settings/syncModel";
import {
  blockedReason,
  canBeLate,
  defaultReadyBy,
  disclosure,
  freshness,
  freshnessLabel,
  infoFor,
  isHhmm,
  noRunner,
  noRunnerPrep,
  provisionRows,
  readyByNote,
  runnerOnAccount,
  showsAddRunner,
  showsCloudSection,
  registrationError,
  removeRunnerMessage,
  routineFreshnessLabel,
  runnerMissing,
  showsToggle,
  shownReadyBy,
  statusNote,
  templateOf,
  toggleCopy,
  toggleDisabled,
  type CloudRoutineInfo,
  type CloudRoutinesView,
  type CloudRunStamp,
} from "./cloudRoutineModel";

// Core's words, as `routines_cloud` passes them (routines::grant_list and GOOGLE_CALENDAR_LIMIT).
const BRIEF_LIST = "meetings, calendar events, transcripts, past routine results and client settings";
const GOOGLE = "Events from your Google Calendar connection are not read by the cloud. Calendars added to your computer's Calendar app are.";

const info = (template: string, over: Partial<CloudRoutineInfo> = {}): CloudRoutineInfo => {
  const eligible = ["daily_brief", "eod_recap", "weekly_digest", "meeting_prep"].includes(template);
  return {
    template,
    eligible,
    grant: eligible ? ["meeting"] : [],
    grant_list: eligible ? BRIEF_LIST : "",
    grant_sentence: "",
    google_calendar_limit: template === "daily_brief" || template === "meeting_prep" ? GOOGLE : "",
    cloud: false,
    ready_by: null,
    ...over,
  };
};

const view = (over: Partial<CloudRoutinesView> = {}): CloudRoutinesView => ({
  routines: ["daily_brief", "focus_time", "eod_recap", "weekly_digest", "reply_priorities", "meeting_prep", "away_follow_up"].map((t) => info(t)),
  runner_last_run_ms: null,
  sync_ready: true,
  local_only: false,
  has_synced_key: true,
  devices: [],
  cloud_ui_enabled: false,
  ...over,
});

const device = (over: Partial<DeviceView> = {}): DeviceView => ({
  id: "d1",
  publicKey: "KEY",
  name: null,
  platform: null,
  status: "approved",
  thisDevice: false,
  enrolledAtUnixMs: 0,
  comparisonCode: null,
  ...over,
});

const runner = (status = "approved") => device({ id: "r", publicKey: "RUNNER", platform: "cloud", status });

describe("the toggle is shown only where core says the routine may run in the cloud", () => {
  it("is hidden for reply_priorities, focus_time and away_follow_up", () => {
    for (const t of ["reply_priorities", "focus_time", "away_follow_up"]) expect(showsToggle(info(t))).toBe(false);
    expect(showsToggle(infoFor(view(), "replyPriorities"))).toBe(false);
    expect(showsToggle(infoFor(view(), "focusTime"))).toBe(false);
    expect(showsToggle(null)).toBe(false);
  });

  it("is shown for the four read-only routines, found by their stored names", () => {
    for (const kind of ["dailyBrief", "eodRecap", "weeklyDigest", "meetingPrep"] as const) {
      expect(showsToggle(infoFor(view(), kind))).toBe(true);
    }
    expect(templateOf("eodRecap")).toBe("eod_recap");
    expect(infoFor(null, "dailyBrief")).toBeNull();
  });
});

describe("the toggle's words", () => {
  it("contain the grant list, the key-holding sentence and, where it reads calendar events, core's Google line", () => {
    const brief = toggleCopy(info("daily_brief"));
    expect(brief).toBe(
      `Whimbrel Cloud holds your sync key. While this runs it can read your ${BRIEF_LIST}. It keeps nothing after the run. ${GOOGLE}`,
    );
    const recap = toggleCopy(info("eod_recap", { grant_list: "meetings, transcripts, past routine results and client settings" }));
    expect(recap).toContain("Whimbrel Cloud holds your sync key. While this runs it can read your meetings, transcripts, past routine results and client settings.");
    expect(recap).not.toContain("Google");
  });

  it("disclose what the cloud cannot recall or see", () => {
    expect(disclosure()).toBe(
      "Results already made are not recalled if you later delete a client. On the cloud, exclusion matches by name, alias and contact address; items you filed by hand are not known to the cloud.",
    );
  });

  it("say the cloud holds the key while it runs, and removal never claims the key changes", () => {
    expect(toggleCopy(info("eod_recap"))).toContain("Whimbrel Cloud holds your sync key");
    expect(removeRunnerMessage()).toContain("the copy of your sync key it held is not recalled");
    expect(removeRunnerMessage()).not.toMatch(/rotat/i);
  });
});

describe("why the toggle cannot be turned on, and the one line under it", () => {
  it("names sync first, then the routine being off, then local-only, then the synced key", () => {
    expect(blockedReason(false, false, true, false)).toMatch(/^Turn on sync in Settings → Sync & Devices first/);
    expect(blockedReason(true, false, true, false)).toBe("Turn this routine on first.");
    expect(blockedReason(true, true, true, false)).toMatch(/^Local-only mode is on/);
    expect(blockedReason(true, true, false, false)).toMatch(/^Whimbrel Cloud uses a provider key you sync/);
    expect(blockedReason(true, true, false, true)).toBeNull();
  });

  it('the toggle is blocked with "Turn this routine on first." when the routine is off', () => {
    const blocked = blockedReason(true, false, false, true);
    expect(blocked).toBe("Turn this routine on first.");
    expect(toggleDisabled(false, blocked)).toBe(true);
    expect(statusNote(false, blocked, false)).toBe("Turn this routine on first.");
  });

  it("turning it off is always allowed", () => {
    for (const blocked of [
      blockedReason(false, true, false, true),
      blockedReason(true, false, false, true),
      blockedReason(true, true, true, true),
      blockedReason(true, true, false, false),
    ]) {
      expect(blocked).not.toBeNull();
      expect(toggleDisabled(true, blocked)).toBe(false);
    }
    expect(toggleDisabled(false, null)).toBe(false);
  });

  it("an already-on toggle with sync off shows the sync reason, not the no-runner note", () => {
    expect(statusNote(true, blockedReason(false, false, false, true), runnerMissing([]))).toMatch(/^Turn on sync/);
  });

  it("an already-on toggle whose routine was switched off shows that reason, not the no-runner note", () => {
    expect(statusNote(true, blockedReason(true, false, false, true), runnerMissing([]))).toBe("Turn this routine on first.");
  });

  it("an already-on toggle with local-only on shows the local-only reason", () => {
    expect(statusNote(true, blockedReason(true, true, true, true), true)).toMatch(/^Local-only mode is on/);
  });

  it("an already-on toggle with nothing blocking and no runner says when the computer stands in", () => {
    expect(statusNote(true, null, runnerMissing([device()]))).toBe(noRunner());
    expect(statusNote(true, null, runnerMissing([device()]), "daily_brief")).toBe(noRunner());
    expect(statusNote(true, null, runnerMissing([device()]), "meeting_prep")).toBe(noRunnerPrep());
    expect(noRunner()).toContain("runs on your computer 3 hours after its ready-by time or at 23:30, whichever is earlier");
    expect(noRunner()).toContain("3 hours after its own time if it has no ready-by time");
    expect(noRunnerPrep()).toContain("prepares each meeting 5 minutes before it starts");
    expect(removeRunnerMessage()).toContain("3 hours after their ready-by time or at 23:30, whichever is earlier");
    expect(removeRunnerMessage()).toContain("Meeting Prep 5 minutes before each meeting");
    expect(readyByNote()).toMatch(/^Whimbrel Cloud starts early enough to have the result ready by this time\./);
    for (const text of [noRunner(), noRunnerPrep(), removeRunnerMessage(), readyByNote(), toggleCopy(info("daily_brief"))]) {
      expect(text).not.toMatch(/nothing leaves your computer|never leaves|we never receive/i);
    }
  });

  it("an off toggle with nothing blocking says nothing, and neither does an on one with the runner there", () => {
    expect(statusNote(false, null, true)).toBeNull();
    expect(statusNote(true, null, runnerMissing([device(), runner()]))).toBeNull();
  });

  it("counts only an approved Whimbrel Cloud; an empty list is none", () => {
    expect(runnerMissing([])).toBe(true);
    expect(runnerMissing([runner("revoked")])).toBe(true);
    expect(runnerMissing([device({ status: "approved" })])).toBe(true);
    expect(runnerMissing([runner()])).toBe(false);
  });
});

describe("ready-by", () => {
  it("only the enabled daily routines can be late", () => {
    expect(canBeLate("daily_brief", true)).toBe(true);
    expect(canBeLate("eod_recap", true)).toBe(true);
    expect(canBeLate("daily_brief", false)).toBe(false);
    expect(canBeLate("weekly_digest", true)).toBe(false);
    expect(canBeLate("meeting_prep", true)).toBe(false);
  });

  it("defaults to an hour after the routine's time, never past 23:59", () => {
    expect(defaultReadyBy("08:30")).toBe("09:30");
    expect(defaultReadyBy("23:30")).toBe("23:59");
    expect(defaultReadyBy("8:30")).toBeNull();
    expect(shownReadyBy("08:30", null)).toBe("09:30");
    expect(shownReadyBy("08:30", "07:15")).toBe("07:15");
    expect(isHhmm("24:00")).toBe(false);
  });
});

describe("how fresh a cloud routine's result is", () => {
  // Local-time dates, so the same on every PC whatever its zone.
  const at = (day: number, h: number, m = 0) => new Date(2026, 10, day, h, m).getTime();
  const run = (ms: number, outcome: CloudRunStamp["outcome"] = "success"): CloudRunStamp => ({ ranAtUnixMs: ms, outcome });
  const now = new Date(2026, 10, 15, 10, 0);
  const locale = "en-US";

  it("today's result shows as Updated with its time", () => {
    const f = freshness([run(at(14, 7)), run(at(15, 6, 58))], "09:30", now);
    expect(f).toEqual({ kind: "updated", at: at(15, 6, 58) });
    expect(freshnessLabel(f, now, locale)).toBe("Updated 6:58 AM");
  });

  it("a late run shows the last good result with its time", () => {
    const f = freshness([run(at(14, 7, 5)), run(at(15, 7), "failed")], "09:30", now);
    expect(f).toEqual({ kind: "late", lastGood: at(14, 7, 5) });
    expect(freshnessLabel(f, now, locale)).toBe("Late: showing the last result from Nov 14, 7:05 AM");
  });

  it("late with nothing ever made says so", () => {
    expect(freshnessLabel(freshness([run(at(15, 7), "failed")], "09:30", now), now, locale)).toBe("Late: no result yet");
  });

  it("before the ready-by time it is not late yet", () => {
    expect(freshness([run(at(14, 7))], "10:30", now)).toEqual({ kind: "updated", at: at(14, 7) });
    expect(freshness([], "10:30", now)).toEqual({ kind: "none" });
    expect(freshnessLabel({ kind: "none" }, now, locale)).toBeNull();
  });

  it("a routine that cannot be late only ever shows Updated", () => {
    const routine = { id: 7, schedule: "08:30", enabled: true };
    const runs = [{ ...run(at(14, 9)), routineId: 7 }, { ...run(at(15, 9)), routineId: 8 }];
    expect(routineFreshnessLabel(routine, { template: "weekly_digest", ready_by: "09:00" }, runs, now, locale)).toBe(
      "Updated Nov 14, 9:00 AM",
    );
    expect(routineFreshnessLabel({ ...routine, enabled: false }, { template: "daily_brief", ready_by: null }, runs, now, locale)).toBe(
      "Updated Nov 14, 9:00 AM",
    );
    expect(routineFreshnessLabel(routine, { template: "daily_brief", ready_by: null }, runs, now, locale)).toBe(
      "Late: showing the last result from Nov 14, 9:00 AM",
    );
  });
});

describe("adding Whimbrel Cloud", () => {
  it("maps each refusal to its own words", () => {
    expect(registrationError({ status: "not_a_key" })).toBe(
      "That is not a Whimbrel Cloud key. Paste the one line that whimbrel-cloud-runner init printed.",
    );
    expect(registrationError({ status: "not_available" })).toBe("Whimbrel Cloud is not available for this account yet.");
    expect(registrationError({ status: "already_added" })).toBe(
      "This Whimbrel Cloud is already on your account. To add it again, remove it from the list, run its setup again and paste the new key.",
    );
    expect(registrationError({ status: "sync_off" })).toBe("Turn on sync on this PC first.");
    expect(registrationError({ status: "failed", message: "Couldn't reach the sync server." })).toBe("Couldn't reach the sync server.");
  });

  it("on success shows the three values the host needs, by name", () => {
    const added = { status: "added", provision_bundle: "BUNDLE", approver_public_key: "APPROVER", worker_url: "https://w.test" } as const;
    expect(registrationError(added)).toBeNull();
    expect(provisionRows(added)).toEqual([
      ["RUNNER_PROVISION", "BUNDLE"],
      ["RUNNER_APPROVER_PUBLIC_KEY", "APPROVER"],
      ["RUNNER_WORKER_URL", "https://w.test"],
    ]);
  });
});

describe("the hidden switch (author-only phase)", () => {
  it("off, with no runner on the account, hides the toggle and Add Whimbrel Cloud", () => {
    for (const devices of [[], [device()], [device(), runner("revoked")]]) {
      expect(showsCloudSection(view({ devices }), false)).toBe(false);
    }
    expect(showsAddRunner(view())).toBe(false);
    expect(showsAddRunner(null)).toBe(false);
  });

  it("on, shows the toggle and Add Whimbrel Cloud", () => {
    expect(showsCloudSection(view({ cloud_ui_enabled: true }), false)).toBe(true);
    expect(showsAddRunner(view({ cloud_ui_enabled: true }))).toBe(true);
  });

  it("off, with a runner on the account, shows the toggle but not Add", () => {
    const v = view({ devices: [device(), runner()] });
    expect(runnerOnAccount(v.devices)).toBe(true);
    expect(showsCloudSection(v, false)).toBe(true);
    expect(showsAddRunner(v)).toBe(false);
  });

  it("off, a runner on the list can still be removed", () => {
    expect(deviceActions(runner())).toEqual(["removeRunner"]);
    expect(deviceActions(runner("revoked"))).toEqual([]);
  });

  it("keeps the toggle for a routine already set to run in the cloud, so it can be turned off", () => {
    expect(showsCloudSection(view(), true)).toBe(true);
  });
});
