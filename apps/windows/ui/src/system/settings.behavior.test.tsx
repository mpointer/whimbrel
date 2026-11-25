import { describe, expect, it, vi } from "vitest";
import {
  WConfirmDialog,
  WInlineConfirm,
  WSettingsList,
  WSettingsNav,
  WSettingsNumberRow,
  WSettingsSecretRow,
  WSettingsTextRow,
} from ".";
import { Tree, press, text, type TNode } from "../testkit/tree";

vi.mock("react", async (actual) => (await import("../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));
vi.stubGlobal("document", { activeElement: null, getElementById: () => null });

const call = (node: TNode, prop: string, ...args: unknown[]) => (node.props[prop] as (...a: unknown[]) => void)(...args);
const change = (node: TNode, value: string) => call(node, "onChange", { target: { value } });
const textInput = (tree: Tree) => tree.one((n) => n.tag === "input" && n.props.type === "text", "text box");

describe("the section list, pressed", () => {
  const items = [
    { id: "a", label: "General" },
    { id: "b", label: "Capture", badge: 2, badgeLabel: "2 waiting" },
    { id: "c", label: "Storage" },
  ];
  const make = (selected: string, onSelect = vi.fn()) => new Tree(() => <WSettingsNav label="Settings sections" items={items} selected={selected} onSelect={onSelect} />).render();
  const stops = (tree: Tree) => tree.all((n) => n.tag === "button" && n.props.className === "w-snav-item").map((n) => n.props.tabIndex);

  it("is a labelled nav with one Tab stop, on the open section, which is marked as the current page", () => {
    const tree = make("b");
    expect(tree.one((n) => n.tag === "nav").props["aria-label"]).toBe("Settings sections");
    expect(stops(tree)).toEqual([-1, 0, -1]);
    const buttons = tree.all((n) => n.tag === "button");
    expect(buttons.map((b) => b.props["aria-current"])).toEqual([undefined, "page", undefined]);
  });

  it("Down, Up, Home and End move the one stop and do not open anything; Return (a click) opens", () => {
    const onSelect = vi.fn();
    const tree = make("b", onSelect);
    const [a, b, c] = tree.all((n) => n.tag === "button");
    expect(press(b, "ArrowDown").prevented).toBe(true);
    tree.render();
    expect(stops(tree)).toEqual([-1, -1, 0]);
    press(c, "Home");
    tree.render();
    expect(stops(tree)).toEqual([0, -1, -1]);
    press(a, "End");
    tree.render();
    expect(stops(tree)).toEqual([-1, -1, 0]);
    press(c, "ArrowUp");
    tree.render();
    expect(stops(tree)).toEqual([-1, 0, -1]);
    expect(onSelect).not.toHaveBeenCalled();
    call(tree.all((n) => n.tag === "button")[2], "onClick");
    expect(onSelect).toHaveBeenCalledWith("c");
  });

  it("leaves other keys alone", () => {
    const tree = make("a");
    expect(press(tree.all((n) => n.tag === "button")[0], "ArrowRight").prevented).toBe(false);
  });

  it("names a badge, and has the compact select with the same sections and the same name", () => {
    const tree = make("a");
    const badge = tree.one((n) => n.props.role === "img");
    expect(badge.props["aria-label"]).toBe("2 waiting");
    const select = tree.one((n) => n.tag === "select");
    expect(select.props["aria-label"]).toBe("Settings sections");
    expect(select.props.value).toBe("a");
    expect(tree.all((n) => n.tag === "option").map((o) => text(o))).toEqual(["General", "Capture (2 waiting)", "Storage"]);
  });
});

describe("the number row, pressed", () => {
  const make = (onCommit = vi.fn(), extra: Partial<Parameters<typeof WSettingsNumberRow>[0]> = {}) =>
    new Tree(() => <WSettingsNumberRow label="Screen text" value={30} min={1} max={3650} unit="days" onCommit={onCommit} {...extra} />).render();

  it("never writes an empty box: it says what is allowed and keeps the stored number", () => {
    const onCommit = vi.fn();
    const tree = make(onCommit);
    const box = textInput(tree);
    change(box, "");
    tree.render();
    call(textInput(tree), "onBlur");
    tree.render();
    expect(onCommit).not.toHaveBeenCalled();
    expect(tree.has("Enter a number from 1 to 3,650.")).toBe(true);
    expect(textInput(tree).props["aria-invalid"]).toBe(true);
    // Return in an empty box is the same.
    call(textInput(tree), "onBlur");
    press(textInput(tree), "Enter", { tag: "input", type: "text" });
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("never writes text or an out-of-range number", () => {
    const onCommit = vi.fn();
    const tree = make(onCommit);
    for (const bad of ["abc", "0", "9999", "1e2", "4.5"]) {
      change(textInput(tree), bad);
      tree.render();
      call(textInput(tree), "onBlur");
      tree.render();
    }
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("does not write on a keystroke, writes a valid change on blur and on Return, and not an unchanged number", () => {
    const onCommit = vi.fn();
    const tree = make(onCommit);
    change(textInput(tree), "4");
    change(textInput(tree), "45");
    tree.render();
    expect(onCommit).not.toHaveBeenCalled();
    call(textInput(tree), "onBlur");
    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenLastCalledWith(45);
    tree.render();
    change(textInput(tree), "60");
    tree.render();
    expect(press(textInput(tree), "Enter", { tag: "input", type: "text" })).toEqual({ prevented: true, stopped: true });
    expect(onCommit).toHaveBeenLastCalledWith(60);
    const again = make(onCommit);
    change(textInput(again), "30");
    again.render();
    call(textInput(again), "onBlur");
    expect(onCommit).toHaveBeenCalledTimes(2);
  });

  it("Escape puts the stored number back and writes nothing", () => {
    const onCommit = vi.fn();
    const tree = make(onCommit);
    change(textInput(tree), "77");
    tree.render();
    expect(textInput(tree).props.value).toBe("77");
    press(textInput(tree), "Escape", { tag: "input", type: "text" });
    tree.render();
    expect(textInput(tree).props.value).toBe("30");
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("with the companion checkbox on, the box is off and still shows the stored number; turning it off sends only the checkbox change", () => {
    const onCommit = vi.fn();
    const onCompanion = vi.fn();
    const tree = make(onCommit, { companion: { label: "Keep forever", checked: true, onChange: onCompanion } });
    const box = textInput(tree);
    expect(box.props.disabled).toBe(true);
    expect(box.props.value).toBe("30");
    const boxes = tree.all((n) => n.tag === "input" && n.props.type === "checkbox");
    expect(boxes).toHaveLength(1);
    call(boxes[0], "onChange", { target: { checked: false } });
    expect(onCompanion).toHaveBeenCalledWith(false);
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("ties the label to the box, and the unit and a problem to it by description", () => {
    const tree = make();
    const box = textInput(tree);
    const label = tree.one((n) => n.tag === "label" && n.props.htmlFor === box.props.id);
    expect(text(label)).toBe("Screen text");
    expect(String(box.props["aria-describedby"])).toContain("-x");
  });
});

describe("the text row, pressed", () => {
  it("writes on blur only when the text changed, and not an empty box when it is required", () => {
    const onCommit = vi.fn();
    const tree = new Tree(() => <WSettingsTextRow label="Name" value="Sam" onCommit={onCommit} required />).render();
    change(textInput(tree), "");
    tree.render();
    call(textInput(tree), "onBlur");
    tree.render();
    expect(onCommit).not.toHaveBeenCalled();
    expect(tree.has("This can't be empty.")).toBe(true);
    change(textInput(tree), "Alex");
    tree.render();
    call(textInput(tree), "onBlur");
    expect(onCommit).toHaveBeenCalledWith("Alex");
  });
});

describe("the secret row", () => {
  const passwordBox = (tree: Tree) => tree.all((n) => n.tag === "input" && n.props.type === "password");

  it("empty: a password box, and Save is off until something is typed; Save sends the trimmed value", () => {
    const onSave = vi.fn();
    const tree = new Tree(() => <WSettingsSecretRow label="Example key" saved={false} onSave={onSave} />).render();
    expect(passwordBox(tree)).toHaveLength(1);
    expect(tree.button("Save").props.disabled).toBe(true);
    change(passwordBox(tree)[0], "  sk-example-not-a-key ");
    tree.render();
    expect(tree.button("Save").props.disabled).toBe(false);
    call(tree.button("Save"), "onClick");
    expect(onSave).toHaveBeenCalledWith("sk-example-not-a-key");
    expect(tree.hasButton("Remove")).toBe(false);
  });

  it("Return in the box saves, and is not passed on to the page", () => {
    const onSave = vi.fn();
    const tree = new Tree(() => <WSettingsSecretRow label="Example key" saved={false} onSave={onSave} />).render();
    change(passwordBox(tree)[0], "sk-example-not-a-key");
    tree.render();
    expect(press(passwordBox(tree)[0], "Enter", { tag: "input", type: "password" })).toEqual({ prevented: true, stopped: true });
    expect(onSave).toHaveBeenCalledWith("sk-example-not-a-key");
  });

  it("editing: Replace opens the box with Cancel; Cancel and Escape go back to the saved line with nothing typed kept", () => {
    const onSave = vi.fn();
    const tree = new Tree(() => <WSettingsSecretRow label="Example key" saved onSave={onSave} onRemove={() => {}} />).render();
    expect(passwordBox(tree)).toHaveLength(0);
    call(tree.button("Replace"), "onClick");
    tree.render();
    expect(passwordBox(tree)).toHaveLength(1);
    change(passwordBox(tree)[0], "sk-example-not-a-key");
    tree.render();
    press(passwordBox(tree)[0], "Escape", { tag: "input", type: "password" });
    tree.render();
    expect(passwordBox(tree)).toHaveLength(0);
    expect(tree.hasButton("Replace")).toBe(true);
    expect(tree.has("sk-example-not-a-key")).toBe(false);
    expect(onSave).not.toHaveBeenCalled();
  });

  it("clears the typed key and leaves editing as soon as a save succeeds, even while `saved` stays true (Replace, then Save, twice)", async () => {
    const onSave = vi.fn(async () => true);
    const tree = new Tree(() => <WSettingsSecretRow label="Example key" saved onSave={onSave} onRemove={() => {}} />).render();
    for (const key of ["sk-first-not-a-key", "sk-second-not-a-key"]) {
      call(tree.button("Replace"), "onClick");
      tree.render();
      change(passwordBox(tree)[0], key);
      tree.render();
      expect(passwordBox(tree)[0].props.value).toBe(key);
      call(tree.button("Save"), "onClick");
      expect(onSave).toHaveBeenLastCalledWith(key);
      await tree.settle();
      expect(passwordBox(tree)).toHaveLength(0);
      expect(tree.hasButton("Replace")).toBe(true);
      // Open the box again: nothing from the last time is in it.
      call(tree.button("Replace"), "onClick");
      tree.render();
      expect(passwordBox(tree)[0].props.value).toBe("");
      call(tree.button("Cancel"), "onClick");
      tree.render();
    }
    expect(onSave).toHaveBeenCalledTimes(2);
  });

  it("only `true` is a success: a caller that returns nothing (or anything else) keeps what was typed", async () => {
    for (const result of [undefined, false, null, "ok", 1]) {
      const onSave = vi.fn(async () => result as never);
      const tree = new Tree(() => <WSettingsSecretRow label="Example key" saved={false} onSave={onSave} />).render();
      change(passwordBox(tree)[0], "sk-example-not-a-key");
      tree.render();
      call(tree.button("Save"), "onClick");
      await tree.settle();
      expect(passwordBox(tree)[0].props.value, String(result)).toBe("sk-example-not-a-key");
    }
    const sync = vi.fn(() => undefined as never);
    const tree = new Tree(() => <WSettingsSecretRow label="Example key" saved={false} onSave={sync} />).render();
    change(passwordBox(tree)[0], "sk-example-not-a-key");
    tree.render();
    call(tree.button("Save"), "onClick");
    await tree.settle();
    expect(passwordBox(tree)[0].props.value).toBe("sk-example-not-a-key");
  });

  it("keeps what was typed when the save fails, so it can be corrected", async () => {
    const onSave = vi.fn(async () => false);
    const tree = new Tree(() => <WSettingsSecretRow label="Example key" saved={false} onSave={onSave} />).render();
    change(passwordBox(tree)[0], "sk-wrong-not-a-key");
    tree.render();
    call(tree.button("Save"), "onClick");
    await tree.settle();
    expect(passwordBox(tree)[0].props.value).toBe("sk-wrong-not-a-key");
    const rejects = vi.fn(async () => {
      throw new Error("no");
    });
    const tree2 = new Tree(() => <WSettingsSecretRow label="Example key" saved={false} onSave={rejects} />).render();
    change(passwordBox(tree2)[0], "sk-wrong-not-a-key");
    tree2.render();
    call(tree2.button("Save"), "onClick");
    await tree2.settle();
    expect(passwordBox(tree2)[0].props.value).toBe("sk-wrong-not-a-key");
  });

  it("Replace and Remove are named for their key", () => {
    const tree = new Tree(() => <WSettingsSecretRow label="Anthropic API key" saved onSave={() => true} onRemove={() => {}} />).render();
    expect(tree.button("Replace").props["aria-label"]).toBe("Replace Anthropic API key");
    expect(tree.button("Remove").props["aria-label"]).toBe("Remove Anthropic API key");
  });

  it("Remove asks first when given a question, and removes only on the second press", () => {
    const onRemove = vi.fn();
    const tree = new Tree(() => <WSettingsSecretRow label="Example key" saved onSave={() => true} onRemove={onRemove} removeQuestion="Remove this key?" />).render();
    call(tree.button("Remove"), "onClick");
    tree.render();
    expect(onRemove).not.toHaveBeenCalled();
    expect(tree.one((n) => n.props.role === "group").props["aria-label"]).toBe("Remove this key?");
    call(tree.one((n) => n.tag === "button" && n.props.variant === undefined && text(n).trim() === "Remove"), "onClick");
    expect(onRemove).toHaveBeenCalledTimes(1);
  });
});

describe("a list with an add field", () => {
  const make = (onAdd: (v: string) => string | null | void | Promise<string | null | void>, existing: string[] = []) =>
    new Tree(() => (
      <WSettingsList label="Excluded apps" isEmpty empty="No apps excluded." add={{ label: "App to exclude", onAdd, existing }} />
    )).render();
  const field = (tree: Tree) => tree.one((n) => n.tag === "input" && n.props["aria-label"] === "App to exclude");

  it("shows the empty line, a list with a name, and the add field with a name", () => {
    const tree = make(vi.fn());
    expect(tree.has("No apps excluded.")).toBe(true);
    expect(field(tree).props.type).toBe("text");
  });

  it("refuses an empty entry with a line and does not call onAdd", async () => {
    const onAdd = vi.fn();
    const tree = make(onAdd);
    call(tree.button("Add"), "onClick");
    await tree.settle();
    expect(onAdd).not.toHaveBeenCalled();
    expect(tree.has("Enter a value to add.")).toBe(true);
  });

  it("refuses a repeat, and accepts a new entry on Return, clearing the field", async () => {
    const onAdd = vi.fn();
    const tree = make(onAdd, ["Notes.exe"]);
    change(field(tree), "notes.exe");
    tree.render();
    call(tree.button("Add"), "onClick");
    await tree.settle();
    expect(onAdd).not.toHaveBeenCalled();
    expect(tree.has("notes.exe is already in the list.")).toBe(true);
    change(field(tree), "  Chat.exe ");
    tree.render();
    expect(press(field(tree), "Enter", { tag: "input", type: "text" }).prevented).toBe(true);
    await tree.settle();
    expect(onAdd).toHaveBeenCalledWith("Chat.exe");
    expect(field(tree).props.value).toBe("");
  });

  it("keeps the text and shows the sentence when onAdd says no", async () => {
    const tree = make(() => "Windows would not let Whimbrel list that app.");
    change(field(tree), "Chat.exe");
    tree.render();
    call(tree.button("Add"), "onClick");
    await tree.settle();
    expect(tree.has("Windows would not let Whimbrel list that app.")).toBe(true);
    expect(field(tree).props.value).toBe("Chat.exe");
  });
});

describe("the confirm dialog, pressed", () => {
  const typedTree = (onConfirm = vi.fn(), onCancel = vi.fn()) =>
    new Tree(() => (
      <WConfirmDialog title="Delete everything?" body="It cannot be undone." confirmLabel="Delete everything" typed={{ word: "DELETE", label: "Type DELETE" }} fields={<input type="checkbox" />} onConfirm={onConfirm} onCancel={onCancel} />
    )).render();

  it("keeps the confirm button off until the word is typed, then turns it on", () => {
    const onConfirm = vi.fn();
    const tree = typedTree(onConfirm);
    expect(tree.button("Delete everything").props.disabled).toBe(true);
    call(tree.button("Delete everything"), "onClick");
    expect(onConfirm).not.toHaveBeenCalled();
    change(textInput(tree), "delete");
    tree.render();
    expect(tree.button("Delete everything").props.disabled).toBe(true);
    change(textInput(tree), "DELETE");
    tree.render();
    expect(tree.button("Delete everything").props.disabled).toBe(false);
    call(tree.button("Delete everything"), "onClick");
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("Return in the box does nothing, even when the word matches, and does not reach the page", () => {
    const onConfirm = vi.fn();
    const tree = typedTree(onConfirm);
    change(textInput(tree), "DELETE");
    tree.render();
    const box = tree.one((n) => n.props.role === "alertdialog");
    expect(press(box, "Enter", { tag: "input", type: "text" })).toEqual({ prevented: true, stopped: true });
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("starts in the typed box, never on the destructive button; Escape cancels", () => {
    const onCancel = vi.fn();
    const tree = typedTree(vi.fn(), onCancel);
    expect(textInput(tree).props["data-initial"]).toBe(true);
    expect(tree.button("Delete everything").props["data-initial"]).toBeUndefined();
    press(tree.one((n) => n.props.role === "alertdialog"), "Escape");
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("without a typed word, Cancel is where focus starts and the confirm button is destructive", () => {
    const tree = new Tree(() => <WConfirmDialog title="Delete this skin?" confirmLabel="Delete skin" onConfirm={() => {}} onCancel={() => {}} />).render();
    expect(tree.button("Cancel").props["data-initial"]).toBe(true);
    expect(tree.button("Delete skin").props["data-initial"]).toBeUndefined();
    expect(String(tree.button("Delete skin").props.className)).toContain("w-button-destructive");
  });

  it("passes what was typed to onConfirm, and does not treat a word with spaces around it as a match", () => {
    const onConfirm = vi.fn();
    const tree = typedTree(onConfirm);
    change(textInput(tree), " DELETE ");
    tree.render();
    expect(tree.button("Delete everything").props.disabled).toBe(true);
    change(textInput(tree), "DELETE");
    tree.render();
    call(tree.button("Delete everything"), "onClick");
    expect(onConfirm).toHaveBeenCalledWith("DELETE");
  });

  it("while busy no control is disabled (focus stays in the dialog) and presses do nothing", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    const tree = new Tree(() => <WConfirmDialog title="T" confirmLabel="Go" onConfirm={onConfirm} onCancel={onCancel} busy />).render();
    expect(tree.button("Go").props.disabled).toBeFalsy();
    expect(tree.button("Go").props["aria-disabled"]).toBe(true);
    expect(tree.button("Cancel").props.disabled).toBeFalsy();
    call(tree.button("Go"), "onClick");
    call(tree.button("Cancel"), "onClick");
    expect(onConfirm).not.toHaveBeenCalled();
    expect(onCancel).not.toHaveBeenCalled();
    const inline = vi.fn();
    const t2 = new Tree(() => <WInlineConfirm question="Q?" confirmLabel="Delete" onConfirm={inline} onCancel={onCancel} busy />).render();
    expect(t2.button("Delete").props.disabled).toBeFalsy();
    call(t2.button("Delete"), "onClick");
    expect(inline).not.toHaveBeenCalled();
  });

  it("is off while busy or while another condition is unmet", () => {
    const tree = new Tree(() => <WConfirmDialog title="T" confirmLabel="Go" onConfirm={() => {}} onCancel={() => {}} confirmDisabled />).render();
    expect(tree.button("Go").props.disabled).toBe(true);
  });
});

describe("the inline confirm", () => {
  it("is a named group with the question, a destructive button and Cancel, and Cancel is the one focus goes to", () => {
    const tree = new Tree(() => <WInlineConfirm question="Delete this skin?" confirmLabel="Delete" onConfirm={() => {}} onCancel={() => {}} />).render();
    const group = tree.one((n) => n.props.role === "group");
    expect(group.props["aria-label"]).toBe("Delete this skin?");
    expect(tree.button("Cancel").props["data-cancel"]).toBe("");
    expect(tree.button("Delete").props["data-cancel"]).toBeUndefined();
    expect(String(tree.button("Delete").props.className)).toContain("w-button-destructive");
  });

  it("Escape cancels, and the Delete button is not touched", () => {
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    const tree = new Tree(() => <WInlineConfirm question="Delete this skin?" confirmLabel="Delete" onConfirm={onConfirm} onCancel={onCancel} />).render();
    expect(press(tree.one((n) => n.props.role === "group"), "Escape").stopped).toBe(true);
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("gives focus back to the button it replaced after Cancel", () => {
    vi.useFakeTimers();
    const focus = vi.fn();
    vi.stubGlobal("document", { activeElement: null, getElementById: (id: string) => (id === "rm" ? { focus } : null) });
    const tree = new Tree(() => <WInlineConfirm question="Q?" confirmLabel="Delete" onConfirm={() => {}} onCancel={() => {}} returnFocusTo="rm" />).render();
    call(tree.button("Cancel"), "onClick");
    vi.runAllTimers();
    expect(focus).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
    vi.stubGlobal("document", { activeElement: null, getElementById: () => null });
  });
});
