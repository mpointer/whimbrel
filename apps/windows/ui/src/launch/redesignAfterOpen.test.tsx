import { beforeEach, describe, expect, it, vi } from "vitest";
import { Tree } from "../testkit/tree";
import { primeRedesign } from "../redesign";
import { LaunchGate } from "./LaunchGate";

// Found on a real PC (2026-11-22): after Delete everything, finishing the onboarding showed the old shell until a
// restart, because the page asked the shell for the switch before the launch had opened the store and kept the early
// no-answer. The switch is now on for everyone and nothing asks (switch-removal.md, A1), so the redesigned shell draws
// at once, before and after the store opens, and whatever the fake shell would answer. The fake still answers only once
// the store is open, as the real one did, so a regression that asks again shows here.
vi.mock("react", async (actual) => (await import("../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));

const h = vi.hoisted(() => ({
  opened: false,
  flag: true,
  generation: 1,
  listeners: {} as Record<string, () => void>,
}));

vi.mock("../App", async () => {
  const { useRedesign } = await import("../redesign");
  return { App: () => <main>{useRedesign() ? "redesigned shell" : "old shell"}</main> };
});
// The test tree draws function components only; the context's provider passes its children through.
vi.mock("./noStore", () => ({ NoStoreContext: { Provider: ({ children }: { children: unknown }) => children } }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, fn: () => void) => {
    h.listeners[name] = fn;
    return Promise.resolve(() => {});
  },
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string) => {
    if (cmd === "launch_state")
      return {
        busy: false,
        waitingForUser: !h.opened,
        generation: h.generation,
        stage: h.opened ? { kind: "opened" } : { kind: "onboarding", reusesKey: false },
      };
    if (cmd === "redesign_enabled") {
      if (!h.opened) throw new Error("state not managed for field `state` on command `redesign_enabled`");
      return h.flag;
    }
    if (cmd === "launch_fresh_library_notice") return null;
    return null;
  },
}));

beforeEach(() => {
  vi.stubGlobal("document", { activeElement: null, body: { nodeType: 1 } });
  h.opened = false;
  h.listeners = {};
});

async function onboardThenOpen(): Promise<Tree> {
  // main.tsx asks before the first render, while the launch still waits on the onboarding.
  expect(await primeRedesign()).toBe(true);
  const tree = await new Tree(() => <LaunchGate />).render().settle();
  expect(tree.has("redesigned shell") || tree.has("old shell")).toBe(false);
  // The recovery key steps are done: the store opens and the shell says so.
  h.opened = true;
  h.generation += 1;
  h.listeners["launch://opened"]();
  await tree.settle();
  return tree.settle();
}

describe("the redesign switch once the onboarding is done", () => {
  it("draws the redesigned shell without a restart", async () => {
    h.flag = true;
    const tree = await onboardThenOpen();
    expect(tree.has("redesigned shell")).toBe(true);
    expect(tree.has("old shell")).toBe(false);
  });

  it("draws it even when the stored value or the shell says off, since the switch is no longer read", async () => {
    h.flag = false;
    const tree = await onboardThenOpen();
    expect(tree.has("redesigned shell")).toBe(true);
    expect(tree.has("old shell")).toBe(false);
  });
});
