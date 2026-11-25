import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SyncView, WhimbrelAiView } from "../../../../generated/types";
import { Tree, text, type TNode } from "../../../../testkit/tree";
import { PlanSection } from "../PlanSection";

// The redesigned Plan & billing against the old card's reads and commands, with the shell faked at `invoke`.
vi.mock("react", async (actual) => {
  const stubbed = (await import("../../../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>());
  const useContext = (c: { _currentValue: unknown }) => c._currentValue;
  return { ...stubbed, useContext, default: { ...(stubbed as unknown as { default: object }).default, useContext } };
});

const h = vi.hoisted(() => ({ calls: [] as [string, unknown][], answers: {} as Record<string, unknown> }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: unknown) => {
    h.calls.push([cmd, args ?? null]);
    const a = h.answers[cmd];
    if (a instanceof Error) throw a;
    return a ?? null;
  },
}));

const sync = (over: Partial<SyncView> = {}): SyncView => ({
  state: "ready",
  message: null,
  deviceId: "k",
  deviceLimit: false,
  planUnknown: false,
  plan: { plan: "free", untilUnixMs: null, interval: null, billing: true, plusAvailable: true },
  ...over,
});
const plus = sync({ plan: { plan: "cloud", untilUnixMs: null, interval: "month", billing: true, plusAvailable: true, planTier: "plus" } });
const ai = { signedIn: true } as unknown as WhimbrelAiView;
const listeners: Record<string, () => void> = {};

async function open(view: SyncView, onSection = vi.fn()) {
  h.answers.sync_status = view;
  h.answers.sync_refresh_plan = view;
  h.answers.whimbrel_ai_status = ai;
  h.answers.whimbrel_ai_refresh = ai;
  const tree = new Tree(() => <PlanSection onSection={onSection} onRetentionSaved={() => {}} onOpenAutomations={() => {}} />).render();
  await tree.settle();
  return { tree, onSection };
}
const call = (node: TNode, prop: string, ...a: unknown[]) => (node.props[prop] as (...x: unknown[]) => unknown)(...a);
const buttons = (tree: Tree) => tree.all((n) => n.tag === "button").map((n) => text(n).trim());

beforeEach(() => {
  h.calls = [];
  h.answers = {};
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  vi.stubGlobal("window", { addEventListener: (k: string, f: () => void) => (listeners[k] = f), removeEventListener: () => {} });
});
afterEach(() => vi.useRealTimers());

describe("Plan & billing: what it reads and when", () => {
  it("reads sync, the plan and the AI balance when it opens", async () => {
    await open(sync());
    const names = h.calls.map((c) => c[0]);
    for (const c of ["sync_status", "sync_refresh_plan", "whimbrel_ai_status", "whimbrel_ai_refresh"]) expect(names).toContain(c);
  });

  it("looks at sync every 5 seconds, and reads the plan when the window comes back", async () => {
    await open(sync());
    h.calls = [];
    vi.advanceTimersByTime(5000);
    expect(h.calls.map((c) => c[0])).toEqual(["sync_status"]);
    h.calls = [];
    listeners.focus();
    expect(h.calls.map((c) => c[0]).sort()).toEqual(["sync_refresh_plan", "whimbrel_ai_refresh"]);
  });
});

