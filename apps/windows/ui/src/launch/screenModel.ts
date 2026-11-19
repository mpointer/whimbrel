// The blocking launch screens drawn from their type (brief 3.3.6, port of the Mac's
// LaunchScreenContent.of). The buttons come only from the screen's `offers`, which the shell
// computes from `LaunchScreen::offers`: the webview never offers an answer the coordinator would
// refuse. Every screen has Quit; none starts fresh, creates a database or deletes one.

import type { LastActionView, LaunchAnswer, LaunchErrorView, LaunchScreenView } from "../generated/types";
import { t } from "../l10n";

export interface ScreenButton {
  answer: LaunchAnswer;
  label: string;
  /** Asked before the answer is sent (restore and reset). */
  confirm?: { title: string; body: string; confirm: string; destructive: boolean };
  /** The screen's main action. */
  primary: boolean;
}

export interface ScreenContent {
  title: string;
  paragraphs: string[];
  hints: string[];
  /** The previous press did not finish (error colour). */
  lastAction: string | null;
  buttons: ScreenButton[];
}

/** Why the launch stopped (Mac `errorParagraphs`). */
export function errorParagraphs(e: LaunchErrorView): string[] {
  switch (e.kind) {
    case "plainOpenFailed":
      return [t("The memory database could not be opened. Whimbrel has not changed it.")];
    case "keyedOpenFailed":
      if (e.open === "startupFailed") return [t("The memory database opened, but Whimbrel could not start with it.")];
      if (e.open === "unsupported") return [t("This copy of Whimbrel cannot open an encrypted database. Install the latest version.")];
      if (e.open === "damaged") return [t("The memory database file looks damaged. Whimbrel has not changed it.")];
      return [t("The memory database could not be opened. Whimbrel has not changed it.")];
    case "onboardingKeyMismatch":
    case "freshKeyRefused":
      return [t("Whimbrel found an encryption key it did not expect, so it did not make a new one. Nothing was created.")];
    case "keyMissing":
      return [t("The encryption key could not be found while Whimbrel was starting. Nothing was opened or changed.")];
    case "databaseMissingForRecoveredKey":
      return [
        t("Whimbrel has your recovery key, but the memory database file is not there now. Put the file back, then try again. Whimbrel keeps your recovery key until you quit."),
      ];
    default:
      return [t("Whimbrel stopped before opening the memory database, to keep it safe.")];
  }
}

/** The last press on the encrypted-file-missing screen, when it did not finish. */
export function lastActionText(last: LastActionView | null | undefined): string | null {
  if (!last || last.outcome.result === "done") return null;
  if (last.action === "reset") return t("The encryption status could not be reset. Try again.");
  if (last.outcome.result === "partlyDone") return t("Some leftover files were moved aside, but the copy was not put back yet. Try again.");
  if (last.outcome.result === "refused") return t("The copy could not be put back. Nothing was changed.");
  return t("The copy could not be put back, and some files may have moved. Try again, or quit and write to hello@whimbrelapp.com.");
}

function button(answer: LaunchAnswer): ScreenButton {
  switch (answer) {
    case "retry":
      return { answer, label: t("Try again"), primary: true };
    case "quit":
      return { answer, label: t("Quit Whimbrel"), primary: false };
    case "restoreRollbackCopy":
      return {
        answer,
        label: t("Put back the unencrypted copy…"),
        primary: true,
        confirm: {
          title: t("Put back the unencrypted copy?"),
          body: t("Whimbrel will open the copy from before encryption. It is not encrypted, and anything added after encryption started is not in it."),
          confirm: t("Put it back"),
          destructive: false,
        },
      };
    case "resetEncryptionStatus":
      return {
        answer,
        label: t("Reset encryption status…"),
        primary: true,
        confirm: {
          title: t("Reset encryption status and start empty?"),
          body: t(
            "Whimbrel will start with an empty memory database. Nothing is deleted: only Whimbrel's record of the earlier encryption is set aside. If you can find the memory database, for example in a backup, quit Whimbrel, put it back, and open Whimbrel again.",
          ),
          confirm: t("Reset and start empty"),
          destructive: true,
        },
      };
    case "continueWithoutStore":
      return { answer, label: t("Continue without the memory database"), primary: false };
  }
}

/** Quit last, the main action first: the order a screen shows its offers in. */
const ORDER: LaunchAnswer[] = ["retry", "restoreRollbackCopy", "resetEncryptionStatus", "continueWithoutStore", "quit"];

