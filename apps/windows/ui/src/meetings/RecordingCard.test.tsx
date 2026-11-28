import { beforeEach, describe, expect, it, vi } from "vitest";
import { Tree } from "../testkit/tree";
import { RecordingCard } from "./RecordingCard";

// The Record now card: shown while nothing records, and off with a caption when Whimbrel runs without the memory database.
vi.mock("react", async (actual) => (await import("../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));
const h = vi.hoisted(() => ({ noStore: false, status: { state: "idle", paused: false, pausedTotalMs: 0, resuming: false } as Record<string, unknown> }));
vi.mock("../l10n", async (actual) => {
  const real = await actual<typeof import("../l10n")>();
  return { ...real, useL10n: () => ({ t: real.t, lang: "en", locale: "en-US" }) };
});
vi.mock("../launch/noStore", () => ({ useNoStore: () => h.noStore }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string) => (cmd === "meeting_recording_status" ? h.status : null),
}));

beforeEach(() => {
  h.noStore = false;
  h.status = { state: "idle", paused: false, pausedTotalMs: 0, resuming: false };
  vi.stubGlobal("setInterval", () => 0);
  vi.stubGlobal("clearInterval", () => {});
});

const show = async () => {
  const tree = new Tree(() => <RecordingCard />).render();
  await tree.settle();
  return tree;
};

describe("the Record now card", () => {
  it("offers Record now when nothing records", async () => {
    const tree = await show();
    expect(tree.button("Record now").props.disabled).toBe(false);
    expect(tree.has("Recording is off because the memory database could not be opened.")).toBe(false);
  });

  it("is off, with its caption, when Whimbrel runs without the memory database", async () => {
    h.noStore = true;
    const tree = await show();
    expect(tree.button("Record now").props.disabled).toBe(true);
    expect(tree.has("Recording is off because the memory database could not be opened.")).toBe(true);
  });

  it("gives way to the recording bar while a meeting records", async () => {
    h.status = { state: "recording", paused: false, pausedTotalMs: 0, resuming: false, startedUnixMs: 1_000, meetingId: 4 };
    const tree = await show();
    expect(tree.all((n) => n.tag === "button")).toHaveLength(0);
  });
});
