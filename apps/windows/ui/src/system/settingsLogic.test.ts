import { describe, expect, it } from "vitest";
import { DIALOG_FOCUSABLE, breakAtSeparators, checkAddDraft, commitNumberDraft, dialogEnterAction, navFocusIndex, parseNumberDraft, secretToSave, secretView, trapTabTarget, typedConfirmMatches } from ".";

describe("the section list keys", () => {
  it("Down and Up move and wrap, Home and End go to the ends, other keys are left alone", () => {
    expect(navFocusIndex("ArrowDown", 0, 4)).toBe(1);
    expect(navFocusIndex("ArrowDown", 3, 4)).toBe(0);
    expect(navFocusIndex("ArrowUp", 0, 4)).toBe(3);
    expect(navFocusIndex("ArrowUp", 2, 4)).toBe(1);
    expect(navFocusIndex("Home", 2, 4)).toBe(0);
    expect(navFocusIndex("End", 0, 4)).toBe(3);
    expect(navFocusIndex("ArrowRight", 0, 4)).toBeNull();
    expect(navFocusIndex("Enter", 0, 4)).toBeNull();
    expect(navFocusIndex("ArrowDown", 0, 0)).toBeNull();
  });
});

describe("a number typed in a box", () => {
  const limits = { min: 1, max: 3650, integer: true };

  it("an empty box is a problem, not a number", () => {
    expect(parseNumberDraft("", limits)).toEqual({ ok: false, problem: "empty" });
    expect(parseNumberDraft("   ", limits)).toEqual({ ok: false, problem: "empty" });
  });

  it("text, signs, exponents and stray characters are not numbers", () => {
    for (const bad of ["abc", "12abc", "1e3", "--4", "+5", "1.2.3", "0x10", "٣"]) expect(parseNumberDraft(bad, limits), bad).toMatchObject({ ok: false });
  });

  it("out of range and fractions are refused, the limits themselves are fine", () => {
    expect(parseNumberDraft("0", limits)).toEqual({ ok: false, problem: "range" });
    expect(parseNumberDraft("3651", limits)).toEqual({ ok: false, problem: "range" });
    expect(parseNumberDraft("1", limits)).toEqual({ ok: true, value: 1 });
    expect(parseNumberDraft("3650", limits)).toEqual({ ok: true, value: 3650 });
    expect(parseNumberDraft("2.5", limits)).toEqual({ ok: false, problem: "fraction" });
    expect(parseNumberDraft("2,5", { min: 1 })).toEqual({ ok: true, value: 2.5 });
  });

  it("commits only a valid number that differs from the stored one", () => {
    expect(commitNumberDraft("", 30, limits)).toEqual({ emit: null, problem: "empty" });
    expect(commitNumberDraft("nope", 30, limits)).toEqual({ emit: null, problem: "invalid" });
    expect(commitNumberDraft("99999", 30, limits)).toEqual({ emit: null, problem: "range" });
    expect(commitNumberDraft("30", 30, limits)).toEqual({ emit: null, problem: null });
    expect(commitNumberDraft(" 45 ", 30, limits)).toEqual({ emit: 45, problem: null });
  });
});

describe("an amount of money typed in a box", () => {
  const money = { min: 0, integer: false, money: true };
  const read = (text: string) => parseNumberDraft(text, money);

  it("takes a dollar sign and a point or a short comma as the decimal mark", () => {
    expect(read("5")).toEqual({ ok: true, value: 5 });
    expect(read("$5")).toEqual({ ok: true, value: 5 });
    expect(read("$ 5")).toEqual({ ok: true, value: 5 });
    expect(read("5.50")).toEqual({ ok: true, value: 5.5 });
    expect(read("5,5")).toEqual({ ok: true, value: 5.5 });
    expect(read("5,25")).toEqual({ ok: true, value: 5.25 });
    expect(read("0")).toEqual({ ok: true, value: 0 });
  });

  it("never guesses what a thousands mark means: 1,000 is refused, not saved as 1.00", () => {
    for (const bad of ["1,000", "1,000.50", "1.000,50", "1,000,000", "$1,000"]) expect(read(bad), bad).toEqual({ ok: false, problem: "invalid" });
  });

  it("a point before exactly three digits is ambiguous like 1,000: refused, never read as a fraction or a thousand", () => {
    for (const bad of ["1.000", "$1.000", "12.345", "0.500", "-1.000", "$ 12.345"]) expect(read(bad), bad).toEqual({ ok: false, problem: "invalid" });
    expect(parseNumberDraft("1.000", { min: 1, max: 3650, integer: true })).toEqual({ ok: false, problem: "invalid" });
    expect(parseNumberDraft("1.000", {})).toEqual({ ok: false, problem: "invalid" });
  });

  it("one or two decimals stay valid; money refuses more than two decimals, a plain number does not", () => {
    expect(read("1.00")).toEqual({ ok: true, value: 1 });
    expect(read("1.5")).toEqual({ ok: true, value: 1.5 });
    expect(read("$1.50")).toEqual({ ok: true, value: 1.5 });
    // Four decimals is not an amount of money at 2 decimals, so money refuses it.
    expect(read("1.0000")).toEqual({ ok: false, problem: "invalid" });
    expect(read("2.5051")).toEqual({ ok: false, problem: "invalid" });
    // A plain (non-money) number has no such limit; only the three-digit shape is ambiguous.
    expect(parseNumberDraft("1.0000", {})).toEqual({ ok: true, value: 1 });
  });

  it("empty, negative and words are problems", () => {
    expect(read("")).toEqual({ ok: false, problem: "empty" });
    expect(read("$")).toEqual({ ok: false, problem: "empty" });
    expect(read("-1")).toEqual({ ok: false, problem: "range" });
    expect(read("abc")).toEqual({ ok: false, problem: "invalid" });
  });

  it("a whole-number box refuses 1,000 as well (it was read as 1)", () => {
    expect(parseNumberDraft("1,000", { min: 1, max: 3650, integer: true })).toEqual({ ok: false, problem: "invalid" });
  });

  it("commits through the same rule", () => {
    expect(commitNumberDraft("1,000", 25, money)).toEqual({ emit: null, problem: "invalid" });
    expect(commitNumberDraft("$30", 25, money)).toEqual({ emit: 30, problem: null });
  });
});

