import { beforeEach, describe, expect, it, vi } from "vitest";
import { Tree, text, type TNode } from "../../../../testkit/tree";

vi.mock("react", async (actual) => (await import("../../../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));
vi.stubGlobal("setInterval", () => 0);
vi.stubGlobal("clearInterval", () => {});
vi.stubGlobal("document", { activeElement: null, getElementById: () => null });

const h = vi.hoisted(() => ({ calls: [] as [string, unknown][] }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: unknown) => {
    h.calls.push([cmd, args ?? null]);
    if (cmd === "topic_seeds") return [{ key: "k1", name: "Acme", aliases: [], kind: null, itemCount: 3 }];
    if (cmd === "topic_suggestions") return [{ topicId: 5, name: "Globex", itemCount: 2 }];
    if (cmd === "topic_naming_status") return { placeholderCount: 0 };
    return null;
  },
}));

const { TopicsListCard } = await import("./TopicsListCard");
const call = (n: TNode, prop: string, ...a: unknown[]) => (n.props[prop] as (...x: unknown[]) => void)(...a);
const cmds = (name: string) => h.calls.filter(([c]) => c === name);

beforeEach(() => {
  h.calls = [];
});

describe("accepting a suggested topic", () => {
  it("is refused in words when another topic already has that name, as Add and Edit are", async () => {
    const tree = new Tree(() => <TopicsListCard />).render();
    await tree.settle();
    const field = () => tree.one((n) => n.tag === "input" && n.props.value === "Globex", "suggestion name");
    call(field(), "onChange", { target: { value: " acme " } });
    tree.render();
    call(tree.all((n) => n.tag === "button" && text(n).trim() === "Add").pop()!, "onClick");
    tree.render();
    expect(cmds("accept_topic_suggestion")).toEqual([]);
    expect(tree.has("Acme is already a topic.")).toBe(true);
    // Typing another name clears the refusal and the button works.
    call(tree.one((n) => n.tag === "input" && n.props.value === " acme "), "onChange", { target: { value: "Globex" } });
    tree.render();
    expect(tree.has("is already a topic.")).toBe(false);
    call(tree.all((n) => n.tag === "button" && text(n).trim() === "Add").pop()!, "onClick");
    expect(cmds("accept_topic_suggestion")).toEqual([["accept_topic_suggestion", { topicId: 5, name: "Globex" }]]);
  });
});
