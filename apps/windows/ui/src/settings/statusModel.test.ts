import { describe, expect, it } from "vitest";
import type { AiSettings, SyncView } from "../generated/types";
import { aiLines, backlogStalled, BACKLOG_STALLED_MS, diskSize, relativeTime, routineLines, storageLines, syncLines, updateLines } from "./statusModel";

const NOW = Date.UTC(2026, 10, 7, 12, 0);
const MIN = 60_000;

const ready = (over: Partial<SyncView> = {}): SyncView => ({ deviceId: "me", state: "ready", ...over });

describe("relativeTime", () => {
  it("names the time the way the Mac's relative format does", () => {
    expect(relativeTime(NOW - 5 * MIN, NOW, "en")).toBe("5 minutes ago");
    expect(relativeTime(NOW - 2 * 60 * MIN, NOW, "en")).toBe("2 hours ago");
    expect(relativeTime(NOW - 24 * 60 * MIN, NOW, "en")).toBe("yesterday");
    expect(relativeTime(NOW - 10_000, NOW, "en")).toBe("10 seconds ago");
  });
});

describe("syncLines", () => {
  it("says sync is off before sign-in, and setting up on the way", () => {
    expect(syncLines(ready({ state: "notSignedIn" }), [], null, NOW, "en")).toEqual(["Sync is off."]);
    expect(syncLines(ready({ state: "waitingForApproval" }), [], null, NOW, "en")).toEqual(["Sync is setting up."]);
  });

  it("says why sync needs attention instead of calling it setting up", () => {
    const refused = ready({ state: "error", message: "This Google account isn't allowed to sign in" });
    expect(syncLines(refused, [], null, NOW, "en")).toEqual(["This Google account isn't allowed to sign in"]);
    expect(syncLines(ready({ state: "error" }), [], null, NOW, "en")).toEqual(["Needs attention"]);
  });

  it("counts approved devices and says when content and settings last synced", () => {
    const devices = [{ status: "approved" }, { status: "approved" }, { status: "pending" }];
    const lines = syncLines(ready({ contentLastSyncedMs: NOW - 5 * MIN, settingsLastSyncedMs: NOW - 2 * 60 * MIN }), devices, null, NOW, "en");
    expect(lines).toEqual(["Devices approved: 2", "Content last synced 5 minutes ago", "Settings last synced 2 hours ago"]);
  });

  it("shows a sync error in place of its last-synced line", () => {
    expect(syncLines(ready({ contentError: "offline", contentLastSyncedMs: NOW }), [], null, NOW, "en")).toEqual(["offline"]);
  });

  it("mentions unsent changes only once the oldest has waited an hour", () => {
    const fresh = { outboxPending: 3, outboxOldestMs: NOW - 10 * MIN };
    expect(syncLines(ready(), [], fresh, NOW, "en")).toEqual([]);
    const stalled = { outboxPending: 3, outboxOldestMs: NOW - BACKLOG_STALLED_MS };
    const [line] = syncLines(ready(), [], stalled, NOW, "en");
    expect(line).toMatch(/^Changes waiting to send: 3, the oldest from /);
  });

  it("treats an empty outbox as never stalled", () => {
    expect(backlogStalled(0, NOW - 10 * BACKLOG_STALLED_MS, NOW)).toBe(false);
    expect(backlogStalled(2, null, NOW)).toBe(false);
  });
});

describe("aiLines", () => {
  const ai = (over: Partial<AiSettings> = {}): AiSettings =>
    ({
      budgets: [],
      keys: [],
      prompts: [],
      providers: [{ id: "anthropic", name: "Anthropic", cheapModel: "" }],
      routing: { preferredProvider: "ollama", ollamaModel: "gemma4:12b", anthropicModel: "claude-sonnet-5-5", openaiModel: "", googleModel: "", openrouterModel: "", cheapFirst: false, localOnly: false, useLocalModel: false },
      summaryInstructions: "",
      ...over,
    }) as AiSettings;

  it("names the provider and model summaries use, by its plain name", () => {
    expect(aiLines(ai())).toEqual(["Summaries use Ollama, gemma4:12b."]);
  });

  it("warns when the chosen Ollama model isn't installed", () => {
    const lines = aiLines(ai({ ollamaWarning: { model: "gemma4:12b", fallback: "llama3.2" } }));
    expect(lines[1]).toContain("gemma4:12b isn't installed");
  });

  it("lists a paid provider only with spend or a budget this month", () => {
    const usage = { byProvider: [], last30DaysCents: 0, monthByProvider: [{ provider: "anthropic", cents: 1240 }], monthToDateCents: 1240, todayCents: 0 };
    const lines = aiLines(
      ai({
        budgets: [
          { provider: "anthropic", cents: 2500 },
          { provider: "openai", cents: null },
        ],
        usage,
      } as Partial<AiSettings>),
    );
    expect(lines).toEqual(["Summaries use Ollama, gemma4:12b.", "Anthropic: This month: $12.40 of $25.00 (49%)"]);
  });
});

describe("storageLines", () => {
  it("gives the media on disk and how long meetings are kept", () => {
    expect(storageLines({ audioBytes: 1_300_000_000, imageBytes: 34_200_000 }, { meetingDays: 90 })).toEqual([
      "Audio: 1.3 GB. Images: 34.2 MB.",
      "Meetings are kept for 90 days.",
    ]);
    expect(storageLines(null, { meetingDays: null })).toEqual(["Meetings are kept forever."]);
  });

  it("writes small sizes in MB", () => {
    expect(diskSize(0)).toBe("0.0 MB");
  });
});

describe("updateLines", () => {
  it("prefers a downloaded update over the check status", () => {
    expect(updateLines("0.9.59", "0.9.60", { state: "upToDate", version: "0.9.59" })).toEqual(["Whimbrel 0.9.59", "Version 0.9.60 is ready. Restart to install it."]);
  });

  it("says what the update check is doing", () => {
    expect(updateLines("0.9.59", null, { state: "upToDate", version: "0.9.59" })).toEqual(["Whimbrel 0.9.59", "Up to date."]);
    expect(updateLines("0.9.59", null, { state: "downloading", version: "0.9.60" })).toEqual(["Whimbrel 0.9.59", "Downloading v0.9.60…"]);
    expect(updateLines("0.9.59", null, { state: "idle" })).toEqual(["Whimbrel 0.9.59"]);
  });
});

describe("routineLines", () => {
  const host = { selected: "", options: [{ id: "pc", label: "This PC" }, { id: "mac", label: "Work MacBook" }] };

  it("says where routines run", () => {
    expect(routineLines(host, null, NOW, "en")).toEqual(["Routines run on any device."]);
    expect(routineLines({ ...host, selected: "mac" }, null, NOW, "en")).toEqual(["Routines run on Work MacBook."]);
  });

  it("names the last run and when", () => {
    expect(routineLines(null, { routineName: "Daily Brief", ranAtUnixMs: NOW - 3 * 60 * MIN }, NOW, "en")).toEqual(["Last run: Daily Brief, 3 hours ago"]);
  });
});