describe("Plan & billing: the states", () => {
  it("signed out: says so and offers Sign in to sync, which opens Sync & Devices", async () => {
    const { tree, onSection } = await open(sync({ state: "notSignedIn", plan: null }));
    expect(tree.has("Sign in to sync to see your plan.")).toBe(true);
    call(tree.button("Sign in to sync"), "onClick");
    expect(onSection).toHaveBeenCalledWith("devices");
    expect(buttons(tree)).toEqual(["Sign in to sync", "AI settings"]);
  });

  it("free with Plus on sale: the two Plus buttons, the caption, and no Manage", async () => {
    const { tree } = await open(sync());
    expect(tree.has("Free: 1 computer + 1 phone")).toBe(true);
    expect(buttons(tree)).toEqual(["Get Plus: $15 a month", "Get Plus: $144 a year", "AI settings"]);
    expect(tree.has("Plus includes hosted sync")).toBe(true);
  });

  it("free without Plus: the Cloud buttons", async () => {
    const { tree } = await open(sync({ plan: { plan: "free", untilUnixMs: null, interval: null, billing: true, plusAvailable: false } }));
    expect(buttons(tree)).toEqual(["Upgrade to Cloud: $5 a month", "Upgrade to Cloud: $50 a year", "AI settings"]);
  });

  it("billing closed: the plan line and no purchase buttons", async () => {
    const { tree } = await open(sync({ plan: { plan: "free", untilUnixMs: null, interval: null, billing: false, plusAvailable: true } }));
    expect(buttons(tree)).toEqual(["AI settings"]);
  });

  it("Plus: the plan line, Manage subscription, the allowance line, and the anchors", async () => {
    h.answers.whimbrel_ai_status = ai;
    const { tree } = await open(plus);
    expect(tree.has("Plus")).toBe(true);
    expect(buttons(tree)).toEqual(["Manage subscription", "AI settings"]);
    for (const id of ["plan.summary", "plan.manage", "plan.provider"]) expect(tree.all((n) => n.props.id === id)).toHaveLength(1);
  });

  it("checking, could not check, and finish signing in", async () => {
    expect((await open(sync({ plan: null }))).tree.has("Checking your plan…")).toBe(true);
    const failed = (await open(sync({ plan: null, planUnknown: true }))).tree;
    expect(failed.has("Couldn't check your plan. Try again in a moment.")).toBe(true);
    h.calls = [];
    call(failed.button("Try again"), "onClick");
    expect(h.calls.map((c) => c[0])).toEqual(["sync_refresh_plan"]);
    expect((await open(sync({ state: "waitingForApproval", plan: null }))).tree.has("Finish signing in to manage your plan.")).toBe(true);
  });

  it("always offers the way to AI settings", async () => {
    const { tree, onSection } = await open(sync({ plan: null }));
    call(tree.button("AI settings"), "onClick");
    expect(onSection).toHaveBeenCalledWith("ai");
    expect(tree.has("Use Whimbrel AI as your provider")).toBe(true);
  });
});

describe("Plan & billing: the commands", () => {
  it("Buy sends the same interval the old buttons send", async () => {
    const { tree } = await open(sync());
    h.calls = [];
    call(tree.button("Get Plus: $15 a month"), "onClick");
    call(tree.button("Get Plus: $144 a year"), "onClick");
    expect(h.calls).toEqual([
      ["sync_upgrade", { interval: "month" }],
      ["sync_upgrade", { interval: "year" }],
    ]);
  });

  it("Manage opens the billing portal command with no arguments", async () => {
    const { tree } = await open(plus);
    h.calls = [];
    call(tree.button("Manage subscription"), "onClick");
    expect(h.calls).toEqual([["sync_manage_subscription", null]]);
  });

  it("a billing error shows as words under the buttons, and the buttons work again", async () => {
    const { tree } = await open(sync());
    h.answers.sync_upgrade = new Error("Couldn't reach Stripe. Try again in a minute.");
    call(tree.button("Get Plus: $15 a month"), "onClick");
    await tree.settle();
    expect(tree.has("Couldn't reach Stripe. Try again in a minute.")).toBe(true);
    expect(tree.button("Get Plus: $15 a month").props.disabled).toBe(false);
  });

  it("buttons are off while a billing call runs, and the row says why", async () => {
    const { tree } = await open(sync());
    call(tree.button("Get Plus: $15 a month"), "onClick");
    tree.render();
    expect(tree.button("Get Plus: $15 a month").props.disabled).toBe(true);
    expect(tree.has("Working…")).toBe(true);
    await tree.settle();
  });
});
