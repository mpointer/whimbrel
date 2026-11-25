import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProfileView, SuggestionView } from "../../../../generated/types";
import { Tree, text, type TNode } from "../../../../testkit/tree";
import { BANNED_PROFILE_PHRASES, suggestPrivacyRedesign } from "./profileWording";
import { suggestPrivacy } from "../../../profileModel";
import { ProfileSection } from "../ProfileSection";

// The redesigned Profile against the old card's commands. The shell is faked at its one door, `invoke`, so what is
// compared is the Tauri commands and their arguments.
// No provider is mounted, so a context reads as its default value.
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

const view = (over: Partial<ProfileView> = {}): ProfileView => ({
  name: "Sam",
  role: "",
  organization: "",
  about: "",
  facts: [{ id: "f1", text: "Works in US Eastern time", sourceMeetingId: null }],
  enabled: true,
  suggestEnabled: true,
  suggestionsActive: true,
  preview: "Name: Sam",
  composedChars: 9,
  maxChars: 2000,
  maxFacts: 30,
  maxFactChars: 200,
  ...over,
});
const suggestion: SuggestionView = { id: 7, text: "Leads the platform team", createdAtUnixMs: 0, sourceMeetingId: 3, sourceMeetingTitle: "Weekly sync" };

async function open(v: ProfileView = view(), s: SuggestionView[] = [suggestion]) {
  h.answers.profile_get = v;
  h.answers.profile_suggestions = s;
  const tree = new Tree(() => <ProfileSection onSection={() => {}} onRetentionSaved={() => {}} onOpenAutomations={() => {}} />).render();
  await tree.settle();
  h.calls = [];
  return tree;
}
const call = (node: TNode, prop: string, ...a: unknown[]) => (node.props[prop] as (...x: unknown[]) => unknown)(...a);
const typeIn = (tree: Tree, label: string, value: string) => {
  const f = tree.field(label);
  call(f, "onChange", { target: { value } });
  tree.render();
  call(tree.field(label), "onBlur");
};
const switches = (tree: Tree) => tree.all((n) => n.props.role === "switch");

beforeEach(() => {
  vi.stubGlobal("window", { dispatchEvent: () => true });
  h.calls = [];
  h.answers = {};
});

