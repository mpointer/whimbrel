import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { LaunchAnswer, LaunchErrorView, LaunchScreenView } from "../generated/types";
import { BlockingScreen, KeyUnavailableScreen, MigratingScreen } from "./LaunchScreens";
import { errorParagraphs, isCurrentView, lastActionText, nextProgress, NOT_PRESSED, pressReducer, screenContent } from "./screenModel";

/** The answers each screen offers, as the shell's `offers` computes them (commands/launch.rs
 *  `the_stage_crosses_the_bridge_in_camel_case_with_its_offers` and `LaunchScreen::offers`). */
const CASES: { name: string; screen: LaunchScreenView; offers: LaunchAnswer[] }[] = [
  { name: "key storage", screen: { kind: "keyStorageUnavailable" }, offers: ["retry", "quit"] },
  { name: "interrupted", screen: { kind: "migrationInterrupted" }, offers: ["retry", "quit"] },
  { name: "damaged", screen: { kind: "damaged", reason: "damaged" }, offers: ["quit"] },
  { name: "no access", screen: { kind: "damaged", reason: "noAccess" }, offers: ["retry", "quit"] },
  { name: "missing, backup", screen: { kind: "encryptedFileMissing", backupPresent: true, encryptedCopyPresent: false }, offers: ["quit", "restoreRollbackCopy"] },
  { name: "missing, .enc.tmp only", screen: { kind: "encryptedFileMissing", backupPresent: false, encryptedCopyPresent: true }, offers: ["quit"] },
  { name: "missing, nothing", screen: { kind: "encryptedFileMissing", backupPresent: false, encryptedCopyPresent: false }, offers: ["quit", "resetEncryptionStatus"] },
  { name: "plain open failed", screen: { kind: "error", error: { kind: "plainOpenFailed" } }, offers: ["retry", "quit", "continueWithoutStore"] },
  { name: "keyed open failed", screen: { kind: "error", error: { kind: "keyedOpenFailed", open: "damaged" } }, offers: ["retry", "quit"] },
  { name: "open elsewhere", screen: { kind: "damaged", reason: "openElsewhere" }, offers: ["retry", "quit"] },
];

const answersIn = (html: string) => [...html.matchAll(/data-answer="([a-zA-Z]+)"/g)].map((m) => m[1]).sort();

