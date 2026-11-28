import { beforeEach, describe, expect, it, vi } from "vitest";
import { NoStoreBanner } from "../launch/NoStoreBanner";
import { Tree, text, type TNode } from "../testkit/tree";
import { ArmNotice } from "./ArmNotice";
import { RetentionNotice } from "./RetentionNotice";
import { SignInBanner } from "./SignInBanner";

// The three first-run banners and the no-store banner: they draw as banners and ask the shell for what they always did.
vi.mock("react", async (actual) => (await import("../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));
const h = vi.hoisted(() => ({ calls: [] as [string, unknown][], answers: {} as Record<string, unknown>, stored: {} as Record<string, string> }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: unknown) => {
    h.calls.push([cmd, args ?? null]);
    return cmd in h.answers ? h.answers[cmd] : null;
  },
}));

const EMAIL = "alex.rivera@contoso.example.test";
beforeEach(() => {
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

const show = async (el: () => React.ReactNode) => {
  const tree = new Tree(el).render();
  await tree.settle();
  return tree;
};
const classes = (tree: Tree) => tree.all((n) => typeof n.props.className === "string").map((n) => String(n.props.className));
const commands = () => h.calls;
const click = (b: TNode) => (b.props.onClick as () => void)();

describe("the Arm notice", () => {
  it("is a banner with a system button", async () => {
    const now = await show(() => <ArmNotice />);
    expect(classes(now).some((c) => c.includes("w-banner"))).toBe(true);
    expect(classes(now)).not.toContain("card notice");
    expect(now.has("This PC has an Arm processor")).toBe(true);
    expect(now.text()).toContain("Choosing Whimbrel AI in Settings → AI moves summaries off this PC.");
  });

  it("dismissing writes the mark and removes the notice", async () => {
    h.stored = {};
    const tree = await show(() => <ArmNotice />);
    click(tree.button("Dismiss"));
    tree.render();
    expect(h.stored).toEqual({ "whimbrel.armNotice.dismissed": "1" });
    expect(tree.all((n) => n.tag === "section")).toHaveLength(0);
  });

  it("stays away when it was dismissed before", async () => {
    h.stored = { "whimbrel.armNotice.dismissed": "1" };
    expect((await show(() => <ArmNotice />)).all((n) => n.tag === "section")).toHaveLength(0);
  });
});

describe("the sign-in banner", () => {
  it("is a warning banner with a line and a button", async () => {
    const now = await show(() => <SignInBanner />);
    expect(classes(now).some((c) => c.includes("w-banner-warning"))).toBe(true);
    expect(now.has("Sign in again")).toBe(true);
  });

  it("Sign in again asks for the reconnect with the account", async () => {
    h.calls = [];
    const tree = await show(() => <SignInBanner />);
    click(tree.button("Sign in again"));
    await tree.settle();
    expect(commands().filter(([c]) => c === "calendar_reconnect")).toEqual([["calendar_reconnect", { email: EMAIL }]]);
  });
});

describe("the retention notice", () => {
  it("is a banner with its title and what the sweep would remove", async () => {
    const now = await show(() => <RetentionNotice onChangeSettings={() => {}} />);
    expect(now.one((n) => n.tag === "h2").children.map(text).join("")).toBe("Before Whimbrel deletes anything");
    expect(now.has("Screen text older than 90 days")).toBe(true);
  });

  it("Keep current settings runs the sweep and Change settings goes to Settings", async () => {
    h.calls = [];
    const change = vi.fn();
    const tree = await show(() => <RetentionNotice onChangeSettings={change} />);
    click(tree.button("Change settings…"));
    expect(change).toHaveBeenCalledTimes(1);
    click(tree.button("Keep current settings"));
    await tree.settle();
    expect(commands().filter(([c]) => c === "retention_run_now")).toEqual([["retention_run_now", null]]);
  });

  it("shows nothing once the answer was given", async () => {
    h.answers.retention_settings = { acknowledged: true, lastRun: null, settings: {} };
    expect((await show(() => <RetentionNotice onChangeSettings={() => {}} />)).all((n) => n.tag === "section")).toHaveLength(0);
  });
});

describe("the no-store banner", () => {
  it("is a banner with a button that opens Settings", async () => {
    const open = vi.fn();
    const tree = await show(() => <NoStoreBanner onOpenSettings={open} />);
    expect(tree.has("The memory database could not be opened. Recording is off. Restart Whimbrel to try again.")).toBe(true);
    expect(classes(tree).some((c) => c.includes("w-banner"))).toBe(true);
    click(tree.button("Open Settings"));
    expect(open).toHaveBeenCalledTimes(1);
  });

  it("draws no button without a way to open Settings", async () => {
    expect((await show(() => <NoStoreBanner />)).all((n) => n.tag === "button")).toHaveLength(0);
  });
});
