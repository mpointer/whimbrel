import { describe, expect, it, vi } from "vitest";
import type { SaveFileResult, SetupBegun } from "../generated/types";
import { MINIMUM_LENGTH, passphraseLength, SetupModel, type SetupActions } from "./setupModel";

const WORDS = Array.from({ length: 24 }, (_, i) => `word${i + 1}`);
const GOOD = "a long passphrase";

function actions(over: Partial<SetupActions> = {}): SetupActions {
  return {
    begin: vi.fn(async (): Promise<SetupBegun> => ({ kind: "made", words: WORDS })),
    saveFile: vi.fn(async (): Promise<SaveFileResult> => ({ kind: "saved", path: "E:\\Whimbrel recovery key.txt", sameDiskAsDatabase: false })),
    verify: vi.fn(async (p: string) => p === GOOD),
    startOver: vi.fn(async () => true),
    print: vi.fn(),
    reusesKey: false,
    ...over,
  };
}

async function made(over: Partial<SetupActions> = {}) {
  const m = new SetupModel(actions(over));
  m.setPassphrase(GOOD);
  m.setConfirmation(GOOD);
  await m.create();
  return m;
}

async function tested(m: SetupModel) {
  m.setTestPassphrase(GOOD);
  await m.test();
}

describe("the passphrase", () => {
  it("counts characters after NFC, as core does", () => {
    // "e" + combining acute is one character after NFC; so is a surrogate pair.
    expect(passphraseLength("e\u0301")).toBe(1);
    expect(passphraseLength("\u00e9")).toBe(1);
    expect(passphraseLength("😀")).toBe(1);
    expect(passphraseLength("e\u0301".repeat(11) + "x")).toBe(12);
  });

  it("needs 12 characters after NFC before Create is offered", () => {
    const m = new SetupModel(actions());
    const twelve = "e\u0301".repeat(11) + "e"; // 23 UTF-16 units, 12 characters after NFC
    m.setPassphrase("e\u0301".repeat(11));
    m.setConfirmation("e\u0301".repeat(11));
    expect(m.count).toBe(11);
    expect(m.canCreate).toBe(false);
    expect(m.problem).toBe(`Use at least ${MINIMUM_LENGTH} characters.`);
    m.setPassphrase(twelve);
    m.setConfirmation(twelve);
    expect(m.count).toBe(12);
    expect(m.canCreate).toBe(true);
    expect(m.problem).toBeNull();
  });

  it("a mismatch blocks Create and says so; nothing while empty", () => {
    const m = new SetupModel(actions());
    expect(m.problem).toBeNull();
    m.setPassphrase(GOOD);
    m.setConfirmation(GOOD + "x");
    expect(m.canCreate).toBe(false);
    expect(m.problem).toBe("The two passphrases do not match.");
  });

  it("core's PassphraseTooShort shows the minimum", async () => {
    const m = await made({ begin: async () => ({ kind: "passphraseTooShort" }) });
    expect(m.getState().words).toBeNull();
    expect(m.getState().createError).toBe("Use at least 12 characters.");
  });

  it("any other refusal is createFailed", async () => {
    const m = await made({ begin: async () => ({ kind: "failed" }) });
    expect(m.getState().createError).toBe("The recovery key could not be made. Try again.");
  });
});

describe("Create", () => {
  it("gives the words and locks the passphrase", async () => {
    const m = await made();
    expect(m.getState().words).toEqual(WORDS);
    expect(m.locked).toBe(true);
    m.setPassphrase("something else entirely");
    m.setConfirmation("something else entirely");
    expect(m.getState().passphrase).toBe(GOOD);
    expect(m.getState().confirmation).toBe(GOOD);
    expect(m.problem).toBeNull();
    expect(m.canCreate).toBe(false);
  });

  it("drops a late answer for an older passphrase", async () => {
    let finish!: (b: SetupBegun) => void;
    const m = new SetupModel(actions({ begin: () => new Promise((r) => (finish = r)) }));
    m.setPassphrase(GOOD);
    m.setConfirmation(GOOD);
    const running = m.create();
    m.setPassphrase(GOOD + " changed");
    finish({ kind: "made", words: WORDS });
    await running;
    expect(m.getState().words).toBeNull();
    expect(m.getState().creating).toBe(false);
  });
});