describe("blocking screens", () => {
  it.each(CASES)("$name: the buttons are exactly its offers, Quit always among them", ({ screen, offers }) => {
    const html = renderToStaticMarkup(<BlockingScreen screen={screen} offers={offers} busy={false} onAnswer={() => {}} />);
    expect(answersIn(html)).toEqual([...offers].sort());
    expect(html).toContain("Quit Whimbrel");
  });

  it("draws no button the shell did not offer, even on a screen that could have it", () => {
    const html = renderToStaticMarkup(
      <BlockingScreen screen={{ kind: "error", error: { kind: "plainOpenFailed" } }} offers={["quit"]} busy={false} onAnswer={() => {}} />,
    );
    expect(answersIn(html)).toEqual(["quit"]);
  });

  it("the damaged and .enc.tmp-only screens have Quit only", () => {
    for (const c of CASES.filter((c) => c.name === "damaged" || c.name === "missing, .enc.tmp only")) {
      expect(screenContent(c.screen, c.offers).buttons.map((b) => b.answer)).toEqual(["quit"]);
    }
  });

  it("a file another program holds reads as in use, not damaged, and offers Retry", () => {
    const c = screenContent({ kind: "damaged", reason: "openElsewhere" }, ["retry", "quit"]);
    const shown = [c.title, ...c.paragraphs, ...c.hints].join(" ");
    expect(shown).toContain("Another program has the memory database file open");
    expect(shown).toContain("then try again");
    expect(shown).not.toMatch(/damaged/i);
    expect(c.buttons.map((b) => b.answer)).toEqual(["retry", "quit"]);
  });

  it("Continue without the memory database only after a failed plain open", () => {
    for (const c of CASES) {
      const labels = screenContent(c.screen, c.offers).buttons.map((b) => b.label);
      expect(labels.includes("Continue without the memory database")).toBe(c.name === "plain open failed");
    }
  });

  it("the word 'empty' appears only in the reset confirmation", () => {
    for (const c of CASES) {
      const content = screenContent(c.screen, c.offers);
      const shown = [content.title, ...content.paragraphs, ...content.hints, ...content.buttons.map((b) => b.label)].join(" ");
      expect(shown).not.toMatch(/empty/i);
      for (const b of content.buttons) {
        if (b.confirm) expect(/empty/i.test(`${b.confirm.title} ${b.confirm.body} ${b.confirm.confirm}`)).toBe(b.answer === "resetEncryptionStatus");
      }
    }
  });

  it("restore and reset ask first; reset's confirm is destructive", () => {
    const restore = screenContent(CASES[4].screen, CASES[4].offers).buttons.find((b) => b.answer === "restoreRollbackCopy")!;
    expect(restore.confirm).toMatchObject({ title: "Put back the unencrypted copy?", confirm: "Put it back", destructive: false });
    const reset = screenContent(CASES[6].screen, CASES[6].offers).buttons.find((b) => b.answer === "resetEncryptionStatus")!;
    expect(reset.confirm).toMatchObject({ title: "Reset encryption status and start empty?", confirm: "Reset and start empty", destructive: true });
  });

  it("the titles, bodies and hints follow the brief's table", () => {
    const c = (i: number) => screenContent(CASES[i].screen, CASES[i].offers);
    expect(c(0).title).toBe("Whimbrel can't save its encryption key");
    expect(c(2).hints).toEqual(["Keep the file as it is, and write to hello@whimbrelapp.com for help."]);
    expect(c(3).hints[0]).toContain("AppData\\Local");
    expect(c(5).hints[0]).toContain("This needs help from support.");
    expect(c(7).title).toBe("Whimbrel couldn't start");
    expect(c(7).hints).toEqual(["Try again. If this keeps happening, quit Whimbrel and write to hello@whimbrelapp.com."]);
  });

  it("last-action texts", () => {
    expect(lastActionText(null)).toBeNull();
    expect(lastActionText({ action: "restore", outcome: { result: "done" } })).toBeNull();
    expect(lastActionText({ action: "restore", outcome: { result: "partlyDone", movedAside: 2 } })).toContain("moved aside");
    expect(lastActionText({ action: "restore", outcome: { result: "refused" } })).toBe("The copy could not be put back. Nothing was changed.");
    expect(lastActionText({ action: "restore", outcome: { result: "failed" } })).toContain("some files may have moved");
    expect(lastActionText({ action: "reset", outcome: { result: "failed" } })).toBe("The encryption status could not be reset. Try again.");
    const content = screenContent(CASES[4].screen, CASES[4].offers, { action: "restore", outcome: { result: "refused" } });
    expect(content.lastAction).toBe("The copy could not be put back. Nothing was changed.");
  });

  it("error paragraphs by the launch error", () => {
    const p = (e: LaunchErrorView) => errorParagraphs(e).join(" ");
    expect(p({ kind: "plainOpenFailed" })).toContain("could not be opened");
    expect(p({ kind: "keyedOpenFailed", open: "startupFailed" })).toContain("could not start with it");
    expect(p({ kind: "keyedOpenFailed", open: "unsupported" })).toContain("Install the latest version");
    expect(p({ kind: "keyedOpenFailed", open: "damaged" })).toContain("looks damaged");
    expect(p({ kind: "keyedOpenFailed", open: "keyRejected" })).toContain("could not be opened");
    expect(p({ kind: "freshKeyRefused" })).toContain("did not expect");
    expect(p({ kind: "onboardingKeyMismatch" })).toContain("did not expect");
    expect(p({ kind: "keyMissing", source: "dbKey" })).toContain("could not be found");
    expect(p({ kind: "databaseMissingForRecoveredKey" })).toContain("keeps your recovery key until you quit");
    expect(p({ kind: "stepOutOfOrder", code: 3 })).toBe("Whimbrel stopped before opening the memory database, to keep it safe.");
    expect(p({ kind: "decisionUnavailable" })).toBe("Whimbrel stopped before opening the memory database, to keep it safe.");
  });

  it("buttons are disabled while busy", () => {
    const html = renderToStaticMarkup(<BlockingScreen screen={CASES[0].screen} offers={CASES[0].offers} busy onAnswer={() => {}} />);
    expect(html.match(/disabled=""/g)?.length).toBe(2);
  });
});

