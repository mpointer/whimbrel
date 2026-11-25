import { describe, expect, it } from "vitest";
import type { AiSettings, SyncView } from "../../../generated/types";
import { joinLines, statusRows, storageRedesignLines, type StatusInput } from "./StatusSectionModel";

const NOW = Date.UTC(2026, 10, 7, 12, 0);
const MIN = 60_000;

const ready = (over: Partial<SyncView> = {}): SyncView => ({ deviceId: "me", state: "ready", deviceLimit: false, planUnknown: false, ...over });

const input = (over: Partial<StatusInput> = {}): StatusInput => ({
  sync: ready({ contentLastSyncedMs: NOW - 5 * MIN }),
  devices: [{ status: "approved" }],
  status: { audioBytes: 34_200_000, imageBytes: 1_300_000_000, outboxPending: 0 } as StatusInput["status"],
  ai: null,
  retention: { meetingDays: null } as StatusInput["retention"],
  version: "1.4.0",
  ready: null,
  check: { state: "upToDate", version: "1.4.0" },
  host: null,
  lastRun: null,
  whimbrel: null,
  now: NOW,
  locale: "en",
  ...over,
});

const BANNED = [/memory database/i, /activity capture/i, /capture & privacy/i, /ai & routing/i, /never delete/i, /keep full quality forever/i, /\bcapture\b/i];

describe("the status rows", () => {
  it("are Sync, AI, Storage, Updates, Routines, and each opens where it is changed", () => {
    const rows = statusRows(input());
    expect(rows.map((r) => [r.key, r.target])).toEqual([
      ["sync", "devices"],
      ["ai", "ai"],
      ["storage", "capture"],
      ["updates", "general"],
      ["routines", "automations"],
    ]);
    expect(rows.map((r) => r.title)).toEqual(["Sync", "AI", "Storage", "Updates", "Routines"]);
  });

  it("add the Plan row, between Sync and AI, once the plan is known", () => {
    const sync = ready({ plan: { plan: "free", untilUnixMs: null, interval: null, billing: true, plusAvailable: false } });
    const rows = statusRows(input({ sync }));
    expect(rows.map((r) => r.key)).toEqual(["sync", "plan", "ai", "storage", "updates", "routines"]);
    expect(rows[1].target).toBe("plan");
    expect(rows[1].lines.length).toBeGreaterThan(0);
  });

  it("keep the old page's data: sync lines, versions and the update state", () => {
    const rows = statusRows(input());
    expect(rows[0].lines).toEqual(["Devices approved: 1", "Content last synced 5 minutes ago"]);
    expect(rows.find((r) => r.key === "updates")!.lines).toEqual(["Whimbrel 1.4.0", "Up to date."]);
  });

  it("say the AI choice by name", () => {
    const ai = {
      routing: { preferredProvider: "ollama", ollamaModel: "llama3" },
      providers: [{ id: "ollama", name: "Ollama" }],
      budgets: [],
      usage: { rows: [] },
      modelWarnings: [],
    } as unknown as AiSettings;
    expect(statusRows(input({ ai })).find((r) => r.key === "ai")!.lines[0]).toBe("Summaries use Ollama, llama3.");
  });
});

describe("the redesign wording", () => {
  it("names retention as Meetings and recordings, with Keep forever for no limit", () => {
    expect(storageRedesignLines(null, { meetingDays: null })).toEqual(["Meetings and recordings: Keep forever."]);
    expect(storageRedesignLines(null, { meetingDays: 30 })).toEqual(["Meetings and recordings are kept for 30 days."]);
    expect(storageRedesignLines({ audioBytes: 34_200_000, imageBytes: 1_300_000_000 }, null)).toEqual(["Audio: 34.2 MB. Images: 1.3 GB."]);
  });

  it("shows none of the retired names in any row", () => {
    const text = statusRows(input()).flatMap((r) => [r.title, joinLines(r.lines) ?? ""]).join(" ");
    for (const banned of BANNED) expect(text).not.toMatch(banned);
  });

  it("shows a routine's name and a server's message as they are, never rewritten", () => {
    const text = joinLines(["Last run: Capture and privacy audit, 5 minutes ago", "The memory database is open."]);
    expect(text).toBe("Last run: Capture and privacy audit, 5 minutes ago. The memory database is open.");
    const rows = statusRows(input({ lastRun: { routineName: "Capture and privacy audit", ranAtUnixMs: 1 } as StatusInput["lastRun"] }));
    expect(joinLines(rows.find((r) => r.key === "routines")!.lines)).toContain("Capture and privacy audit");
  });

  it("joins lines into sentences and says nothing for no lines", () => {
    expect(joinLines(["Devices approved: 2", "Up to date."])).toBe("Devices approved: 2. Up to date.");
    expect(joinLines([])).toBeUndefined();
  });
});