describe("Start over", () => {
  it("asks first, then discards everything and tells the shell", async () => {
    const a = actions();
    const m = await made(a);
    await m.saveFile();
    m.setWroteDown(true);
    await tested(m);
    m.requestStartOver();
    expect(m.getState().startOverPending).toBe(true);
    expect(a.startOver).not.toHaveBeenCalled();
    await m.confirmStartOver();
    expect(a.startOver).toHaveBeenCalledOnce();
    const s = m.getState();
    expect(s.words).toBeNull();
    expect([s.passphrase, s.confirmation, s.testPassphrase, s.savedPath]).toEqual(["", "", "", null]);
    expect([s.wroteDown, s.testPassed, s.savedConfirmed, s.startOverPending]).toEqual([false, false, false, false]);
  });

  it("Keep this one changes nothing", async () => {
    const a = actions();
    const m = await made(a);
    m.requestStartOver();
    m.cancelStartOver();
    expect(a.startOver).not.toHaveBeenCalled();
    expect(m.getState().words).toEqual(WORDS);
  });

  it("says what it does for a fresh key and for a reused one", async () => {
    expect((await made()).startOverTitle).toBe("Start over with a new recovery key?");
    expect((await made()).startOverBody).toBe("Any recovery file or printout you already made will not work with the new one.");
    const reused = await made({ reusesKey: true });
    expect(reused.startOverTitle).toBe("Change the passphrase for your recovery file?");
    expect(reused.startOverBody).toContain("Your 24 words stay the same.");
  });
});

describe("kept somewhere, and Continue", () => {
  it("a saved file is a route; with the test it allows the confirmation", async () => {
    const m = await made();
    await m.saveFile();
    expect(m.keptSomewhere).toBe(true);
    expect(m.canConfirmSaved).toBe(false);
    await tested(m);
    expect(m.canConfirmSaved).toBe(true);
  });

  it("written down is a route", async () => {
    const m = await made();
    m.setWroteDown(true);
    await tested(m);
    expect(m.canConfirmSaved).toBe(true);
  });

  it("printing is not a route on Windows (WebView2 cannot tell paper from Print to PDF)", async () => {
    const a = actions();
    const m = await made(a);
    m.print();
    expect(a.print).toHaveBeenCalledOnce();
    await tested(m);
    expect(m.keptSomewhere).toBe(false);
    expect(m.canConfirmSaved).toBe(false);
  });

  it("each route alone, without a passed test, is not enough", async () => {
    const m = await made();
    await m.saveFile();
    m.setWroteDown(true);
    m.setSavedConfirmed(true);
    expect(m.getState().savedConfirmed).toBe(false);
    m.setTestPassphrase("wrong passphrase!");
    await m.test();
    expect(m.getState().testFailed).toBe(true);
    m.setSavedConfirmed(true);
    expect(m.getState().savedConfirmed).toBe(false);
  });

  it("Continue needs the test, a route and the box", async () => {
    const m = await made();
    m.setWroteDown(true);
    expect(m.canContinue).toBe(false);
    await tested(m);
    expect(m.canContinue).toBe(false);
    m.setSavedConfirmed(true);
    expect(m.canContinue).toBe(true);
  });

  it("losing the last route unticks the box", async () => {
    const m = await made();
    m.setWroteDown(true);
    await tested(m);
    m.setSavedConfirmed(true);
    m.setWroteDown(false);
    expect(m.getState().savedConfirmed).toBe(false);
    expect(m.canContinue).toBe(false);
  });

  it("editing the test passphrase clears the result and the box", async () => {
    const m = await made();
    m.setWroteDown(true);
    await tested(m);
    m.setSavedConfirmed(true);
    m.setTestPassphrase(GOOD + "!");
    expect(m.getState().testPassed).toBe(false);
    expect(m.getState().savedConfirmed).toBe(false);
  });

  it("ignores a test result for text the user has since changed", async () => {
    let finish!: (ok: boolean) => void;
    const m = await made({ verify: () => new Promise((r) => (finish = r)) });
    m.setTestPassphrase(GOOD);
    const running = m.test();
    m.setTestPassphrase(GOOD + "x");
    finish(true);
    await running;
    expect(m.getState().testPassed).toBe(false);
  });

  it("a failed save says so; cancel says nothing; same disk warns but counts", async () => {
    const failed = await made({ saveFile: async () => ({ kind: "failed" }) });
    await failed.saveFile();
    expect(failed.getState().saveFailed).toBe(true);
    expect(failed.keptSomewhere).toBe(false);
    const cancelled = await made({ saveFile: async () => ({ kind: "cancelled" }) });
    await cancelled.saveFile();
    expect([cancelled.getState().saveFailed, cancelled.keptSomewhere]).toEqual([false, false]);
    const same = await made({ saveFile: async () => ({ kind: "saved", path: "C:\\k.txt", sameDiskAsDatabase: true }) });
    await same.saveFile();
    expect(same.getState().savedOnSameDisk).toBe(true);
    expect(same.keptSomewhere).toBe(true);
  });
});