describe("key unavailable", () => {
  it("Try again is the only action besides Quit; the second identical stage says still unavailable", () => {
    const first = renderToStaticMarkup(<KeyUnavailableScreen stillUnavailable={false} busy={false} onAnswer={() => {}} />);
    expect(first).toContain("Try again");
    expect(first).toContain("Quit Whimbrel");
    expect(first.match(/<button/g)?.length).toBe(2);
    expect(first).not.toContain("still can&#x27;t be read");
    const again = renderToStaticMarkup(<KeyUnavailableScreen stillUnavailable busy={false} onAnswer={() => {}} />);
    expect(again).toContain("The key still can&#x27;t be read.");
  });
});

describe("the migration", () => {
  it("the bar never goes back", () => {
    expect(nextProgress(0.5, 0.4)).toBe(0.5);
    expect(nextProgress(0.5, 0.6)).toBe(0.6);
    expect(nextProgress(0.2, Number.NaN)).toBe(0.2);
    expect(nextProgress(0.2, 7)).toBe(1);
  });

  it("has no Quit and no button at all", () => {
    const html = renderToStaticMarkup(<MigratingScreen overall={0.3} />);
    expect(html).toContain("Encrypting the memory database");
    expect(html).not.toContain("<button");
  });
});

describe("buttons while an answer is in flight", () => {
  const held: LaunchScreenView = { kind: "damaged", reason: "openElsewhere" };
  const offers: LaunchAnswer[] = ["retry", "quit"];
  const disabledAnswers = (busy: boolean) => {
    const html = renderToStaticMarkup(<BlockingScreen screen={held} offers={offers} busy={busy} onAnswer={() => {}} />);
    return [...html.matchAll(/<button[^>]*>/g)].map((m) => m[0]).filter((b) => /disabled/.test(b) && /data-answer/.test(b)).length;
  };

  it("a Retry that brings back the very same screen turns Retry and Quit back on", () => {
    let s = pressReducer(NOT_PRESSED, { kind: "view", generation: 4 });
    s = pressReducer(s, { kind: "pressed" });
    expect(disabledAnswers(s.pressed)).toBe(2);
    // The shell re-reads while it works: same generation, still pressed.
    s = pressReducer(s, { kind: "view", generation: 4 });
    expect(s.pressed).toBe(true);
    // The same in-use screen again (the file still held): a new generation.
    s = pressReducer(s, { kind: "view", generation: 5 });
    expect(s.pressed).toBe(false);
    expect(disabledAnswers(s.pressed)).toBe(0);
  });

  it("an older view resolving late never turns a newer screen's buttons back on", () => {
    // launch://stage and launch://opened each read the state; the older read lands last.
    let s = pressReducer(NOT_PRESSED, { kind: "view", generation: 7 });
    s = pressReducer(s, { kind: "pressed" });
    s = pressReducer(s, { kind: "view", generation: 6 });
    expect(s.pressed).toBe(true);
    expect(s.generation).toBe(7);
    expect(isCurrentView(7, 6)).toBe(false);
    // The same generation may carry fresher state (the wait ended); a newer one always counts.
    expect(isCurrentView(7, 7)).toBe(true);
    expect(isCurrentView(7, 8)).toBe(true);
    expect(isCurrentView(null, 0)).toBe(true);
  });

  it("a refused answer turns the buttons back on at once", () => {
    const s = pressReducer(pressReducer(NOT_PRESSED, { kind: "pressed" }), { kind: "refused" });
    expect(s.pressed).toBe(false);
  });
});
