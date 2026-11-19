import { describe, expect, it, vi } from "vitest";
import { WDialog, WFlowPage, WMoreMenu, WNumberedList, WSegmented, WTextField } from ".";
import { Tree, press } from "../testkit/tree";

vi.mock("react", async (actual) => (await import("../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));

describe("a flow page, pressed", () => {
  it("Return runs the primary action and Escape goes back, from the page's own keys", () => {
    const onPrimary = vi.fn();
    const onBack = vi.fn();
    const tree = new Tree(() => <WFlowPage title="T" onPrimary={onPrimary} onBack={onBack} />).render();
    const main = tree.one((n) => n.tag === "main");
    expect(press(main, "Enter", { tag: "input", type: "password" }).prevented).toBe(true);
    expect(onPrimary).toHaveBeenCalledTimes(1);
    press(main, "Escape", { tag: "input" });
    expect(onBack).toHaveBeenCalledTimes(1);
    press(main, "Enter", { tag: "button" });
    press(main, "Enter", { tag: "div", inDialog: true });
    expect(onPrimary).toHaveBeenCalledTimes(1);
  });

  it("with no primary action allowed, Return does nothing and is not swallowed", () => {
    const tree = new Tree(() => <WFlowPage title="T" onBack={vi.fn()} />).render();
    expect(press(tree.one((n) => n.tag === "main"), "Enter", { tag: "input" }).prevented).toBe(false);
  });
});

describe("a text field, pressed", () => {
  it("Return in a field that takes it runs its own action and nothing else", () => {
    const onEnter = vi.fn();
    const tree = new Tree(() => <WTextField label="Passphrase" value="" onChange={() => {}} onEnter={onEnter} />).render();
    expect(press(tree.field("Passphrase"), "Enter", { tag: "input" }).prevented).toBe(true);
    expect(onEnter).toHaveBeenCalledTimes(1);
  });
});

describe("a protected list, pressed", () => {
  it("refuses copy, cut, the context menu and drag, and has no button", () => {
    const words = Array.from({ length: 24 }, (_, i) => `word${i + 1}`);
    const tree = new Tree(() => <WNumberedList items={words} label="x" protectedFromCopy />).render();
    const list = tree.one((n) => n.tag === "ol");
    for (const handler of ["onCopy", "onCut", "onContextMenu", "onDragStart"]) {
      let prevented = false;
      (list.props[handler] as (e: unknown) => void)({ preventDefault: () => (prevented = true) });
      expect(prevented, handler).toBe(true);
    }
    expect(tree.all((n) => n.tag === "button")).toHaveLength(0);
    expect(String(list.props.className)).toContain("w-numbered-protected");
  });
});

describe("the segmented choice", () => {
  it("is a radio group with one tab stop, and the arrow keys choose", () => {
    const onChange = vi.fn();
    const tree = new Tree(() => (
      <WSegmented label="Recovery key type" value="file" onChange={onChange} options={[{ value: "file", label: "Recovery file" }, { value: "words", label: "24 words" }]} />
    )).render();
    const [file, words] = tree.all((n) => n.props.role === "radio");
    expect(file.props["aria-checked"]).toBe(true);
    expect(file.props.tabIndex).toBe(0);
    expect(words.props.tabIndex).toBe(-1);
    press(file, "ArrowRight");
    expect(onChange).toHaveBeenCalledWith("words");
    press(words, "ArrowLeft");
    expect(onChange).toHaveBeenLastCalledWith("file");
  });
});

describe("the dialog", () => {
  vi.stubGlobal("document", { activeElement: null });
  it("is a named alert dialog, and Escape cancels it without the page seeing the key", () => {
    const onCancel = vi.fn();
    const tree = new Tree(() => (
      <WDialog title="Start over with a new recovery key?" body="Any recovery file you made will not work." onCancel={onCancel}>
        <button>Cancel</button>
      </WDialog>
    )).render();
    const box = tree.one((n) => n.props.role === "alertdialog");
    expect(box.props["aria-modal"]).toBe("true");
    expect(box.props["aria-labelledby"]).toBeTruthy();
    expect(press(box, "Escape")).toEqual({ prevented: true, stopped: true });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});

describe("more options, pressed", () => {
  it("opens to the choices as menu items that act", () => {
    const put = vi.fn();
    const items = [{ key: "a", label: "Put it back", onPress: put }, { key: "b", label: "Reset", onPress: vi.fn() }];
    const tree = new Tree(() => <WMoreMenu items={items} />).render();
    (tree.button("More options").props.onClick as () => void)();
    tree.render();
    expect(tree.all((n) => n.props.role === "menuitem")).toHaveLength(2);
    (tree.button("Put it back").props.onClick as () => void)();
    expect(put).toHaveBeenCalledTimes(1);
  });
});
