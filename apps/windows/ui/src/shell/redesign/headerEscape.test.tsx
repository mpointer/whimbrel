// The header's search field (where it collapses to an icon, see searchCollapse.test.ts): driven with the stub tree. Ctrl+K's tick opens
// it; Escape in the empty field closes it, and the icon says so; Escape on the Ask button beside the field does nothing.

import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";

vi.mock("react", async (actual) => (await import("../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));
vi.mock("../../api", () => ({ openAiSettings: vi.fn() }));
vi.mock("../../library/redesign/askState", () => ({ useAskState: () => ({}) }));
vi.mock("../../library/redesign/askStore", () => ({ useContinuedConversation: () => null }));
vi.mock("../../library/redesign/routePreview", () => ({ useRoutePreview: () => null }));
vi.mock("../../library/redesign/findModel", async (actual) => ({ ...(await actual<Record<string, unknown>>()), askReadiness: () => "ready" }));
vi.mock("../../today/redesign/useHomeData", () => ({ useCaptureApps: () => ({ apps: [], load() {} }), usePeople: () => ({ people: [], load() {} }) }));

import { press, Tree, type TNode } from "../../testkit/tree";
import { setHeaderText } from "../../library/redesign/findStore";
import { HeaderRow } from "./HeaderRow";

const draw = (focusTick?: number) => {
  const inputRef = createRef<HTMLInputElement>() as { current: HTMLInputElement | null };
  return new Tree(() => <HeaderRow title="Meetings" capture={null} ask="field" placeholder="Search" filters={false} now={0} inputRef={inputRef as never} onFind={() => {}} focusTick={focusTick} />).render();
};
const row = (t: Tree) => t.one((n) => typeof n.props.className === "string" && n.props.className === "w-shell-header-row", "header row");
const toggle = (t: Tree) => t.one((n) => n.tag === "button" && n.props["aria-controls"] === "w-shell-search", "search icon");
const box = (t: Tree) => t.one((n) => n.props.id === "w-shell-search", "search box");
const field = (t: Tree) => t.one((n) => n.tag === "input", "field");
const askButton = (t: Tree): TNode => t.one((n) => n.tag === "button" && n.props.title !== undefined && String(n.props["aria-keyshortcuts"]) === "Control+Enter", "Ask button");

describe("the collapsed search field", () => {
  it("is closed, and the icon says so, until it is asked for", () => {
    setHeaderText("");
    const t = draw();
    expect(row(t).props["data-search"]).toBe("closed");
    expect(toggle(t).props["aria-expanded"]).toBe(false);
  });

  it("opens from the icon, and the icon says it is expanded", () => {
    setHeaderText("");
    const t = draw();
    (toggle(t).props.onClick as () => void)();
    t.render();
    expect(row(t).props["data-search"]).toBe("open");
    expect(toggle(t).props["aria-expanded"]).toBe(true);
  });

  it("opens on Ctrl+K (the tick), and asks for focus on the field", () => {
    setHeaderText("");
    const t = draw(1);
    expect(row(t).props["data-search"]).toBe("open");
    expect(toggle(t).props["aria-expanded"]).toBe(true);
  });

  it("Escape in the empty field closes it and the icon says so", () => {
    setHeaderText("");
    const t = draw(1);
    expect(row(t).props["data-search"]).toBe("open");
    const r = press(box(t), "Escape", { tag: "input" });
    t.render();
    expect(r).toBeDefined();
    expect(row(t).props["data-search"]).toBe("closed");
    expect(toggle(t).props["aria-expanded"]).toBe(false);
  });

  it("Escape with words in the field leaves it open", () => {
    setHeaderText("Northwind");
    const t = draw(1);
    press(box(t), "Escape", { tag: "input" });
    t.render();
    expect(row(t).props["data-search"]).toBe("open");
    expect(toggle(t).props["aria-expanded"]).toBe(true);
    setHeaderText("");
  });

  it("Escape on the Ask button does not close the field: focus is not in it", () => {
    setHeaderText("");
    const t = draw(1);
    expect(askButton(t)).toBeDefined();
    press(box(t), "Escape", { tag: "button" });
    t.render();
    expect(row(t).props["data-search"]).toBe("open");
    expect(toggle(t).props["aria-expanded"]).toBe(true);
    expect(field(t)).toBeDefined();
  });

  it("other keys never close it", () => {
    setHeaderText("");
    const t = draw(1);
    press(box(t), "Enter", { tag: "input" });
    press(box(t), "a", { tag: "input" });
    t.render();
    expect(row(t).props["data-search"]).toBe("open");
  });
});
