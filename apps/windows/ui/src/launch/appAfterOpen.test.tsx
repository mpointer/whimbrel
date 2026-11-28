import { beforeEach, describe, expect, it, vi } from "vitest";
import { Tree } from "../testkit/tree";
import { LaunchGate } from "./LaunchGate";

// Found on a real PC (2026-11-22): after Delete everything, finishing the onboarding showed the old shell until a restart,
// because the page asked the shell for a look switch before the launch had opened the store. There is no switch to ask
// now, so the app draws as soon as the store opens, and nothing asks the shell for one. The fake shell still answers
// only once the store is open, as the real one did, and records every command so a new ask shows here.
vi.mock("react", async (actual) => (await import("../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));

const h = vi.hoisted(() => ({
  opened: false,
  calls: [] as string[],
  generation: 1,
  listeners: {} as Record<string, () => void>,
}));

vi.mock("../App", () => ({ App: () => <main>the app</main> }));
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
    h.calls.push(cmd);
    if (cmd === "launch_state")
      return {
        busy: false,
        waitingForUser: !h.opened,
        generation: h.generation,
        stage: h.opened ? { kind: "opened" } : { kind: "onboarding", reusesKey: false },
      };
    if (cmd === "launch_fresh_library_notice") return null;
    return null;
  },
}));

beforeEach(() => {
  vi.stubGlobal("document", { activeElement: null, body: { nodeType: 1 } });
  h.opened = false;
  h.calls = [];
  h.listeners = {};
});

async function onboardThenOpen(): Promise<Tree> {
  const tree = await new Tree(() => <LaunchGate />).render().settle();
  expect(tree.has("the app")).toBe(false);
  // The recovery key steps are done: the store opens and the shell says so.
  h.opened = true;
  h.generation += 1;
  h.listeners["launch://opened"]();
  await tree.settle();
  return tree.settle();
}

describe("the app once the onboarding is done", () => {
  it("draws without a restart, and asks the shell for no look switch", async () => {
    const tree = await onboardThenOpen();
    expect(tree.has("the app")).toBe(true);
    expect(h.calls.filter((c) => c !== "launch_state" && c !== "launch_fresh_library_notice" && c !== "launch_first_window_rendered")).toEqual([]);
  });
});