describe("clearing, and the quit guard", () => {
  it("clear forgets the words and every input (unmount, Continue)", async () => {
    const m = await made();
    m.setTestPassphrase("x");
    m.clear();
    const s = m.getState();
    expect(s.words).toBeNull();
    expect([s.passphrase, s.confirmation, s.testPassphrase]).toEqual(["", "", ""]);
  });

  it("asks before quitting only with a fresh key whose words exist", async () => {
    expect(new SetupModel(actions()).quitNeedsConfirm).toBe(false);
    expect((await made()).quitNeedsConfirm).toBe(true);
    expect((await made({ reusesKey: true })).quitNeedsConfirm).toBe(false);
  });

  it("Quit asks the shell after a reload: it still holds words this screen lost", async () => {
    const holds = vi.fn(async () => true);
    const m = new SetupModel(actions({ begin: vi.fn(async (): Promise<SetupBegun> => ({ kind: "alreadyMade" })), hasFreshMaterial: holds }));
    m.setPassphrase(GOOD);
    m.setConfirmation(GOOD);
    await m.create();
    expect(m.quitNeedsConfirm).toBe(false);
    expect(await m.quitAsksFirst()).toBe(true);
    expect(holds).toHaveBeenCalledOnce();
    expect(await new SetupModel(actions({ hasFreshMaterial: vi.fn(async () => false) })).quitAsksFirst()).toBe(false);
    // A shell that cannot answer: ask rather than lose the words.
    expect(await new SetupModel(actions({ hasFreshMaterial: vi.fn(async () => Promise.reject(new Error("x"))) })).quitAsksFirst()).toBe(true);
    // Words on screen decide without asking the shell.
    const shown = vi.fn(async () => false);
    expect(await (await made({ hasFreshMaterial: shown })).quitAsksFirst()).toBe(true);
    expect(shown).not.toHaveBeenCalled();
  });
});

describe("a window reloaded mid-setup", () => {
  it("offers Start over (not a dead end), names the saved file it voids, then makes the words again", async () => {
    let first = true;
    const a = actions({
      begin: vi.fn(async (): Promise<SetupBegun> => {
        if (first) {
          first = false;
          return { kind: "alreadyMade" };
        }
        return { kind: "made", words: WORDS };
      }),
      savedTo: vi.fn(async () => "E:\\Whimbrel recovery key.txt"),
    });
    const m = new SetupModel(a);
    m.setPassphrase(GOOD);
    m.setConfirmation(GOOD);
    await m.create();
    expect(m.getState().createError).toBeNull();
    expect(m.getState().alreadyMade).toEqual({ savedPath: "E:\\Whimbrel recovery key.txt" });
    expect(m.canCreate).toBe(false);
    expect(m.alreadyMadeLines.join(" ")).toContain("already made before this window reloaded");
    expect(m.alreadyMadeLines.join(" ")).toContain("will not work with the new key");
    // Start over voids that file, so it asks first, as the other Start over does.
    m.requestStartOver();
    expect(m.getState().startOverPending).toBe(true);
    expect(a.startOver).not.toHaveBeenCalled();
    m.cancelStartOver();
    expect(m.getState().alreadyMade).not.toBeNull();
    expect(a.startOver).not.toHaveBeenCalled();
    m.requestStartOver();
    await m.confirmStartOver();
    expect(m.getState().startOverPending).toBe(false);
    expect(a.startOver).toHaveBeenCalledTimes(1);
    expect(m.getState().alreadyMade).toBeNull();
    expect(m.getState().words).toEqual(WORDS);
  });

  it("with a reused key the saved file still opens, so it is not called void", async () => {
    const m = new SetupModel(actions({ reusesKey: true, begin: vi.fn(async (): Promise<SetupBegun> => ({ kind: "alreadyMade" })), savedTo: vi.fn(async () => "E:\\x.txt") }));
    m.setPassphrase(GOOD);
    m.setConfirmation(GOOD);
    await m.create();
    expect(m.alreadyMadeLines).toHaveLength(1);
  });
});
