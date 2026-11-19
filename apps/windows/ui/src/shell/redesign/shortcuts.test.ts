import { describe, expect, it } from "vitest";
import type { TabId } from "../../tabs";
import { liveShortcutOf, shortcutHint, shortcutOf } from "./shortcuts";

const order: TabId[] = ["today", "meetings", "library", "topics", "automations", "settings"];
const key = (k: string, extra: Record<string, boolean> = {}) => ({ key: k, ctrlKey: true, ...extra });

describe("shortcutOf", () => {
  it("maps Ctrl+1 to Ctrl+6 to the sections in sidebar order", () => {
    expect(order.map((_, i) => shortcutOf(key(String(i + 1)), order))).toEqual(order.map((id) => ({ kind: "section", id })));
  });
  it("maps Ctrl+K to search, either case", () => {
    expect(shortcutOf(key("k"), order)).toEqual({ kind: "search" });
    expect(shortcutOf(key("K"), order)).toEqual({ kind: "search" });
  });
  it("ignores keys that are not exactly Ctrl and the key", () => {
    expect(shortcutOf({ key: "1", ctrlKey: false }, order)).toBeNull();
    expect(shortcutOf(key("1", { altKey: true }), order)).toBeNull();
    expect(shortcutOf(key("1", { shiftKey: true }), order)).toBeNull();
    expect(shortcutOf(key("k", { metaKey: true }), order)).toBeNull();
    expect(shortcutOf(key("7"), order)).toBeNull();
    expect(shortcutOf(key("0"), order)).toBeNull();
    expect(shortcutOf(key("a"), order)).toBeNull();
  });
  it("writes the hint the sidebar shows", () => expect(shortcutHint(0)).toBe("Ctrl+1"));
});

describe("liveShortcutOf", () => {
  it("maps Ctrl+Shift+P to pause and Ctrl+Shift+M to mark, either case", () => {
    expect(liveShortcutOf(key("P", { shiftKey: true }))).toBe("pause");
    expect(liveShortcutOf(key("p", { shiftKey: true }))).toBe("pause");
    expect(liveShortcutOf(key("M", { shiftKey: true }))).toBe("mark");
  });
  it("needs Ctrl and Shift together and nothing else", () => {
    expect(liveShortcutOf(key("p"))).toBeNull();
    expect(liveShortcutOf(key("m"))).toBeNull();
    expect(liveShortcutOf({ key: "P", ctrlKey: false, shiftKey: true })).toBeNull();
    expect(liveShortcutOf(key("P", { shiftKey: true, altKey: true }))).toBeNull();
    expect(liveShortcutOf(key("P", { shiftKey: true, metaKey: true }))).toBeNull();
    expect(liveShortcutOf(key("K", { shiftKey: true }))).toBeNull();
  });
  it("leaves the section shortcuts alone", () => {
    expect(shortcutOf(key("P", { shiftKey: true }), order)).toBeNull();
  });
});
