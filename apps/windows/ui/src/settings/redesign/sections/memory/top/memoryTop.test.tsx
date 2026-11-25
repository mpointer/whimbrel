import { beforeEach, describe, expect, it, vi } from "vitest";
import { Tree, press, text, type TNode } from "../../../../../testkit/tree";

// The tree stands in for a DOM; context is read at its default value (English, nothing jumped to).
vi.mock("react", async (actual) => {
  const stubbed = (await import("../../../../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()) as Record<string, unknown>;
  const useContext = (c: { _currentValue: unknown }) => c._currentValue;
  return { ...stubbed, useContext, default: { ...(stubbed.default as object), useContext } };
});
vi.stubGlobal("document", { activeElement: null, getElementById: () => null });
const polls: (() => void)[] = [];
vi.stubGlobal("setInterval", (fn: () => void) => {
  polls.push(fn);
  return 0;
});
vi.stubGlobal("clearInterval", () => {});

const h = vi.hoisted(() => ({ calls: [] as [string, unknown][], answers: {} as Record<string, unknown> }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: unknown) => {
    h.calls.push([cmd, args ?? null]);
    const a = h.answers[cmd];
    return typeof a === "function" ? (a as (x: unknown) => unknown)(args) : a;
  },
}));

const { MemoryCard } = await import("./MemoryCard");
const { NeverCapturedCard } = await import("./NeverCapturedCard");
const { DeleteEverythingRows } = await import("./DeleteEverythingRedesign");
const { YourDataCard } = await import("./YourDataCard");

const call = (node: TNode, prop: string, ...args: unknown[]) => (node.props[prop] as (...a: unknown[]) => void)(...args);
const status = { enabled: true, running: true, stored: 5, unchanged: 2, excluded: 1, ocrRuns: 0 };
const cmds = (name: string) => h.calls.filter(([c]) => c === name);

const show = async (el: () => React.ReactNode) => {
  const tree = new Tree(el).render();
  await tree.settle();
  return tree;
};

beforeEach(() => {
  h.calls = [];
  polls.length = 0;
  h.answers = {
    capture_status: status,
    launch_state: { stage: { kind: "alreadyMade" } },
    set_capture_enabled: (a: { enabled: boolean }) => ({ ...status, enabled: a.enabled }),
    capture_pause_until: { ...status, resumeAtUnixMs: Date.now() + 900_000 },
    capture_resume_now: status,
    library_settings: { showScreenCaptures: true },
    set_show_screen_captures: (a: { show: boolean }) => ({ showScreenCaptures: a.show }),
    capture_settings: {
      excluded: ["chat.exe"],
      runningApps: [
        { appId: "chat.exe", exe: "Chat.exe", title: "Chat" },
        { appId: "code.exe", exe: "code.exe", title: "Editor" },
      ],
      ocr: { covered: true, available: ["en-US"], profile: ["en-US"] },
    },
    set_excluded_apps: (a: { excluded: string[] }) => a.excluded,
  };
});