describe("Profile section: the same reads and commands as the old card", () => {
  it("reads the profile and the suggestions once, and draws the card with its anchor", async () => {
    h.answers.profile_get = view();
    h.answers.profile_suggestions = [];
    const tree = new Tree(() => <ProfileSection onSection={() => {}} onRetentionSaved={() => {}} onOpenAutomations={() => {}} />).render();
    await tree.settle();
    expect(h.calls.map((c) => c[0]).sort()).toEqual(["profile_get", "profile_suggestions"]);
    expect(tree.one((n) => n.tag === "section" && n.props.id === "profile.profile", "profile card")).toBeTruthy();
    expect(tree.one((n) => n.tag === "h3", "card heading").children.join("")).toBe("Profile");
  });

  it("a field whose save failed keeps its text and is saved again on the next blur, without another edit", async () => {
    const tree = await open();
    h.answers.profile_set_fields = new Error("disk is full");
    typeIn(tree, "Role", "Engineer");
    await tree.settle();
    expect(h.calls.filter((c) => c[0] === "profile_set_fields")).toHaveLength(1);
    expect(tree.field("Role").props.value).toBe("Engineer");
    h.answers.profile_set_fields = view({ role: "Engineer" });
    call(tree.field("Role"), "onBlur");
    await tree.settle();
    expect(h.calls.filter((c) => c[0] === "profile_set_fields")).toHaveLength(2);
    expect(h.calls.filter((c) => c[0] === "profile_set_fields")[1][1]).toMatchObject({ fields: { role: "Engineer" } });
    // Saved now: another blur sends nothing.
    call(tree.field("Role"), "onBlur");
    await tree.settle();
    expect(h.calls.filter((c) => c[0] === "profile_set_fields")).toHaveLength(2);
  });

  it("Learned facts, Suggested and What Whimbrel sees are real headings under the card heading", async () => {
    const tree = await open();
    const names = tree.all((n) => n.tag === "h4").map((n) => text(n).trim());
    expect(names).toEqual(["Learned facts", "Suggested", "What Whimbrel sees"]);
  });

  it("saves the four fields on blur only when one changed, as the old card does", async () => {
    const tree = await open();
    call(tree.field("Name"), "onBlur");
    expect(h.calls).toEqual([]);
    h.answers.profile_set_fields = view({ role: "Lead" });
    typeIn(tree, "Role", "Lead");
    expect(h.calls).toEqual([["profile_set_fields", { fields: { name: "Sam", role: "Lead", organization: "", about: "" } }]]);
  });

  it("saves About me too, with the other three unchanged", async () => {
    const tree = await open();
    typeIn(tree, "About me and my work", "I run ops.");
    expect(h.calls).toEqual([["profile_set_fields", { fields: { name: "Sam", role: "", organization: "", about: "I run ops." } }]]);
  });

  it("the two switches send both values, changing one", async () => {
    const tree = await open();
    const [use, suggest] = switches(tree);
    await call(use, "onClick");
    await call(suggest, "onClick");
    expect(h.calls).toEqual([
      ["profile_set_switches", { switches: { enabled: false, suggestEnabled: true } }],
      ["profile_set_switches", { switches: { enabled: true, suggestEnabled: false } }],
    ]);
  });

  it("removes a fact by id, and edits one with the same command", async () => {
    const tree = await open();
    h.answers.profile_remove_fact = view();
    h.answers.profile_update_fact = view();
    await call(tree.button("Remove"), "onClick");
    await tree.settle();
    expect(h.calls).toEqual([["profile_remove_fact", { id: "f1" }]]);
    h.calls = [];
    call(tree.button("Edit"), "onClick");
    tree.render();
    const box = tree.one((n) => n.tag === "input" && n.props["aria-label"] === "Edit fact", "edit box");
    call(box, "onChange", { target: { value: "Works in UK time" } });
    tree.render();
    await call(tree.button("Save"), "onClick");
    expect(h.calls).toEqual([["profile_update_fact", { id: "f1", text: "Works in UK time" }]]);
  });

  it("adds a fact with the typed text, and says why when it cannot", async () => {
    const tree = await open(view({ maxFactChars: 10 }));
    const box = () => tree.one((n) => n.tag === "input" && n.props["aria-label"] === "Add a fact", "add box");
    call(box(), "onChange", { target: { value: "a fact that is far too long" } });
    tree.render();
    await call(tree.button("Add fact"), "onClick");
    tree.render();
    expect(h.calls).toEqual([]);
    expect(tree.has("A fact can be at most 10 characters (this one is 27).")).toBe(true);
    call(box(), "onChange", { target: { value: "short" } });
    tree.render();
    h.answers.profile_add_fact = view();
    await call(tree.button("Add fact"), "onClick");
    expect(h.calls).toEqual([["profile_add_fact", { text: "short" }]]);
  });

  it("refuses a fact when the list is full, with the old sentence", async () => {
    const facts = Array.from({ length: 2 }, (_, i) => ({ id: `f${i}`, text: `fact ${i}`, sourceMeetingId: null }));
    const tree = await open(view({ facts, maxFacts: 2 }));
    call(tree.one((n) => n.tag === "input" && n.props["aria-label"] === "Add a fact"), "onChange", { target: { value: "one more" } });
    tree.render();
    await call(tree.button("Add fact"), "onClick");
    tree.render();
    expect(h.calls).toEqual([]);
    expect(tree.has("You can keep at most 2 facts. Remove one first.")).toBe(true);
  });

  it("Add and Dismiss on a suggestion call the old commands, and the row goes", async () => {
    const tree = await open();
    expect(tree.has("From “Weekly sync”")).toBe(true);
    h.answers.profile_accept_suggestion = view();
    await call(tree.button("Add"), "onClick");
    await tree.settle();
    expect(h.calls).toEqual([["profile_accept_suggestion", { id: 7 }]]);
    expect(tree.has("Leads the platform team")).toBe(false);
    const again = await open();
    await call(again.button("Dismiss"), "onClick");
    expect(h.calls).toEqual([["profile_dismiss_suggestion", { id: 7 }]]);
  });

  it("shows a refused save as words and leaves the typed text in the box", async () => {
    const tree = await open();
    h.answers.profile_set_fields = new Error("The profile could not be saved.");
    typeIn(tree, "Role", "Lead");
    await tree.settle();
    expect(tree.has("The profile could not be saved.")).toBe(true);
    expect((tree.field("Role").props as { value: string }).value).toBe("Lead");
  });

  it("over the cap: an alert says how much is not sent; under it, a plain count", async () => {
    const over = await open(view({ composedChars: 2100 }));
    const alert = over.all((n) => n.props.role === "alert" && text(n).includes("are not sent"));
    expect(alert).toHaveLength(1);
    expect(text(alert[0])).toBe("2,100 of 2,000 characters. The last 100 are not sent. Shorten it to send all of it.");
    const under = await open();
    expect(under.all((n) => n.props.role === "alert" && text(n).includes("characters"))).toHaveLength(0);
    expect(under.has("9 of 2,000 characters")).toBe(true);
  });

  it("shows the preview, and the off and empty lines", async () => {
    expect((await open()).has("Name: Sam")).toBe(true);
    expect((await open(view({ enabled: false, preview: "" }))).has("Off. Nothing from your Profile is sent with a prompt.")).toBe(true);
    expect((await open(view({ preview: "" }))).has("Nothing written yet. Nothing is sent.")).toBe(true);
  });
});

describe("Profile section: wording", () => {
  it("says AI where the old line said AI & routing", () => {
    expect(suggestPrivacy()).toContain("AI & routing");
    expect(suggestPrivacyRedesign()).not.toContain("AI & routing");
    expect(suggestPrivacyRedesign()).toContain("you chose in AI,");
  });

  it("shows none of the dropped names anywhere on the card, in any state", async () => {
    for (const v of [view(), view({ suggestEnabled: false }), view({ suggestionsActive: false }), view({ enabled: false, preview: "" })]) {
      const all = (await open(v, [suggestion])).text();
      for (const phrase of BANNED_PROFILE_PHRASES) expect(all).not.toContain(phrase);
    }
  });
});
