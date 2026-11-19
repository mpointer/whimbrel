import { beforeEach, describe, expect, it, vi } from "vitest";
import { NoStoreBanner } from "../launch/NoStoreBanner";
import { Tree, text, type TNode } from "../testkit/tree";
import { ArmNotice } from "./ArmNotice";
import { RetentionNotice } from "./RetentionNotice";
import { SignInBanner } from "./SignInBanner";

// The three first-run banners and the no-store banner, flag off against flag on: the old markup is what it was,
// and the new one asks the shell for the same things.
vi.mock("react", async (actual) => (await import("../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));
const h = vi.hoisted(() => ({ flag: false, calls: [] as [string, unknown][], answers: {} as Record<string, unknown>, stored: {} as Record<string, string> }));
vi.mock("../redesign", () => ({ useRedesign: () => h.flag }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: unknown) => {
    h.calls.push([cmd, args ?? null]);
    return cmd in h.answers ? h.answers[cmd] : null;
  },
}));

const EMAIL = "alex.rivera@contoso.example.test";
beforeEach(() => {
  h.flag = false;
  h.calls = [];
  h.stored = {};
  h.answers = {
    running_emulated: true,
    retention_settings: { acknowledged: false, lastRun: null, settings: { captureDays: 90, conversationDays: null, meetingDays: 365 } },
    retention_preview: { captures: 214, conversations: 0, meetings: 3, orphanBytes: 0, orphans: 0 },
    retention_run_now: { acknowledged: true, lastRun: null, settings: {} },
    calendar_accounts: { accessIssues: [], blocked: null, emails: [EMAIL], signInExpired: [EMAIL] },
    gmail_accounts: { accounts: [], blocked: null },
    calendar_reconnect: { accessIssues: [], blocked: null, emails: [EMAIL], signInExpired: [] },
  };
  vi.stubGlobal("localStorage", { getItem: (k: string) => h.stored[k] ?? null, setItem: (k: string, v: string) => void (h.stored[k] = v) });
  vi.stubGlobal("setInterval", () => 0);
  vi.stubGlobal("clearInterval", () => {});
});

const show = async (el: () => React.ReactNode, flag: boolean) => {
  h.flag = flag;
  const tree = new Tree(el).render();
  await tree.settle();
  return tree;
};
const classes = (tree: Tree) => tree.all((n) => typeof n.props.className === "string").map((n) => String(n.props.className));
const commands = () => h.calls;
const click = (b: TNode) => (b.props.onClick as () => void)();

describe("the Arm notice", () => {
  it("flag off: the old card, with its mode button; flag on: the banner, with a system button", async () => {
    const old = await show(() => <ArmNotice />, false);
    expect(classes(old)).toContain("card notice");
    expect(String(old.button("Dismiss").props.className)).toBe("mode");
    expect(classes(old).some((c) => c.includes("w-banner"))).toBe(false);
    const now = await show(() => <ArmNotice />, true);
    expect(classes(now).some((c) => c.includes("w-banner"))).toBe(true);
    expect(classes(now)).not.toContain("card notice");
    expect(now.has("This PC has an Arm processor")).toBe(true);
    expect(old.text()).toContain("Choosing Whimbrel AI in Settings → AI moves summaries off this PC.");
  });

  it("dismissing writes the same mark and removes the notice, in either look", async () => {
    for (const flag of [false, true]) {
      h.stored = {};
      const tree = await show(() => <ArmNotice />, flag);
      click(tree.button("Dismiss"));
      tree.render();
      expect(h.stored).toEqual({ "whimbrel.armNotice.dismissed": "1" });
      expect(tree.all((n) => n.tag === "section")).toHaveLength(0);
    }
  });

  it("stays away when it was dismissed before", async () => {
    h.stored = { "whimbrel.armNotice.dismissed": "1" };
    for (const flag of [false, true]) expect((await show(() => <ArmNotice />, flag)).all((n) => n.tag === "section")).toHaveLength(0);
  });
});

describe("the sign-in banner", () => {
  it("flag off: the old card; flag on: a warning banner with the same line and button", async () => {
    const old = await show(() => <SignInBanner />, false);
    expect(classes(old)).toContain("card notice");
    const now = await show(() => <SignInBanner />, true);
    expect(classes(now).some((c) => c.includes("w-banner-warning"))).toBe(true);
    expect(now.text()).toBe(old.text());
  });

  it("Sign in again asks for the same reconnect with the same account, in either look", async () => {
    const sent: unknown[][] = [];
    for (const flag of [false, true]) {
      h.calls = [];
      const tree = await show(() => <SignInBanner />, flag);
      click(tree.button("Sign in again"));
      await tree.settle();
      sent.push(commands().filter(([c]) => c === "calendar_reconnect"));
    }
    expect(sent[1]).toEqual(sent[0]);
    expect(sent[1]).toEqual([["calendar_reconnect", { email: EMAIL }]]);
  });
});

describe("the retention notice", () => {
  it("flag off: the old card, region and mode buttons; flag on: a banner with its title and the same lines", async () => {
    const old = await show(() => <RetentionNotice onChangeSettings={() => {}} />, false);
    expect(classes(old)).toContain("card notice");
    expect(String(old.button("Keep current settings").props.className)).toBe("mode");
    const now = await show(() => <RetentionNotice onChangeSettings={() => {}} />, true);
    expect(now.one((n) => n.tag === "h2").children.map(text).join("")).toBe("Before Whimbrel deletes anything");
    for (const line of old.all((n) => n.tag === "li")) expect(now.has(text(line).replace(/\s+/g, " ").trim().slice(0, 20))).toBe(true);
    expect(now.has("Screen text older than 90 days")).toBe(true);
  });

  it("Keep current settings runs the sweep and Change settings goes to Settings, in either look", async () => {
    const sent: unknown[][] = [];
    for (const flag of [false, true]) {
      h.calls = [];
      const change = vi.fn();
      const tree = await show(() => <RetentionNotice onChangeSettings={change} />, flag);
      click(tree.button("Change settings…"));
      expect(change).toHaveBeenCalledTimes(1);
      click(tree.button("Keep current settings"));
      await tree.settle();
      sent.push(commands().filter(([c]) => c === "retention_run_now"));
    }
    expect(sent[1]).toEqual(sent[0]);
    expect(sent[1]).toEqual([["retention_run_now", null]]);
  });

  it("shows nothing once the answer was given", async () => {
    h.answers.retention_settings = { acknowledged: true, lastRun: null, settings: {} };
    for (const flag of [false, true]) expect((await show(() => <RetentionNotice onChangeSettings={() => {}} />, flag)).all((n) => n.tag === "section")).toHaveLength(0);
  });
});

describe("the no-store banner", () => {
  it("flag off: the old card and link; flag on: a banner with a button; both open Settings", async () => {
    for (const flag of [false, true]) {
      const open = vi.fn();
      const tree = await show(() => <NoStoreBanner onOpenSettings={open} />, flag);
      expect(tree.has("The memory database could not be opened. Recording is off. Restart Whimbrel to try again.")).toBe(true);
      expect(classes(tree).includes("card notice no-store-banner")).toBe(!flag);
      expect(classes(tree).some((c) => c.includes("w-banner"))).toBe(flag);
      click(tree.button("Open Settings"));
      expect(open).toHaveBeenCalledTimes(1);
    }
  });

  it("draws no button without a way to open Settings", async () => {
    for (const flag of [false, true]) expect((await show(() => <NoStoreBanner />, flag)).all((n) => n.tag === "button")).toHaveLength(0);
  });
});