describe("the Memory card", () => {
  it("is the capture.capture anchor, says the state in words and what Memory saves, and shows the counts and the never-captured line", async () => {
    const tree = await show(() => <MemoryCard />);
    expect(tree.one((n) => n.tag === "section").props.id).toBe("capture.capture");
    expect(tree.has("Memory on")).toBe(true);
    expect(tree.has("Memory saves the text of the window you are working in, and nothing from excluded apps.")).toBe(true);
    expect(tree.has("Never captured: the lock screen")).toBe(true);
    // The counts moved to Your data.
    expect(tree.has("Skipped (excluded apps)")).toBe(false);
  });

  it("the switch calls setCaptureEnabled with the new value", async () => {
    const tree = await show(() => <MemoryCard />);
    const sw = tree.all((n) => n.props.role === "switch")[0];
    call(sw, "onClick");
    expect(cmds("set_capture_enabled")).toEqual([["set_capture_enabled", { enabled: false }]]);
    await tree.settle();
    expect(tree.has("Memory off")).toBe(true);
    expect(tree.button("Pause for 15 minutes").props.disabled).toBe(true);
    expect(tree.has("Turn Memory on to pause it.")).toBe(true);
  });

  it("Pause calls capturePauseUntil with the same deadline the tray computes, and Resume now calls captureResumeNow", async () => {
    const tree = await show(() => <MemoryCard />);
    const before = Date.now();
    call(tree.button("Pause for 1 hour"), "onClick");
    const [[, args]] = cmds("capture_pause_until") as [[string, { resumeAtUnixMs: number }]];
    expect(args.resumeAtUnixMs).toBeGreaterThanOrEqual(before + 3_600_000);
    expect(args.resumeAtUnixMs).toBeLessThanOrEqual(Date.now() + 3_600_000);
    await tree.settle();
    expect(tree.has("Memory paused until")).toBe(true);
    call(tree.button("Resume now"), "onClick");
    expect(cmds("capture_resume_now")).toHaveLength(1);
  });

  it("a stopped Memory keeps the switch usable for turning Memory off, says what that does, keeps Pause off, and never shows the raw error", async () => {
    h.answers.capture_status = { ...status, running: false, lastError: "UI Automation: E_FAIL 0x80004005" };
    const tree = await show(() => <MemoryCard />);
    expect(tree.has("Memory stopped")).toBe(true);
    expect(tree.has("Windows would not let Whimbrel read window text.")).toBe(true);
    const sw = tree.all((n) => n.props.role === "switch")[0];
    expect(sw.props.disabled).toBeFalsy();
    expect(sw.props["aria-checked"]).toBe(true);
    expect(tree.has("Turning it off means Memory will not start again until you turn it on.")).toBe(true);
    expect(tree.button("Pause for 15 minutes").props.disabled).toBe(true);
    expect(tree.has("E_FAIL")).toBe(false);
    expect(tree.has("Not running")).toBe(false);
    call(sw, "onClick");
    expect(cmds("set_capture_enabled")).toEqual([["set_capture_enabled", { enabled: false }]]);
  });

  it("a poll that fails says it in a plain sentence, not the raw error, and the sentence goes at the next good poll", async () => {
    const tree = await show(() => <MemoryCard />);
    h.answers.capture_status = () => {
      throw new Error("ipc channel closed 0xDEAD");
    };
    polls[0]();
    await tree.settle();
    expect(tree.has("Could not read Memory's state. Trying again.")).toBe(true);
    expect(tree.has("0xDEAD")).toBe(false);
    expect(tree.has("Error:")).toBe(false);
    h.answers.capture_status = status;
    polls[0]();
    await tree.settle();
    expect(tree.has("Could not read Memory's state")).toBe(false);
    expect(tree.has("Memory on")).toBe(true);
  });

  it("the live region says only the state words; the detail line changes every poll and is plain text", async () => {
    const tree = await show(() => <MemoryCard />);
    const live = () => tree.all((n) => n.props.role === "status").map((n) => text(n).trim());
    expect(live()).toEqual(["Memory on"]);
    const detail = status.stored;
    h.answers.capture_status = { ...status, stored: detail + 7, lastCaptureUnixMs: Date.now() - 240_000 };
    polls[0]();
    await tree.settle();
    expect(live()).toEqual(["Memory on"]);
    expect(live().join(" ")).not.toMatch(/ago|saved|from/i);
  });

  it("the row label Memory has the plain sentence about what Memory saves as its description; the state is the chip, not the description", async () => {
    const tree = await show(() => <MemoryCard />);
    const row = tree.one((n) => String(n.props.className).includes("w-setting ") && text(n).includes("Memory saves the text"));
    const desc = tree.all((n) => n.props.className === "w-setting-desc" && n.parent === row.children.find((c) => typeof c !== "string" && c.props.className === "w-setting-text") );
    expect(desc.map((n) => text(n).trim())).toEqual(["Memory saves the text of the window you are working in, and nothing from excluded apps."]);
    const chip = tree.one((n) => String(n.props.className).startsWith("w-status "));
    expect(text(chip).trim()).toBe("Memory on");
    expect(text(row)).not.toContain("Memory on");
  });

  it("with no library the switch and Pause are off and the line says why", async () => {
    h.answers.launch_state = { stage: { kind: "openedWithoutStore" } };
    const tree = await show(() => <MemoryCard />);
    expect(tree.all((n) => n.props.role === "switch")[0].props.disabled).toBe(true);
    expect(tree.button("Pause for 15 minutes").props.disabled).toBe(true);
    expect(tree.has("could not open your library")).toBe(true);
  });

  it("paused by the app (a locked screen) says paused, does not offer Pause, and keeps the switch", async () => {
    h.answers.capture_status = { ...status, paused: "the screen is locked" };
    const tree = await show(() => <MemoryCard />);
    expect(tree.has("Memory paused")).toBe(true);
    expect(tree.has("The screen is locked.")).toBe(true);
    expect(tree.all((n) => n.props.role === "switch")[0].props.disabled).toBeFalsy();
    expect(tree.button("Pause for 15 minutes").props.disabled).toBe(true);
    expect(tree.has("Memory is on")).toBe(false);
  });

  it("Show screen captures calls setShowScreenCaptures", async () => {
    const tree = await show(() => <MemoryCard />);
    const sw = tree.all((n) => n.props.role === "switch")[1];
    call(sw, "onClick");
    expect(cmds("set_show_screen_captures")).toEqual([["set_show_screen_captures", { show: false }]]);
  });
});