/** What a blocking screen shows. `offers` is the shell's list; nothing else becomes a button. */
export function screenContent(screen: LaunchScreenView, offers: readonly LaunchAnswer[], last?: LastActionView | null): ScreenContent {
  const buttons = ORDER.filter((a) => offers.includes(a)).map(button);
  const base = { lastAction: null as string | null, buttons };
  switch (screen.kind) {
    case "keyStorageUnavailable":
      return {
        ...base,
        title: t("Whimbrel can't save its encryption key"),
        paragraphs: [
          t(
            "Whimbrel keeps the key for the memory database in Windows Credential Manager, and saves it there before it creates the database. Windows did not allow that.",
          ),
        ],
        hints: [t("Check that you are signed in to Windows with your own user account, then try again. Whimbrel has not created anything.")],
      };
    case "migrationInterrupted":
      return {
        ...base,
        title: t("Encrypting the memory database was interrupted"),
        paragraphs: [
          t("Whimbrel could not finish or undo the encryption it started earlier. Your data is still on this PC. Try again, and Whimbrel will finish or undo it."),
        ],
        hints: [],
      };
    case "damaged":
      // In use by another program (core 0b9f8deb): not damage. Retry, like no access.
      if (screen.reason === "openElsewhere")
        return {
          ...base,
          title: t("Whimbrel can't open your memory database"),
          paragraphs: [t("Another program has the memory database file open, so Whimbrel cannot read it. Whimbrel has not changed it.")],
          hints: [t("Close the other program (a second copy of Whimbrel, a backup tool or antivirus), then try again.")],
        };
      return screen.reason === "noAccess"
        ? {
            ...base,
            title: t("Whimbrel can't open your memory database"),
            paragraphs: [t("Windows did not let Whimbrel read the memory database file. Whimbrel has not changed it.")],
            hints: [t("Check that your Windows user account can open the Whimbrel folder in AppData\\Local, then try again.")],
          }
        : {
            ...base,
            title: t("Whimbrel can't open your memory database"),
            paragraphs: [t("The memory database file looks damaged, so Whimbrel cannot open it. Whimbrel has not changed it and will not delete or replace it.")],
            hints: [t("Keep the file as it is, and write to hello@whimbrelapp.com for help.")],
          };
    case "encryptedFileMissing":
      if (screen.backupPresent)
        return {
          ...base,
          title: t("Whimbrel can't find the memory database"),
          paragraphs: [
            t(
              "The encrypted memory database is missing, but the unencrypted copy from before encryption is still on this PC. Whimbrel can put that copy back and open it.",
            ),
          ],
          hints: [
            t(
              "The copy holds your data from before encryption started. Anything added after that was in the encrypted file. If you can find the encrypted file, for example in a backup, put it back instead and try again.",
            ),
          ],
          lastAction: lastActionText(last),
        };
      if (screen.encryptedCopyPresent)
        return {
          ...base,
          title: t("Whimbrel can't find the memory database"),
          paragraphs: [t("The memory database is missing. Only an unfinished encrypted copy is left, and Whimbrel will not change it.")],
          hints: [
            t("This needs help from support. Quit Whimbrel, leave the Whimbrel folder in AppData\\Local as it is, and write to hello@whimbrelapp.com."),
          ],
        };
      return {
        ...base,
        title: t("Whimbrel can't find the memory database"),
        paragraphs: [t("Whimbrel found no memory database and no copy of it. Reset sets aside Whimbrel's record of the earlier encryption; nothing is deleted.")],
        hints: [
          t(
            "If you have a backup of the memory database, quit Whimbrel, put the backup back, and open Whimbrel again. If not, use Reset. For help, write to hello@whimbrelapp.com.",
          ),
        ],
        lastAction: lastActionText(last),
      };
    case "error":
      return {
        ...base,
        title: t("Whimbrel couldn't start"),
        paragraphs: errorParagraphs(screen.error),
        hints: [t("Try again. If this keeps happening, quit Whimbrel and write to hello@whimbrelapp.com.")],
      };
    case "keyUnavailable":
    case "recovery":
      // Drawn as their own stages (KeyUnavailableScreen, RecoveryView); here only so a stray one
      // still shows Quit.
      return { ...base, title: t("Whimbrel couldn't start"), paragraphs: [t("Whimbrel stopped before opening the memory database, to keep it safe.")], hints: [] };
  }
}

/** The migration bar never goes back (`launch://progress` can arrive out of order). */
export function nextProgress(shown: number, reported: number): number {
  const r = Number.isFinite(reported) ? Math.min(1, Math.max(0, reported)) : 0;
  return Math.max(shown, r);
}

/** Whether the user's answer is in flight, and the screen it answered. */
export interface PressState {
  pressed: boolean;
  /** The generation of the screen last shown (`LaunchView.generation`). */
  generation: number | null;
}

export const NOT_PRESSED: PressState = { pressed: false, generation: null };

export type PressEvent = { kind: "pressed" } | { kind: "refused" } | { kind: "view"; generation: number };

/** Buttons go off when the user answers and back on when the shell shows a screen again. The
 *  shell's generation moves on every screen it shows, so a Retry that brings back the very same
 *  screen (the file still held, the key store still silent) re-enables them; comparing the stage
 *  alone left every button off, Quit included. A refused or failed answer re-enables at once.
 *  Only a NEWER generation counts: two reads (launch://stage and launch://opened) can resolve out
 *  of order, and an older view arriving late must not re-enable the buttons of a newer screen. */
export function pressReducer(s: PressState, e: PressEvent): PressState {
  switch (e.kind) {
    case "pressed":
      return { ...s, pressed: true };
    case "refused":
      return { ...s, pressed: false };
    case "view":
      return e.generation > (s.generation ?? -1) ? { pressed: false, generation: e.generation } : s;
  }
}

/** Whether a view read from the shell may replace the one shown. The same generation may carry
 *  fresher state (an answer stops the wait without a new screen); an older one resolved late. */
export function isCurrentView(shown: number | null, read: number): boolean {
  return read >= (shown ?? -1);
}
