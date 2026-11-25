import { beforeEach, describe, expect, it, vi } from "vitest";
import { Tree, text, type TNode } from "../../../../../testkit/tree";

vi.mock("react", async (actual) => (await import("../../../../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));
vi.mock("../../../../../l10n", async (actual) => {
  const real = await actual<typeof import("../../../../../l10n")>();
  return { ...real, useL10n: () => ({ t: real.t, locale: "en", lang: "en" }) };
});
const h = vi.hoisted(() => ({ calls: [] as [string, unknown][], stored: null as unknown, acknowledged: true, failReads: null as string | null }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: { settings?: unknown }) => {
    h.calls.push([cmd, args ?? null]);
    if (cmd === "retention_settings" && h.failReads) throw { kind: "other", message: h.failReads };
    if (cmd === "retention_settings" || cmd === "retention_run_now") return { acknowledged: h.acknowledged, lastRun: null, settings: h.stored };
    if (cmd === "set_retention_settings") {
      h.stored = args?.settings;
      return { acknowledged: h.acknowledged, lastRun: null, settings: h.stored };
    }
    if (cmd === "retention_preview") return { captures: 214, conversations: 0, meetings: 3, orphanBytes: 0, orphans: 0 };
    return null;
  },
}));
vi.stubGlobal("document", { activeElement: null, getElementById: () => null });

const { RetentionKeepingCard } = await import("./RetentionKeepingCard");

const saves = () => h.calls.filter(([c]) => c === "set_retention_settings");
const runs = () => h.calls.filter(([c]) => c === "retention_run_now");
const click = (b: TNode) => (b.props.onClick as () => void)();
const ticks = (tree: Tree) => tree.all((n) => n.tag === "input" && n.props.type === "checkbox");
const boxes = (tree: Tree) => tree.all((n) => n.tag === "input" && n.props.type === "text");
const tick = (box: TNode, checked: boolean) => (box.props.onChange as (e: unknown) => void)({ target: { checked } });
const typeDays = async (tree: Tree, index: number, value: string) => {
  const box = boxes(tree)[index];
  (box.props.onChange as (e: unknown) => void)({ target: { value } });
  tree.render();
  (boxes(tree)[index].props.onBlur as () => void)();
  await tree.settle();
};
let saved = 0;
const show = async () => {
  saved = 0;
  const tree = new Tree(() => <RetentionKeepingCard onSaved={() => saved++} />).render();
  await tree.settle();
  return tree;
};

beforeEach(() => {
  h.calls = [];
  h.acknowledged = true;
  h.failReads = null;
  // Screen text 30 days, Meetings and recordings forever, Conversations 90 days.
  h.stored = { captureDays: 30, meetingDays: null, conversationDays: 90 };
});

describe("Keeping data", () => {
  it("is the card with the retention anchor, with the three renamed rows", async () => {
    const tree = await show();
    expect(tree.one((n) => n.tag === "section" && n.props.id === "capture.retention").tag).toBe("section");
    for (const label of ["Screen text", "Meetings and recordings", "Conversations"]) expect(tree.has(label)).toBe(true);
    expect(tree.has("Chats")).toBe(false);
    expect(ticks(tree).map((b) => b.props.checked)).toEqual([false, true, false]);
  });

  it("unticking Keep forever saves nothing and shows the last number", async () => {
    const tree = await show();
    tick(ticks(tree)[1], false);
    tree.render();
    await tree.settle();
    expect(saves()).toEqual([]);
    expect(boxes(tree)[1].props.value).toBe("30");
    expect(tree.has("Not saved yet")).toBe(true);
    expect(ticks(tree)[1].props.checked).toBe(false);
  });

  it("a period after forever opens the dialog; nothing is saved until Delete older data", async () => {
    const tree = await show();
    tick(ticks(tree)[1], false);
    tree.render();
    await typeDays(tree, 1, "10");
    expect(saves()).toEqual([]);
    const dialog = tree.one((n) => n.props.role === "alertdialog");
    expect(text(dialog)).toContain("Delete meetings and recordings older than 10 days?");
    expect(text(dialog)).toContain("It is kept forever now.");
    expect(tree.button("Cancel").props["data-initial"]).toBe(true);
    click(tree.button("Delete older data"));
    await tree.settle();
    expect(saves()).toEqual([["set_retention_settings", { settings: { captureDays: 30, meetingDays: 10, conversationDays: 90 } }]]);
    expect(saved).toBe(1);
    expect(tree.all((n) => n.props.role === "alertdialog")).toEqual([]);
  });

  it("a shorter period asks first, and Cancel leaves the stored value", async () => {
    const tree = await show();
    await typeDays(tree, 0, "7");
    expect(saves()).toEqual([]);
    expect(text(tree.one((n) => n.props.role === "alertdialog"))).toContain("Delete screen text older than 7 days?");
    click(tree.button("Cancel"));
    await tree.settle();
    expect(saves()).toEqual([]);
    expect(saved).toBe(0);
    expect(boxes(tree)[0].props.value).toBe("30");
    expect(tree.all((n) => n.props.role === "alertdialog")).toEqual([]);
  });

  it("a longer period saves without asking, and the same number writes nothing", async () => {
    const tree = await show();
    await typeDays(tree, 0, "60");
    expect(tree.all((n) => n.props.role === "alertdialog")).toEqual([]);
    expect(saves()).toEqual([["set_retention_settings", { settings: { captureDays: 60, meetingDays: null, conversationDays: 90 } }]]);
    expect(saved).toBe(1);
    await typeDays(tree, 0, "60");
    expect(saves().length).toBe(1);
  });

  it("writes only the changed key on top of settings read again just before the save (another computer changed another row)", async () => {
    const tree = await show();
    // Another computer set Meetings to 5 days and Conversations to 14 since this screen was read.
    h.stored = { captureDays: 30, meetingDays: 5, conversationDays: 14 };
    await typeDays(tree, 0, "60");
    expect(saves()).toEqual([["set_retention_settings", { settings: { captureDays: 60, meetingDays: 5, conversationDays: 14 } }]]);
  });

  it("asks again when this row changed on another computer and the save is now a shortening", async () => {
    const tree = await show();
    // Another computer made Screen text 90 days. 60 looked longer than 30, but it is shorter than 90.
    h.stored = { captureDays: 90, meetingDays: null, conversationDays: 90 };
    await typeDays(tree, 0, "60");
    expect(saves()).toEqual([]);
    const dialog = tree.one((n) => n.props.role === "alertdialog");
    expect(text(dialog)).toContain("Delete screen text older than 60 days?");
    expect(text(dialog)).toContain("It is kept for 90 days now.");
    click(tree.button("Delete older data"));
    await tree.settle();
    expect(saves()).toEqual([["set_retention_settings", { settings: { captureDays: 60, meetingDays: null, conversationDays: 90 } }]]);
  });

  it("committing the same number says it is already set", async () => {
    const tree = await show();
    await typeDays(tree, 0, "30");
    expect(saves()).toEqual([]);
    expect(tree.has("Already set to 30 days.")).toBe(true);
  });

  it("with Keep forever just unticked, the same number as the remembered one goes to the question, not to Already set", async () => {
    const tree = await show();
    tick(ticks(tree)[1], false);
    tree.render();
    expect(boxes(tree)[1].props.value).toBe("30");
    await typeDays(tree, 1, "30");
    expect(tree.has("Already set to")).toBe(false);
    expect(saves()).toEqual([]);
    expect(text(tree.one((n) => n.props.role === "alertdialog"))).toContain("Delete meetings and recordings older than 30 days?");
    click(tree.button("Delete older data"));
    await tree.settle();
    expect(saves()).toEqual([["set_retention_settings", { settings: { captureDays: 30, meetingDays: 30, conversationDays: 90 } }]]);
  });

  it("when the current setting cannot be read before a save, it says so plainly, nothing is saved, and the shell text is detail", async () => {
    const tree = await show();
    h.failReads = "disk busy (os error 5)";
    await typeDays(tree, 0, "60");
    expect(saves()).toEqual([]);
    expect(tree.has("Could not check the current setting, so nothing was saved.")).toBe(true);
    expect(tree.has("disk busy (os error 5)")).toBe(true);
  });

  it("an old stored value outside the range shows a note and a valid number replaces it", async () => {
    h.stored = { captureDays: 0, meetingDays: null, conversationDays: 90 };
    const tree = await show();
    expect(tree.has("The stored value, 0 days, is outside 1 to 3,650.")).toBe(true);
    await typeDays(tree, 0, "10");
    expect(saves()).toEqual([["set_retention_settings", { settings: { captureDays: 10, meetingDays: null, conversationDays: 90 } }]]);
  });

  it("an empty or out of range number saves nothing and says why", async () => {
    const tree = await show();
    await typeDays(tree, 0, "");
    expect(saves()).toEqual([]);
    expect(tree.has("Enter a number from 1 to 3,650.")).toBe(true);
    await typeDays(tree, 2, "99999");
    expect(saves()).toEqual([]);
    expect(tree.text().match(/Enter a number from/g)?.length).toBe(2);
    expect(tree.all((n) => n.props.role === "alertdialog")).toEqual([]);
  });

  it("ticking Keep forever saves null at once", async () => {
    const tree = await show();
    tick(ticks(tree)[0], true);
    await tree.settle();
    expect(saves()).toEqual([["set_retention_settings", { settings: { captureDays: null, meetingDays: null, conversationDays: 90 } }]]);
    expect(saved).toBe(1);
    // The number it had is what an untick shows.
    tick(ticks(tree)[0], false);
    tree.render();
    expect(boxes(tree)[0].props.value).toBe("30");
  });

  it("Delete old data now asks first, with counts, and Cancel is where focus starts", async () => {
    const tree = await show();
    click(tree.button("Delete old data now"));
    await tree.settle();
    expect(runs()).toEqual([]);
    const dialog = tree.one((n) => n.props.role === "alertdialog");
    expect(text(dialog)).toContain("Screen text older than 30 days: 214 items");
    expect(tree.button("Cancel").props["data-initial"]).toBe(true);
    click(tree.button("Delete old data"));
    await tree.settle();
    expect(runs().length).toBe(1);
  });

  it("Delete old data now can be cancelled without running", async () => {
    const tree = await show();
    click(tree.button("Delete old data now"));
    await tree.settle();
    click(tree.button("Cancel"));
    await tree.settle();
    expect(runs()).toEqual([]);
  });

  it("says the daily sweep is waiting when the notice was not answered", async () => {
    h.acknowledged = false;
    const tree = await show();
    expect(tree.has("The daily sweep is waiting for the notice on the main screen.")).toBe(true);
  });
});
