import { describe, expect, it, vi, beforeEach } from "vitest";
import type { LaunchAnswer, LaunchScreenView, LaunchStageView } from "../../generated/types";
import { Tree, press, text, type TNode, type Tree as TreeT } from "../../testkit/tree";
import { LaunchGate } from "../LaunchGate";
import { MigratingScreen as OldMigrating } from "../LaunchScreens";
import { SetupModel } from "../setupModel";
import { StartingScreen as StartingScreenR } from "./screens";
import { SetupScreen } from "./SetupScreen";

// The launch's redesigned stages (flag on) against the old ones (flag off). The shell is faked at its one door,
// `invoke`, so what is compared is the Tauri commands and their arguments, which is what must not change.
vi.mock("react", async (actual) => (await import("../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));

const h = vi.hoisted(() => ({
  flag: false,
  view: { stage: { kind: "starting" } } as { stage: unknown },
  generation: 1,
  calls: [] as [string, unknown][],
  answers: {} as Record<string, unknown>,
}));
vi.mock("../../redesign", () => ({ useRedesign: () => h.flag }));
vi.mock("../../App", () => ({ App: () => null }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: unknown) => {
    h.calls.push([cmd, args ?? null]);
    if (cmd === "launch_state") return { busy: false, waitingForUser: true, generation: h.generation, stage: h.view.stage };
    if (cmd in h.answers) return h.answers[cmd];
    return null;
  },
}));

const WORDS = Array.from({ length: 24 }, (_, i) => `word${i + 1}`);
const GOOD = "a long passphrase";

beforeEach(() => {
  vi.stubGlobal("document", { activeElement: null, body: { nodeType: 1 } });
  h.flag = false;
  h.calls = [];
  h.generation += 1;
  h.answers = {
    launch_respond: true,
    launch_fresh_library_notice: null,
    setup_begin: { kind: "made", words: WORDS },
    setup_save_file: { kind: "saved", path: "E:\\Whimbrel recovery key.txt", sameDiskAsDatabase: false },
    setup_verify: true,
    setup_continue: true,
    setup_has_fresh_material: true,
    setup_saved_to: null,
    launch_recovery_choose_file: { kind: "chosen", name: "Whimbrel recovery key.txt" },
    launch_recovery_unlock_file: "keyRejected",
    launch_recovery_unlock_words: "badWords",
  };
});

async function gate(stage: LaunchStageView, flag: boolean): Promise<TreeT> {
  h.flag = flag;
  h.view = { stage };
  const tree = new Tree(() => <LaunchGate />).render();
  await tree.settle();
  return tree;
}
const commands = () => h.calls.filter(([c]) => c !== "launch_state" && c !== "launch_fresh_library_notice");

/** A tree as the page would hold it, without the functions: for "the same DOM". */
function shape(tree: TreeT): string {
  const strip = (n: TNode | string): unknown =>
    typeof n === "string" ? n : { tag: n.tag, props: Object.fromEntries(Object.entries(n.props).filter(([k]) => k !== "children").map(([k, v]) => [k, typeof v === "function" ? "[fn]" : v])), children: n.children.map(strip) };
  return JSON.stringify(tree.root.map(strip));
}

/** A field by its visible label, as either look draws it: tied with `for`, wrapped in the label, or named by aria-label. `nth` picks among several. */
function named(tree: TreeT, label: string, nth = 0): TNode {
  const found: TNode[] = [];
  for (const lab of tree.all((n) => n.tag === "label" && text(n).trim() === label)) {
    if (lab.props.htmlFor !== undefined) found.push(...tree.all((n) => (n.tag === "input" || n.tag === "textarea") && n.props.id === lab.props.htmlFor));
    else found.push(...lab.children.filter((c): c is TNode => typeof c !== "string" && (c.tag === "input" || c.tag === "textarea")));
  }
  found.push(...tree.all((n) => (n.tag === "input" || n.tag === "textarea") && n.props["aria-label"] === label));
  return found[nth];
}
function box(tree: TreeT, label: string): TNode {
  const lab = tree.one((n) => n.tag === "label" && text(n).trim() === label, `checkbox "${label}"`);
  return lab.children.find((c): c is TNode => typeof c !== "string" && c.tag === "input")!;
}
const click = (b: TNode) => {
  if (b.props.onClick) return (b.props.onClick as () => void)();
  // The old screens put some buttons in a form; pressing one submits it.
  let form = b.parent;
  while (form && form.tag !== "form") form = form.parent;
  if (b.props.type !== "submit" || !form) throw new Error(`nothing to press on ${text(b)}`);
  (form.props.onSubmit as (e: unknown) => void)({ preventDefault() {} });
};
const tick = (c: TNode, on = true) => (c.props.onChange as (e: unknown) => void)({ target: { checked: on } });
const type = (f: TNode, value: string) => (f.props.onChange as (e: unknown) => void)({ target: { value } });
const enter = (tree: TreeT, tag = "div", inputType?: string) => press(tree.one((n) => n.tag === "main"), "Enter", { tag, type: inputType });
const escape = (tree: TreeT) => press(tree.one((n) => n.tag === "main"), "Escape", { tag: "div" });

describe("the switch", () => {
  const SCREENS: { name: string; stage: LaunchStageView }[] = [
    { name: "damaged", stage: { kind: "screen", lastAction: null, offers: ["retry", "quit"], screen: { kind: "damaged", reason: "noAccess" } } },
    { name: "key storage", stage: { kind: "screen", lastAction: null, offers: ["retry", "quit"], screen: { kind: "keyStorageUnavailable" } } },
    { name: "migrating", stage: { kind: "migrating", overall: 0.42 } },
  ];

  it.each(SCREENS)("flag off: $name is the old screen, in the old wrapper, with no new class", async ({ stage }) => {
    const tree = await gate(stage, false);
    expect(tree.all((n) => String(n.props.className ?? "").split(" ").includes("launch"))).toHaveLength(1);
    expect(tree.all((n) => /\bw-(flow|sys)\b/.test(String(n.props.className ?? "")))).toHaveLength(0);
  });

  it("flag off: the migration is exactly what the old screen draws, wrapped as before", async () => {
    const tree = await gate({ kind: "migrating", overall: 0.42 }, false);
    const expected = new Tree(() => (
      <div className="launch">
        <OldMigrating overall={0.42} />
      </div>
    )).render();
    expect(shape(tree)).toBe(shape(expected));
  });

  it("flag on: every stage draws on the new page, never the old launch classes", async () => {
    const stages: LaunchStageView[] = [
      { kind: "onboarding", reusesKey: false },
      { kind: "migrating", overall: 0.3 },
      { kind: "recovery", reason: "needsKey" },
      { kind: "recovery", reason: "keyRejected" },
      { kind: "keyUnavailable", stillUnavailable: false },
      ...SCREENS.filter((s) => s.stage.kind === "screen").map((s) => s.stage),
    ];
    for (const stage of stages) {
      const tree = await gate(stage, true);
      expect(tree.all((n) => n.tag === "main" && n.props.className === "w-flow"), stage.kind).toHaveLength(1);
      expect(tree.all((n) => /\blaunch(-screen|-starting)?\b|\bsetup\b/.test(String(n.props.className ?? ""))), stage.kind).toHaveLength(0);
    }
  });

  it("the start waits a second before it shows anything, in either look, and has no buttons", async () => {
    for (const flag of [false, true]) {
      const tree = await gate({ kind: "starting" }, flag);
      expect(tree.all((n) => n.tag === "button")).toHaveLength(0);
      expect(tree.has("Starting Whimbrel…")).toBe(false);
    }
  });

  it("quitting draws the start screen in either look, with no buttons", async () => {
    const now = await gate({ kind: "quitting" }, true);
    expect(now.all((n) => String(n.props.className ?? "").includes("w-flow-root"))).toHaveLength(1);
    expect(now.all((n) => n.tag === "button")).toHaveLength(0);
    const old = await gate({ kind: "quitting" }, false);
    expect(old.all((n) => n.props.className === "launch-starting")).toHaveLength(1);
  });

  it("the start with no delay shows its words on the new look's wait page", () => {
    const tree = new Tree(() => <StartingScreenR delayMs={0} />).render();
    expect(tree.has("Starting Whimbrel…")).toBe(true);
    expect(tree.one((n) => n.props.role === "status").props["aria-busy"]).toBe("true");
    expect(tree.all((n) => n.tag === "button")).toHaveLength(0);
  });
});

describe("the migration", () => {
  it("shows a progress bar that holds its value, a page title and no button, as the old one", async () => {
    const tree = await gate({ kind: "migrating", overall: 0.42 }, true);
    const bar = tree.one((n) => n.props.role === "progressbar");
    expect(bar.props["aria-valuenow"]).toBe(42);
    expect(tree.all((n) => n.tag === "button")).toHaveLength(0);
    expect(tree.one((n) => n.tag === "h1").children.join("")).toBe("Encrypting the memory database");
    expect(tree.has("Keep Whimbrel open and your PC awake until it finishes.")).toBe(true);
    expect(tree.has("If it is interrupted, Whimbrel finishes or undoes it the next time it starts.")).toBe(true);
    expect(tree.has("Step ")).toBe(false);
  });
});

describe("the key-unavailable screen", () => {
  const stage = (stillUnavailable: boolean): LaunchStageView => ({ kind: "keyUnavailable", stillUnavailable });

  it("has Try again and Quit and sends the same answers as the old screen", async () => {
    for (const [label, answer] of [["Try again", "retry"], ["Quit Whimbrel", "quit"]] as const) {
      const per: unknown[][] = [];
      for (const flag of [false, true]) {
        h.calls = [];
        const tree = await gate(stage(false), flag);
        click(tree.button(label));
        per.push(commands());
      }
      expect(per[1]).toEqual(per[0]);
      expect(per[1]).toEqual([["launch_respond", { answer }]]);
    }
  });

  it("says the key still can't be read, as an alert, only on the second identical stage", async () => {
    expect((await gate(stage(false), true)).has("still can't be read")).toBe(false);
    const again = await gate(stage(true), true);
    expect(again.one((n) => n.props.role === "alert").children.map(text).join("")).toContain("The key still can't be read.");
  });

  it("Return is Try again", async () => {
    const tree = await gate(stage(false), true);
    expect(enter(tree).prevented).toBe(true);
    expect(commands()).toEqual([["launch_respond", { answer: "retry" }]]);
  });

  it("keeps its fine print and the sentence that nothing was changed", async () => {
    const tree = await gate(stage(false), true);
    expect(tree.has("Whimbrel has not opened, changed or deleted anything.")).toBe(true);
    expect(tree.has("you do not need your recovery key for it.")).toBe(true);
  });
});

describe("the blocking screens", () => {
  const CASES: { name: string; screen: LaunchScreenView; offers: LaunchAnswer[] }[] = [
    { name: "key storage", screen: { kind: "keyStorageUnavailable" }, offers: ["retry", "quit"] },
    { name: "interrupted", screen: { kind: "migrationInterrupted" }, offers: ["retry", "quit"] },
    { name: "damaged", screen: { kind: "damaged", reason: "damaged" }, offers: ["quit"] },
    { name: "open elsewhere", screen: { kind: "damaged", reason: "openElsewhere" }, offers: ["retry", "quit"] },
    { name: "missing, backup", screen: { kind: "encryptedFileMissing", backupPresent: true, encryptedCopyPresent: false }, offers: ["quit", "restoreRollbackCopy"] },
    { name: "missing, nothing", screen: { kind: "encryptedFileMissing", backupPresent: false, encryptedCopyPresent: false }, offers: ["quit", "resetEncryptionStatus"] },
    { name: "plain open failed", screen: { kind: "error", error: { kind: "plainOpenFailed" } }, offers: ["retry", "quit", "continueWithoutStore"] },
  ];
  const stageOf = (c: (typeof CASES)[number]): LaunchStageView => ({ kind: "screen", lastAction: null, offers: c.offers, screen: c.screen });
  const labels = (tree: TreeT) => tree.all((n) => n.tag === "button" && n.props["data-answer"] !== undefined).map((n) => `${n.props["data-answer"]}:${text(n)}`).sort();

  it.each(CASES)("$name: the same answers and the same words on the buttons as the old screen", async (c) => {
    const old = await gate(stageOf(c), false);
    const now = await gate(stageOf(c), true);
    expect(labels(now)).toEqual(labels(old));
    expect(labels(now).some((l) => l.startsWith("quit:"))).toBe(true);
    // The title and every paragraph are the old screen's words.
    expect(now.one((n) => n.tag === "h1").children.join("")).toBe(old.one((n) => n.tag === "h1").children.join(""));
    for (const p of old.all((n) => n.tag === "p")) expect(now.has(text(p)), text(p)).toBe(true);
  });

  it.each(CASES)("$name: each answer that needs no question sends the same command as the old one", async (c) => {
    for (const answer of c.offers) {
      const sent: unknown[][] = [];
      for (const flag of [false, true]) {
        h.calls = [];
        const tree = await gate(stageOf(c), flag);
        const b = tree.one((n) => n.tag === "button" && n.props["data-answer"] === answer);
        click(b);
        sent.push(commands());
        if (answer === "restoreRollbackCopy" || answer === "resetEncryptionStatus") {
          // It asks first: nothing is sent until the question is answered.
          expect(commands(), `${c.name} ${answer} flag=${flag}`).toEqual([]);
          tree.render();
          const dialog = tree.one((n) => n.props.role === "alertdialog");
          expect(dialog).toBeTruthy();
          click(tree.all((n) => n.tag === "button" && text(n).trim() === (answer === "restoreRollbackCopy" ? "Put it back" : "Reset and start empty"))[0]);
          sent[sent.length - 1] = commands();
        }
      }
      expect(sent[1], `${c.name} ${answer}`).toEqual(sent[0]);
      expect(sent[1]).toEqual([["launch_respond", { answer }]]);
    }
  });

  it("draws the one primary last, and one primary only", async () => {
    for (const c of CASES) {
      const tree = await gate(stageOf(c), true);
      const row = tree.one((n) => n.props.className === "w-flow-actions");
      const buttons = row.children.flatMap((ch) => (typeof ch === "string" ? [] : ch.tag === "button" ? [ch] : [])).filter((b) => b.props["data-answer"] !== undefined);
      const primaries = buttons.filter((b) => String(b.props.className).includes("w-button-primary"));
      expect(primaries, c.name).toHaveLength(1);
      expect(buttons[buttons.length - 1], c.name).toBe(primaries[0]);
    }
  });

  it("never shows more than three controls in a row: past three, the first go under More options", async () => {
    const offers: LaunchAnswer[] = ["retry", "restoreRollbackCopy", "resetEncryptionStatus", "continueWithoutStore", "quit"];
    const tree = await gate({ kind: "screen", lastAction: null, offers, screen: { kind: "error", error: { kind: "plainOpenFailed" } } }, true);
    const row = tree.one((n) => n.props.className === "w-flow-actions");
    const controls = row.children.filter((c): c is TNode => typeof c !== "string");
    expect(controls.length).toBeLessThanOrEqual(3);
    expect(tree.hasButton("More options")).toBe(true);
    click(tree.button("More options"));
    tree.render();
    const items = tree.all((n) => n.props.role === "menuitem").map(text);
    const shown = tree.all((n) => n.tag === "button" && n.props["data-answer"] !== undefined).map((n) => n.props["data-answer"]);
    expect([...items.map(() => "x"), ...shown]).toHaveLength(offers.length + 0);
    expect(shown).toEqual(expect.arrayContaining(["quit", "retry"]));
  });

  it("Return is the primary action, and on a question it only opens the question", async () => {
    const quitOnly = await gate(stageOf(CASES[2]), true);
    enter(quitOnly);
    expect(commands()).toEqual([["launch_respond", { answer: "quit" }]]);

    h.calls = [];
    const restore = await gate(stageOf(CASES[4]), true);
    enter(restore);
    expect(commands()).toEqual([]);
    restore.render();
    expect(restore.all((n) => n.props.role === "alertdialog")).toHaveLength(1);
  });

  it("Escape closes the question and sends nothing", async () => {
    const tree = await gate(stageOf(CASES[5]), true);
    click(tree.one((n) => n.props["data-answer"] === "resetEncryptionStatus"));
    tree.render();
    const dialog = tree.one((n) => n.props.role === "alertdialog");
    expect(press(dialog, "Escape").stopped).toBe(true);
    tree.render();
    expect(tree.all((n) => n.props.role === "alertdialog")).toHaveLength(0);
    expect(commands()).toEqual([]);
  });

  it("a question's confirming button is destructive only for the reset, and Cancel has the focus first on both", async () => {
    const reset = await gate(stageOf(CASES[5]), true);
    click(reset.one((n) => n.props["data-answer"] === "resetEncryptionStatus"));
    reset.render();
    expect(String(reset.button("Reset and start empty").props.className)).toContain("w-button-destructive");
    expect(reset.button("Cancel").props["data-initial"]).toBe("");
    const restore = await gate(stageOf(CASES[4]), true);
    click(restore.one((n) => n.props["data-answer"] === "restoreRollbackCopy"));
    restore.render();
    expect(String(restore.button("Put it back").props.className)).not.toContain("w-button-destructive");
    expect(restore.button("Cancel").props["data-initial"]).toBe("");
    expect(restore.button("Put it back").props["data-initial"]).toBeUndefined();
  });

  it("a screen with nothing to show between its words and its buttons draws no empty body", async () => {
    const tree = await gate(stageOf(CASES[0]), true);
    expect(tree.all((n) => n.props.className === "w-flow-body")).toHaveLength(0);
  });

  it("shows a failed last action next to the screen as an alert", async () => {
    const stage: LaunchStageView = {
      kind: "screen",
      lastAction: { action: "restore", outcome: { result: "refused" } },
      offers: ["quit", "restoreRollbackCopy"],
      screen: CASES[4].screen,
    };
    const tree = await gate(stage, true);
    expect(tree.one((n) => n.props.role === "alert").children.map(text).join("")).toBe("The copy could not be put back. Nothing was changed.");
  });

  it("buttons are off while an answer is in flight", async () => {
    const tree = await gate(stageOf(CASES[0]), true);
    click(tree.one((n) => n.props["data-answer"] === "retry"));
    tree.render();
    for (const b of tree.all((n) => n.tag === "button" && n.props["data-answer"] !== undefined)) expect(b.props.disabled).toBe(true);
    // Return does nothing while it is.
    h.calls = [];
    enter(tree);
    expect(commands()).toEqual([]);
  });
});

describe("recovery", () => {
  const stage = (reason: "needsKey" | "keyRejected"): LaunchStageView => ({ kind: "recovery", reason });

  it("says why, in the old words, for both reasons, with the old fine print", async () => {
    for (const reason of ["needsKey", "keyRejected"] as const) {
      const old = await gate(stage(reason), false);
      const now = await gate(stage(reason), true);
      for (const p of old.all((n) => n.tag === "p" && !String(n.props.className ?? "").includes("setup-error"))) expect(now.has(text(p)), text(p)).toBe(true);
      expect(now.one((n) => n.tag === "h1").children.join("")).toBe("Whimbrel can't open your memory database");
    }
  });

  const walk = async (flag: boolean) => {
    h.calls = [];
    const tree = await gate(stage("needsKey"), flag);
    click(tree.button("Choose recovery file…"));
    await tree.settle();
    type(named(tree, "Passphrase"), "my passphrase");
    tree.render();
    flag ? enter(tree, "input", "password") : click(tree.button("Unlock with file"));
    await tree.settle();
    // The other way in: the 24 words.
    click(tree.all((n) => n.props.role === "radio" && text(n) === "24 words")[0]);
    tree.render();
    type(named(tree, "24 words"), "abandon ability able");
    tree.render();
    flag ? press(tree.one((n) => n.tag === "main"), "Enter", { tag: "textarea", ctrl: true }) : click(tree.button("Unlock with words"));
    await tree.settle();
    click(tree.button("Quit Whimbrel"));
    return { calls: commands(), tree };
  };

  it("calls the same commands, with the same arguments, in the same order as the old screen", async () => {
    const old = await walk(false);
    const now = await walk(true);
    expect(now.calls).toEqual(old.calls);
    expect(now.calls.map(([c]) => c)).toEqual(["launch_recovery_choose_file", "launch_recovery_unlock_file", "launch_recovery_unlock_words", "launch_respond"]);
    expect(now.calls[1]).toEqual(["launch_recovery_unlock_file", { passphrase: "my passphrase" }]);
    expect(now.calls[2]).toEqual(["launch_recovery_unlock_words", { words: "abandon ability able" }]);
  });

  it("unlock is off until there is something to unlock with, and Return does nothing then", async () => {
    const tree = await gate(stage("needsKey"), true);
    expect(tree.button("Unlock with file").props.disabled).toBe(true);
    h.calls = [];
    expect(enter(tree, "input", "password").prevented).toBe(false);
    expect(commands()).toEqual([]);
  });

  it("a plain Return in the words box is a new line, not an unlock", async () => {
    const tree = await gate(stage("needsKey"), true);
    click(tree.all((n) => n.props.role === "radio" && text(n) === "24 words")[0]);
    tree.render();
    type(named(tree, "24 words"), "abandon");
    tree.render();
    h.calls = [];
    expect(press(tree.one((n) => n.tag === "main"), "Enter", { tag: "textarea" }).prevented).toBe(false);
    expect(commands()).toEqual([]);
  });

  it("puts the error directly under the field it belongs to, in the right method", async () => {
    const tree = await gate(stage("needsKey"), true);
    click(tree.button("Choose recovery file…"));
    await tree.settle();
    type(named(tree, "Passphrase"), "my passphrase");
    tree.render();
    enter(tree, "input", "password");
    await tree.settle();
    const field = tree.all((n) => n.props.className === "w-field").find((f) => text(f).includes("Passphrase"))!;
    expect(text(field)).toContain("That recovery key does not open this memory database.");
    // The field points at it.
    const input = tree.field("Passphrase");
    expect(String(input.props["aria-describedby"])).toContain(String(field.children.map((c) => (typeof c === "string" ? null : c.props.id)).find((id) => String(id).endsWith("-error"))));
    expect(input.props["aria-invalid"]).toBe(true);
  });

  it("every input has a visible label, and there is no step line", async () => {
    const tree = await gate(stage("needsKey"), true);
    expect(tree.has("Step ")).toBe(false);
    for (const input of tree.all((n) => n.tag === "input" || n.tag === "textarea"))
      expect(tree.all((n) => n.tag === "label" && n.props.htmlFor === input.props.id).length, String(input.props.id)).toBe(1);
  });
});

describe("the setup", () => {
  const stage: LaunchStageView = { kind: "onboarding", reusesKey: false };
  const stepLine = (tree: TreeT) => tree.all((n) => n.props.className === "w-flow-step").map(text)[0];
  const title = (tree: TreeT) => text(tree.one((n) => n.tag === "h1"));

  /** The whole setup, in the old screen and in the new one. */
  const walk = async (flag: boolean) => {
    h.calls = [];
    const tree = await gate(stage, flag);
    const pass = flag ? () => named(tree, "Passphrase") : () => named(tree, "Passphrase", 0);
    type(pass(), GOOD);
    tree.render();
    type(named(tree, "Type it again"), GOOD);
    tree.render();
    if (flag) enter(tree, "input", "password");
    else click(tree.button("Create recovery key"));
    await tree.settle();
    if (flag) expect(stepLine(tree)).toBe("Step 2 of 4");
    click(tree.button("Save recovery file…"));
    await tree.settle();
    if (flag) {
      click(tree.button("Next step"));
      tree.render();
      expect(stepLine(tree)).toBe("Step 3 of 4");
    }
    tick(box(tree, "I wrote down all 24 words, in order"));
    tree.render();
    if (flag) {
      enter(tree);
      tree.render();
      expect(stepLine(tree)).toBe("Step 4 of 4");
    }
    type(named(tree, "Passphrase", flag ? 0 : 1), GOOD);
    tree.render();
    if (flag) press(tree.one((n) => n.tag === "main"), "Enter", { tag: "input", type: "password" });
    click(tree.button("Test"));
    await tree.settle();
    tick(box(tree, "I have kept my recovery file or my 24 words somewhere other than this PC"));
    tree.render();
    return { calls: commands(), tree };
  };

  it("calls the same commands, with the same arguments, in the same order as the old screen", async () => {
    const old = await walk(false);
    const now = await walk(true);
    expect(now.calls).toEqual(old.calls);
    const names = now.calls.map(([c]) => c);
    expect(names).toContain("setup_begin");
    expect(names).toContain("setup_save_file");
    expect(names).toContain("setup_verify");
    expect(now.calls.find(([c]) => c === "setup_begin")![1]).toEqual({ scope: "launch", passphrase: GOOD });
    expect(now.calls.find(([c]) => c === "setup_verify")![1]).toEqual({ scope: "launch", passphrase: GOOD });
  });

  it("Continue, then Quit, send what the old ones sent", async () => {
    for (const flag of [false, true]) {
      const { tree } = await walk(flag);
      h.calls = [];
      if (flag) enter(tree);
      else click(tree.button("Continue"));
      await tree.settle();
      expect(commands()).toEqual([["setup_continue", { scope: "launch" }]]);
    }
  });

  it("starts on step 1 of 4 with its title, the old paragraph, the rule before typing and the count tied to the field", async () => {
    const tree = await gate(stage, true);
    expect(stepLine(tree)).toBe("Step 1 of 4");
    expect(title(tree)).toBe("Create your recovery key");
    expect(tree.has("Whimbrel will encrypt the memory database on this PC with a key kept in Windows Credential Manager.")).toBe(true);
    expect(tree.has("At least 12 characters. A short sentence you will remember works well.")).toBe(true);
    expect(tree.has("0 of at least 12 characters")).toBe(true);
    const pass = named(tree, "Passphrase");
    expect(String(pass.props["aria-describedby"]).split(" ").length).toBeGreaterThanOrEqual(3);
    expect(tree.button("Create recovery key").props.disabled).toBe(true);
    // Return does nothing yet, and is not swallowed.
    expect(enter(tree, "input", "password").prevented).toBe(false);
  });

  it("puts the problem right under the field it belongs to", async () => {
    const tree = await gate(stage, true);
    type(named(tree, "Passphrase"), "short");
    tree.render();
    const fields = tree.all((n) => n.props.className === "w-field");
    expect(text(fields[0])).toContain("Use at least 12 characters.");
    expect(text(fields[1])).not.toContain("Use at least");
    type(named(tree, "Passphrase"), GOOD);
    type(named(tree, "Type it again"), "something else entirely");
    tree.render();
    const after = tree.all((n) => n.props.className === "w-field");
    expect(text(after[1])).toContain("The two passphrases do not match.");
    expect(text(after[0])).not.toContain("do not match");
  });

  it("walks the four steps with Next step and Back, and Escape goes back", async () => {
    const { tree } = await walk(true);
    expect(stepLine(tree)).toBe("Step 4 of 4");
    expect(title(tree)).toBe("Test your passphrase");
    escape(tree);
    tree.render();
    expect(stepLine(tree)).toBe("Step 3 of 4");
    expect(title(tree)).toBe("Write down your 24 recovery words");
    click(tree.button("Back"));
    tree.render();
    expect(title(tree)).toBe("Save the recovery file");
    // The passphrase is locked once the words exist: back on step 1, it cannot be changed, and Start over is offered.
    escape(tree);
    tree.render();
    expect(stepLine(tree)).toBe("Step 1 of 4");
    expect(named(tree, "Passphrase").props.disabled).toBe(true);
    expect(tree.has("To use a different passphrase, start over.")).toBe(true);
    expect(tree.hasButton("Start over…")).toBe(true);
    // Escape on the first step has nowhere to go.
    expect(escape(tree).prevented).toBe(false);
    expect(stepLine(tree)).toBe("Step 1 of 4");
  });

  it("the words: 24 of them, numbered, in a list nobody can copy from, and no button copies them", async () => {
    const tree = await gate(stage, true);
    type(named(tree, "Passphrase"), GOOD);
    type(named(tree, "Type it again"), GOOD);
    tree.render();
    click(tree.button("Create recovery key"));
    await tree.settle();
    click(tree.button("Next step"));
    tree.render();
    const list = tree.one((n) => n.tag === "ol");
    expect(list.children).toHaveLength(24);
    expect(text(list.children[23] as TNode)).toBe("24.word24");
    expect(tree.has("These words are the key itself.")).toBe(true);
    expect(tree.all((n) => n.tag === "button" && /copy/i.test(text(n)))).toHaveLength(0);
    expect(tree.hasButton("Print words and QR code…")).toBe(true);
  });

  it("every input has a visible label on every step", async () => {
    const tree = await gate(stage, true);
    const check = (where: string) => {
      for (const input of tree.all((n) => n.tag === "input" && n.props.type !== "checkbox"))
        expect(tree.all((n) => n.tag === "label" && n.props.htmlFor === input.props.id).length, `${where} ${String(input.props.id)}`).toBe(1);
      const boxes = tree.all((n) => n.tag === "input" && n.props.type === "checkbox").length;
      expect(tree.all((n) => n.tag === "label" && n.children.some((c) => typeof c !== "string" && c.tag === "input")).length, where).toBe(boxes);
    };
    check("step 1");
    type(named(tree, "Passphrase"), GOOD);
    type(named(tree, "Type it again"), GOOD);
    tree.render();
    click(tree.button("Create recovery key"));
    await tree.settle();
    for (const where of ["step 2", "step 3", "step 4"]) {
      check(where);
      if (where !== "step 4") {
        click(tree.button("Next step"));
        tree.render();
      }
    }
  });

  it("a failed test is an error under the passphrase field; a good one says so", async () => {
    h.answers.setup_verify = false;
    const tree = await gate(stage, true);
    type(named(tree, "Passphrase"), GOOD);
    type(named(tree, "Type it again"), GOOD);
    tree.render();
    click(tree.button("Create recovery key"));
    await tree.settle();
    escape(tree);
    tree.render();
    for (let i = 0; i < 3; i++) {
      click(tree.button("Next step"));
      tree.render();
    }
    type(named(tree, "Passphrase"), "wrong");
    tree.render();
    click(tree.button("Test"));
    await tree.settle();
    const field = tree.all((n) => n.props.className === "w-field")[0];
    expect(text(field)).toContain("That passphrase does not open the recovery file. Check it and try again.");
    expect(tree.has("That passphrase opens your recovery file.")).toBe(false);
  });

  it("Continue stays off until the test passed and the key was kept somewhere, and Return does nothing then", async () => {
    const tree = await gate(stage, true);
    type(named(tree, "Passphrase"), GOOD);
    type(named(tree, "Type it again"), GOOD);
    tree.render();
    click(tree.button("Create recovery key"));
    await tree.settle();
    for (let i = 0; i < 2; i++) {
      click(tree.button("Next step"));
      tree.render();
    }
    expect(tree.button("Continue").props.disabled).toBe(true);
    expect(tree.has("First pass the test above, and save the recovery file or tick that you wrote the words down.")).toBe(true);
    h.calls = [];
    enter(tree);
    expect(commands()).toEqual([]);
  });

  it("Quit asks first once there are words, with a question Escape closes, and quits at once before", async () => {
    // Before any words: it asks the shell, which holds none, and quits.
    h.answers.setup_has_fresh_material = false;
    const first = await gate(stage, true);
    click(first.button("Quit Whimbrel"));
    await first.settle();
    expect(commands().some(([c, a]) => c === "launch_respond" && (a as { answer: string }).answer === "quit")).toBe(true);

    h.calls = [];
    h.answers.setup_has_fresh_material = true;
    const tree = await gate(stage, true);
    type(named(tree, "Passphrase"), GOOD);
    type(named(tree, "Type it again"), GOOD);
    tree.render();
    click(tree.button("Create recovery key"));
    await tree.settle();
    h.calls = [];
    click(tree.button("Quit Whimbrel"));
    await tree.settle();
    expect(commands()).toEqual([]);
    const dialog = tree.one((n) => n.props.role === "alertdialog");
    expect(text(dialog)).toContain("Your recovery file or printout will not work if you quit now. Quit anyway?");
    expect(text(dialog)).toContain("The next time Whimbrel starts, it makes a new recovery key with new words.");
    press(dialog, "Escape");
    tree.render();
    expect(tree.all((n) => n.props.role === "alertdialog")).toHaveLength(0);
    expect(commands()).toEqual([]);
  });

  it("keeps the sentence about losing both on every step, word for word", async () => {
    const lost = "nobody can unlock the memory database, including Whimbrel.";
    const tree = await gate(stage, true);
    expect(tree.has(lost)).toBe(true);
    type(named(tree, "Passphrase"), GOOD);
    type(named(tree, "Type it again"), GOOD);
    tree.render();
    click(tree.button("Create recovery key"));
    await tree.settle();
    expect(tree.has(lost)).toBe(true);
  });

  it("the page after Delete everything leads with its notice, on the first step only", () => {
    const model = new SetupModel({ begin: async () => ({ kind: "made", words: WORDS }), saveFile: vi.fn(), verify: vi.fn(), startOver: vi.fn(), print: vi.fn(), reusesKey: false });
    const line = "Your previous data was deleted. This is a new, empty library.";
    const tree = new Tree(() => <SetupScreen model={model} continueTitle="Continue" onContinue={() => {}} onQuit={() => {}} notice={line} />).render();
    const main = tree.one((n) => n.tag === "main");
    expect(text(main.children[0] as TNode)).toContain(line);
    expect(tree.all((n) => n.props.role === "status").map(text)).toContain(line);
    // Never without a notice.
    expect(new Tree(() => <SetupScreen model={model} continueTitle="Continue" onContinue={() => {}} onQuit={() => {}} />).render().has(line)).toBe(false);
    // And not on the later steps.
    const later = new Tree(() => <SetupScreen model={model} continueTitle="Continue" onContinue={() => {}} onQuit={() => {}} notice={line} initialStep={3} />).render();
    expect(later.has("Step 1 of 4")).toBe(true);
  });
});
