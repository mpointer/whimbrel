import { describe, expect, it } from "vitest";
import { recorderStep } from "./hotkeyRecorder";

const press = (over: Partial<Parameters<typeof recorderStep>[0]>) => recorderStep({ key: "k", code: "KeyK", ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...over });

describe("the shortcut recorder", () => {
  it("Escape cancels", () => {
    expect(press({ key: "Escape", code: "Escape" })).toEqual({ kind: "cancel" });
    expect(press({ key: "Escape", code: "Escape", ctrlKey: true })).toEqual({ kind: "cancel" });
  });
  it("a modifier alone keeps waiting, and so does a repeat of the key that started it", () => {
    expect(press({ key: "Control", code: "ControlLeft", ctrlKey: true })).toEqual({ kind: "waiting" });
    expect(press({ key: "Enter", code: "Enter", repeat: true })).toEqual({ kind: "waiting" });
  });
  it("refuses a key with no modifier, with the same words as the old card", () => {
    const r = press({});
    expect(r).toEqual({ kind: "refused", reason: expect.stringContaining("has no modifier key, so it would block normal typing. Include Ctrl or Alt.") });
  });
  it("refuses Shift or Windows alone with the shared-shortcut reason", () => {
    for (const over of [{ shiftKey: true }, { metaKey: true }]) {
      const r = press(over);
      expect(r.kind).toBe("refused");
      if (r.kind === "refused") expect(r.reason).toContain("would take a shortcut apps or Windows use themselves");
    }
  });
  it("applies Ctrl+Alt+Space and Ctrl+K", () => {
    expect(press({ key: " ", code: "Space", ctrlKey: true, altKey: true })).toEqual({ kind: "apply", accelerator: "Control+Alt+Space" });
    expect(press({ ctrlKey: true })).toEqual({ kind: "apply", accelerator: "Control+KeyK" });
    expect(press({ shiftKey: true, metaKey: true })).toEqual({ kind: "apply", accelerator: "Shift+Super+KeyK" });
  });

  it("Tab ends the recording and is left alone, with or without Shift; Ctrl+Tab is still a key to record", () => {
    expect(press({ key: "Tab", code: "Tab" })).toEqual({ kind: "leave" });
    expect(press({ key: "Tab", code: "Tab", shiftKey: true })).toEqual({ kind: "leave" });
    expect(press({ key: "Tab", code: "Tab", ctrlKey: true }).kind).not.toBe("leave");
  });
});