describe("the secret row", () => {
  it("has three faces", () => {
    expect(secretView(false, false)).toBe("empty");
    expect(secretView(false, true)).toBe("empty");
    expect(secretView(true, false)).toBe("saved");
    expect(secretView(true, true)).toBe("editing");
  });

  it("saves what was typed without stray spaces, and nothing for an empty box", () => {
    expect(secretToSave("  sk-example-not-a-key\n")).toBe("sk-example-not-a-key");
    expect(secretToSave("   ")).toBeNull();
    expect(secretToSave("")).toBeNull();
  });
});

describe("adding to a list", () => {
  it("refuses an empty entry and a repeat, ignoring case and edge spaces", () => {
    expect(checkAddDraft("  ")).toEqual({ ok: false, problem: "empty" });
    expect(checkAddDraft("Notes.exe", ["notes.exe "])).toEqual({ ok: false, problem: "duplicate" });
    expect(checkAddDraft("  Chat.exe ", ["notes.exe"])).toEqual({ ok: true, value: "Chat.exe" });
  });
});

describe("the dialog's keys", () => {
  const el = (id: string) => ({ id });
  const [a, b, c] = [el("a"), el("b"), el("c")];
  const box = el("box");

  it("Tab wraps from the last control to the first and Shift+Tab from the first to the last, fields included", () => {
    expect(trapTabTarget([a, b, c], c, box, false)).toBe(a);
    expect(trapTabTarget([a, b, c], a, box, true)).toBe(c);
    expect(trapTabTarget([a, b, c], b, box, false)).toBeNull();
    expect(trapTabTarget([a, b, c], b, box, true)).toBeNull();
  });

  it("with focus on the dialog itself, or outside every control, Tab brings it in", () => {
    expect(trapTabTarget([a, b, c], box, box, false)).toBe(a);
    expect(trapTabTarget([a, b, c], box, box, true)).toBe(c);
    expect(trapTabTarget([a, b, c], el("elsewhere"), box, false)).toBe(a);
    expect(trapTabTarget([], box, box, false)).toBeNull();
  });

  it("the trap reaches buttons, text boxes, selects, text areas, checkboxes and links, and skips disabled ones", () => {
    for (const part of ["button", "input", "select", "textarea", "a[href]"]) expect(DIALOG_FOCUSABLE).toContain(part);
    expect(DIALOG_FOCUSABLE).toContain(":not(:disabled)");
  });

  it("Return in a text box does nothing; on a button, a checkbox or a select it is that control's own", () => {
    expect(dialogEnterAction({ tag: "INPUT", type: "text" })).toBe("swallow");
    expect(dialogEnterAction({ tag: "input" })).toBe("swallow");
    expect(dialogEnterAction({ tag: "input", type: "password" })).toBe("swallow");
    expect(dialogEnterAction({ tag: "textarea" })).toBe("swallow");
    expect(dialogEnterAction({ tag: "input", type: "checkbox" })).toBe("native");
    expect(dialogEnterAction({ tag: "button" })).toBe("native");
    expect(dialogEnterAction({ tag: "select" })).toBe("native");
  });

  it("a typed confirmation matches only the word, exactly: edge spaces do not count", () => {
    expect(typedConfirmMatches("DELETE", "DELETE")).toBe(true);
    expect(typedConfirmMatches("  DELETE ", "DELETE")).toBe(false);
    expect(typedConfirmMatches(" DELETE", "DELETE")).toBe(false);
    expect(typedConfirmMatches("DELETE\n", "DELETE")).toBe(false);
    expect(typedConfirmMatches("delete", "DELETE")).toBe(false);
    expect(typedConfirmMatches("DELET", "DELETE")).toBe(false);
    expect(typedConfirmMatches("", "DELETE")).toBe(false);
    expect(typedConfirmMatches("", "")).toBe(false);
  });
});

describe("a path that can wrap at its folders", () => {
  it("adds a break opportunity after each separator and leaves the visible text alone", () => {
    const wrapped = breakAtSeparators("C:\\Users\\Sam\\Whimbrel Slides/2026");
    expect(wrapped.replace(/\u200b/g, "")).toBe("C:\\Users\\Sam\\Whimbrel Slides/2026");
    expect(wrapped.split("\u200b")).toHaveLength(5);
    expect(breakAtSeparators(wrapped)).toBe(wrapped);
  });
});