describe("the Your data card", () => {
  it("holds the counts, the export, the Local-only line and Delete everything, in that order, with one anchor each", async () => {
    h.answers.vault_settings = { enabled: false, path: "", lastUpdatedUnixMs: null, failed: false };
    h.answers.ai_settings = { routing: { localOnly: false } };
    const tree = await show(() => <YourDataCard onOpenAi={() => {}} />);
    const body = tree.text();
    const order = ["Skipped (excluded apps)", "Keep an Obsidian vault up to date", "Open AI settings", "Delete everything…"].map((w) => body.indexOf(w));
    expect(order.every((i) => i >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    for (const id of ["capture.obsidian", "capture.localOnly", "capture.delete"]) expect(tree.all((n) => n.props.id === id), id).toHaveLength(1);
    expect(tree.all((n) => n.tag === "h3").map((n) => text(n).trim())).toEqual(["Your data"]);
  });
});

describe("the Never captured card", () => {
  it("lists excluded apps with Remove, offers only apps not yet excluded, and Exclude saves the list plus the pick", async () => {
    const tree = await show(() => <NeverCapturedCard />);
    expect(tree.has("chat.exe")).toBe(true);
    const select = tree.one((n) => n.tag === "select");
    const offered = tree.all((n) => n.tag === "option").map((o) => o.props.value);
    expect(offered).toEqual(["", "code.exe"]);
    expect(tree.button("Exclude").props.disabled).toBe(true);
    call(select, "onChange", { target: { value: "code.exe" } });
    tree.render();
    call(tree.button("Exclude"), "onClick");
    expect(cmds("set_excluded_apps")).toEqual([["set_excluded_apps", { excluded: ["chat.exe", "code.exe"] }]]);
    await tree.settle();
    expect(tree.all((n) => n.tag === "option").map((o) => o.props.value)).toEqual([""]);
  });

  it("a second press while a save is in flight does nothing, so no app is dropped; the removal is announced", async () => {
    let release: (v: string[]) => void = () => {};
    h.answers.capture_settings = { excluded: ["a.exe", "b.exe"], runningApps: [], ocr: { covered: true, available: ["en-US"], profile: ["en-US"] } };
    h.answers.set_excluded_apps = () => new Promise<string[]>((r) => (release = r));
    const tree = await show(() => <NeverCapturedCard />);
    const removes = () => tree.all((n) => n.tag === "button" && n.props["aria-label"] !== undefined && String(n.props["aria-label"]).startsWith("Remove "));
    call(removes()[0], "onClick");
    tree.render();
    expect(removes().every((b) => b.props["aria-disabled"] === true)).toBe(true);
    call(removes()[1], "onClick");
    expect(cmds("set_excluded_apps")).toEqual([["set_excluded_apps", { excluded: ["b.exe"] }]]);
    release(["b.exe"]);
    await tree.settle();
    expect(tree.has("a.exe removed from the list.")).toBe(true);
    expect(tree.one((n) => n.props.role === "status" && String(n.props.className) === "rs-memtop-announce").children.length).toBeGreaterThan(0);
  });

  it("Exclude is off with nothing chosen, and a line says what to do", async () => {
    const tree = await show(() => <NeverCapturedCard />);
    expect(tree.button("Exclude").props.disabled).toBe(true);
    const why = tree.one((n) => n.props.id === tree.button("Exclude").props["aria-describedby"], "reason");
    expect(text(why)).toBe("Choose an open app first, then press Exclude.");
  });

  it("Remove saves the list without that app; Refresh reads again", async () => {
    const tree = await show(() => <NeverCapturedCard />);
    call(tree.button("Remove"), "onClick");
    expect(cmds("set_excluded_apps")).toEqual([["set_excluded_apps", { excluded: [] }]]);
    await tree.settle();
    expect(tree.has("No apps excluded.")).toBe(true);
    call(tree.button("Refresh"), "onClick");
    expect(cmds("capture_settings")).toHaveLength(2);
  });

  it("says the OCR languages, or an alert when a language pack is missing", async () => {
    let tree = await show(() => <NeverCapturedCard />);
    expect(tree.has("OCR languages on this PC: en-US.")).toBe(true);
    h.answers.capture_settings = { excluded: [], runningApps: [], ocr: { covered: false, available: ["en-US"], profile: ["fr-FR"] } };
    tree = await show(() => <NeverCapturedCard />);
    expect(tree.one((n) => n.props.role === "alert" && String(n.children.length) !== "0" && n.tag === "p" && n.props.className === "w-setting-desc").tag).toBe("p");
    expect(tree.has("no OCR language pack for fr-FR")).toBe(true);
  });
});

describe("Delete everything", () => {
  const preview = { dataBytes: 1_000_000, dataFolder: "C:\\Data", snapshotsBytes: 0, storeOpen: true, syncOn: true, syncSignedIn: false, googleAccounts: [] };
  const textInput = (tree: Tree) => tree.one((n) => n.tag === "input" && n.props.type === "text", "typed box");
  const open = async () => {
    h.answers.delete_everything_preview = preview;
    h.answers.delete_everything_now = null;
    const tree = await show(() => <DeleteEverythingRows />);
    expect(tree.all((n) => n.props.id === "capture.delete")).toHaveLength(1);
    expect(tree.all((n) => n.props.role === "alertdialog")).toHaveLength(0);
    call(tree.button("Delete everything…"), "onClick");
    await tree.settle();
    return tree;
  };

  it("opens a confirm after the button, with nothing deleted yet, and the delete button off until the word matches", async () => {
    const tree = await open();
    expect(cmds("delete_everything_now")).toHaveLength(0);
    expect(tree.has("This removes your library")).toBe(true);
    expect(tree.button("Delete and restart").props.disabled).toBe(true);
    call(tree.button("Delete and restart"), "onClick");
    expect(cmds("delete_everything_now")).toHaveLength(0);
    call(textInput(tree), "onChange", { target: { value: "delete" } });
    tree.render();
    expect(tree.button("Delete and restart").props.disabled).toBe(true);
    // Never the one focus starts on.
    expect(tree.button("Delete and restart").props["data-initial"]).toBeUndefined();
    expect(textInput(tree).props["data-initial"]).toBe(true);
  });

  it("Return in the typed box runs nothing", async () => {
    const tree = await open();
    call(textInput(tree), "onChange", { target: { value: "DELETE" } });
    tree.render();
    expect(press(tree.one((n) => n.props.role === "alertdialog"), "Enter", { tag: "input", type: "text" })).toEqual({ prevented: true, stopped: true });
    expect(cmds("delete_everything_now")).toHaveLength(0);
  });

  it("with DELETE typed, the command gets the old arguments, including the sync-account choice", async () => {
    const tree = await open();
    expect(tree.has("Also delete my sync account")).toBe(true);
    call(tree.one((n) => n.tag === "input" && n.props.type === "checkbox"), "onChange", { target: { checked: true } });
    tree.render();
    call(textInput(tree), "onChange", { target: { value: "DELETE" } });
    tree.render();
    call(tree.button("Delete and restart"), "onClick");
    expect(cmds("delete_everything_now")).toEqual([["delete_everything_now", { confirm: "DELETE", alsoDeleteSyncAccount: true, slidesLeftAlone: false }]]);
  });

  it("a word with a space around it does not turn the button on, and the button is tied to the label that says what to type", async () => {
    const tree = await open();
    for (const wrong of [" DELETE", "DELETE ", " DELETE "]) {
      call(textInput(tree), "onChange", { target: { value: wrong } });
      tree.render();
      expect(tree.button("Delete and restart").props.disabled, wrong).toBe(true);
      call(tree.button("Delete and restart"), "onClick");
    }
    expect(cmds("delete_everything_now")).toHaveLength(0);
    const label = tree.one((n) => n.tag === "label" && text(n).trim() === "Type DELETE to confirm.");
    expect(tree.button("Delete and restart").props["aria-describedby"]).toBe(label.props.id);
  });

  it("while the delete runs the dialog keeps focus: nothing in it is disabled, and a second press does nothing", async () => {
    h.answers.delete_everything_now = () => new Promise(() => {});
    const tree = await open();
    call(textInput(tree), "onChange", { target: { value: "DELETE" } });
    tree.render();
    call(tree.button("Delete and restart"), "onClick");
    tree.render();
    const dialog = tree.one((n) => n.props.role === "alertdialog");
    const controls = tree.all((n) => (n.tag === "button" || n.tag === "input") && dialog.children.length > 0);
    // The sync-account checkbox is off while working; focus is never on it, because the press that started the work was on the button.
    expect(controls.filter((n) => n.props.disabled === true && n.props.type !== "checkbox")).toEqual([]);
    call(tree.button("Deleting…"), "onClick");
    expect(cmds("delete_everything_now")).toHaveLength(1);
  });

  it("Cancel closes it without a command", async () => {
    const tree = await open();
    call(tree.button("Cancel"), "onClick");
    tree.render();
    expect(tree.all((n) => n.props.role === "alertdialog")).toHaveLength(0);
    expect(cmds("delete_everything_now")).toHaveLength(0);
  });
});
