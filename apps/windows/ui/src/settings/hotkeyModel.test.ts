import { describe, expect, it } from "vitest";
import { changeResultLine, DEFAULT_SHORTCUT, displayShortcut, hotkeyStatusLine, recordKey, type KeyPress } from "./hotkeyModel";

const press = (code: string, mods: Partial<KeyPress> = {}): KeyPress => ({ code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods });

describe("displayShortcut", () => {
  it("shows the default as Ctrl+Alt+Space", () => {
    expect(displayShortcut(DEFAULT_SHORTCUT)).toBe("Ctrl+Alt+Space");
  });
  it("shortens key codes", () => {
    expect(displayShortcut("Control+Shift+KeyK")).toBe("Ctrl+Shift+K");
    expect(displayShortcut("Alt+Digit1")).toBe("Alt+1");
    expect(displayShortcut("Super+Alt+ArrowUp")).toBe("Win+Alt+Up");
  });
});

describe("hotkeyStatusLine", () => {
  it("says registered with the shortcut", () => {
    expect(hotkeyStatusLine({ shortcut: DEFAULT_SHORTCUT, registered: true, error: null })).toBe("Registered: Ctrl+Alt+Space");
  });
  it("says why not, and points to the tray item", () => {
    const line = hotkeyStatusLine({ shortcut: DEFAULT_SHORTCUT, registered: false, error: "HotKey already registered" });
    expect(line).toMatch(/^Ctrl\+Alt\+Space couldn't be registered/);
    expect(line).toContain("(HotKey already registered)");
    expect(line).toContain("Ask about this window…");
  });
});

describe("changeResultLine", () => {
  const current = { shortcut: DEFAULT_SHORTCUT, registered: true, error: null };
  it("is null when the new shortcut registered", () => {
    expect(changeResultLine({ shortcut: "Control+Alt+KeyK", registered: true, error: null }, current)).toBeNull();
  });
  it("names the refused shortcut and the one still in use", () => {
    const line = changeResultLine({ shortcut: "Control+Alt+KeyK", registered: false, error: "taken" }, current);
    expect(line).toBe("Ctrl+Alt+K couldn't be registered: another app may already use it. (taken) Still using Ctrl+Alt+Space.");
  });
});

describe("recordKey", () => {
  it("waits through modifier presses", () => {
    expect(recordKey(press("ControlLeft", { ctrlKey: true }))).toEqual({ kind: "waiting" });
    expect(recordKey(press("AltRight", { altKey: true, ctrlKey: true }))).toEqual({ kind: "waiting" });
  });
  it("records Ctrl or Alt combinations in the plugin's form", () => {
    expect(recordKey(press("Space", { ctrlKey: true, altKey: true }))).toEqual({ kind: "shortcut", accelerator: "Control+Alt+Space" });
    expect(recordKey(press("KeyK", { altKey: true }))).toEqual({ kind: "shortcut", accelerator: "Alt+KeyK" });
    expect(recordKey(press("KeyK", { shiftKey: true, metaKey: true }))).toEqual({ kind: "shortcut", accelerator: "Shift+Super+KeyK" });
  });
  it("refuses a key with no modifier, or Shift or Win alone, and says why", () => {
    const none = recordKey(press("KeyK"));
    expect(none.kind).toBe("refused");
    expect(none.kind === "refused" && none.reason).toMatch(/^K has no modifier key/);
    const shift = recordKey(press("KeyK", { shiftKey: true }));
    expect(shift.kind === "refused" && shift.reason).toMatch(/^Shift\+K would take a shortcut/);
    expect(recordKey(press("KeyE", { metaKey: true })).kind).toBe("refused");
  });
});
