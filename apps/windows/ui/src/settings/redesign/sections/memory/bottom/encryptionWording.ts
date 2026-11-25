// Library encryption, in the redesign's own words: every sentence the card, the encrypt wizard and its messages can
// show says "your library", never "the memory database". These are redesign-only t() keys with their own translations;
// the old card's strings (settings/encryptionCardModel.ts, launch/SetupView.tsx) are not touched and not rewritten by
// a text replace. A line that has no library word in it is the same literal as the old card's, so it shares its key.

import type { CardBadge, EncryptOutcome, EncryptionSkip, NoteAction, NoteActionResult, NoteRow, SetupRefusal, SpeedCheckView } from "../../../../../generated/types";
import { t } from "../../../../../l10n";
import type { EncryptSectionWords } from "../../../../EncryptionCard";
import { badgeLabel, noteActionMessage, type NoteLines } from "../../../../encryptionCardModel";

export const libraryBadge = (badge: CardBadge): { tone: "good" | "neutral" | "warning"; label: string } => ({
  tone: badge === "encrypted" ? "good" : badge === "notEncrypted" ? "neutral" : "warning",
  label: badgeLabel(badge),
});

/** The card's body, by how the launch opened the library. */
export function libraryBody(badge: CardBadge): string[] {
  if (badge === "needsAttention") return [t("Your library could not be opened.")];
  return [
    badge === "encrypted"
      ? t(
          "Your library on this PC is encrypted with a key kept in Windows Credential Manager. A copy of the library file is unreadable without it. Backups made before you turned this on still hold a readable copy.",
        )
      : t("Your library is an ordinary file on this PC. Encrypting it makes a copy of the file unreadable without a key kept in Windows Credential Manager."),
    t("Not encrypted: recordings, slide images, exports, the Obsidian vault, captured screen text files, settings, logs and diagnostics. These stay ordinary files."),
    t("Encryption does not protect against malware running as you, or someone using your PC while it is unlocked."),
  ];
}

export const unencryptedCopyNote = (): string =>
  t(
    "Whimbrel keeps an unencrypted copy from before encryption until it has started normally three times, then deletes it. Deleting a file does not guarantee its contents are gone from the disk.",
  );

/** Why the precheck refused to start the setup (before any key or words exist). */
export function libraryRefusal(r: SetupRefusal): string {
  switch (r) {
    case "keychainUnavailable":
      return t("Whimbrel can't read Credential Manager right now, so encryption cannot start. Try again in a moment.");
    case "twoKeysFound":
      return t(
        "This PC holds two different encryption keys from earlier attempts, so encryption cannot start. Your data is unchanged. Write to hello@whimbrelapp.com for help.",
      );
    case "recording":
      return t("A meeting is being recorded. Try again when it ends. Your data is unchanged.");
    case "lowDisk":
      return t("There is not enough free disk space to encrypt your library. Free up some space and try again.");
    case "openElsewhere":
      return t("Another program has your library open. Close it and try again.");
    case "failedBefore":
      return t("Encryption could not be completed. Your data is unchanged.");
    default:
      return t("Encryption did not start. Your data is unchanged. Try again later.");
  }
}

/** What the encrypt step said, when it did not schedule the migration. */
export function libraryEncryptMessage(o: EncryptOutcome): string | null {
  if (o === "atNextLaunch") return null;
  if (o === "twoKeysFound") return libraryRefusal("twoKeysFound");
  return t("Encryption could not be completed. Your data is unchanged.");
}

function skipNote(skip: EncryptionSkip | null | undefined): string {
  switch (skip?.reason) {
    case "recording":
      return t("Encryption did not start because a meeting was being recorded. Whimbrel will try again the next time it starts.");
    case "low_disk":
    case "free_space_unknown":
      return t("Encryption did not start because there was not enough free disk space. Free up some space, and Whimbrel will try again the next time it starts.");
    case "open_elsewhere":
      return t("Encryption did not start because another program had your library open. Whimbrel will try again the next time it starts.");
    default:
      return t("Encryption did not start at this launch. Your data is unchanged. Whimbrel will try again the next time it starts.");
  }
}

function actionLabel(a: NoteAction): string {
  if (a === "retryAfterFailedBefore") return t("Try again");
  if (a === "resetEncryptionStatus") return t("Reset encryption status");
  return t("Delete the unencrypted copy");
}

/** A launch note as the card shows it. The same cases as the old card's `noteLines`, in the redesign's words. */
export function libraryNote(row: NoteRow): NoteLines {
  const n = row.note;
  const action = row.action ? { action: row.action, label: actionLabel(row.action) } : null;
  const texts = ((): string[] => {
    switch (n.kind) {
      case "keyStoreUnavailable":
        return [t("Whimbrel could not read Credential Manager when it started, so encryption cannot start yet. Your library opened as usual.")];
      case "failedBefore":
        return [t("Encryption could not be completed. Your data is unchanged.")];
      case "migrationSkipped":
        return [n.stateUnreadable ? t("Encryption did not start because Whimbrel cannot read its record of an earlier encryption attempt. Your data is unchanged.") : skipNote(n.skip)];
      case "migrationFailed":
        return [t("Encryption could not be completed at this launch. Your data is unchanged. Whimbrel will try again the next time it starts.")];
      case "encryptionAvailable":
        return [t("This PC already has an encryption key from an earlier attempt. Encrypting your library uses it, so recovery words you kept for it still work.")];
      case "libraryMissing":
        // The app's banner says it; the shell keeps it out of the card.
        return [];
      case "plainBackupKept":
        return [
          t("An unencrypted copy of your library from before encryption is still on this PC."),
          t("Deleting a file does not guarantee its contents are gone from the disk."),
        ];
      case "restoredCopy":
        return [
          t("Whimbrel put back the unencrypted copy of your library. It is not encrypted."),
          ...(n.movedAside > 0 ? [t("Leftover files that were in the way were moved aside next to it.")] : []),
          ...(n.warning === "stateNotMovedAside" ? [t("If encryption does not start later, use Reset encryption status.")] : []),
        ];
      case "pendingKeyNotPromoted":
        return [t("Whimbrel could not finish saving its encryption key in Credential Manager. It will try again the next time it starts. Your library opened normally.")];
      case "pendingKeyNotDeleted":
        return [t("Whimbrel kept a spare encryption key it found in Credential Manager. Your library opened normally.")];
      case "recoveredKeyNotStored":
        return [
          n.failure === "noPreviousSlot"
            ? t(
                "Your recovery key opened your library, but Credential Manager already holds as many old Whimbrel keys as Whimbrel keeps, so it was not saved. Whimbrel will ask for it again the next time it starts.",
              )
            : t(
                "Your recovery key opened your library, but Whimbrel could not save it in Credential Manager. Keep your recovery key at hand: Whimbrel will ask for it again the next time it starts.",
              ),
        ];
      case "recoveredKeyStoreUnverified":
        return [
          t(
            "Your recovery key opened your library, but Whimbrel could not confirm it was saved in Credential Manager. If Whimbrel asks for it the next time it starts, use it again.",
          ),
        ];
      case "memoryDatabaseUnavailable":
        return [t("Whimbrel is running without your library, so nothing new is saved. Restart Whimbrel to try again.")];
    }
  })();
  return { texts, action };
}

/** What a note's button did. None of these sentences name the library, so they are the old card's own. */
export function libraryNoteActionMessage(action: NoteAction, result: NoteActionResult) {
  return noteActionMessage(action, result);
}

export const fastSearchHelp = (): string =>
  t("Gives search up to 1 GB of extra memory. Takes effect after you restart Whimbrel, and only applies once your library is encrypted.");

/** A duration in milliseconds with one decimal, in `locale`. */
function ms(value: number | null | undefined, locale: string): string {
  if (value == null || !Number.isFinite(value)) return t("not measured");
  try {
    return new Intl.NumberFormat(locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(value);
  } catch {
    return value.toFixed(1);
  }
}

/** The speed report's lines: timings and the kind of PC only. */
export function librarySpeedLines(r: SpeedCheckView, locale = "en"): string[] {
  const lines = [r.encrypted ? t("Your library: encrypted") : t("Your library: not encrypted")];
  lines.push(t("Opening: {ms} ms", { ms: ms(r.openMs, locale) }));
  if (r.tooLittleData) {
    lines.push(t("Not enough data to time searches yet."));
  } else {
    lines.push(t("Word search: {typical} ms typical, {slow} ms at the slow end", { typical: ms(r.keywordP50Ms, locale), slow: ms(r.keywordP95Ms, locale) }));
    lines.push(
      r.semanticUnavailable
        ? t("Search by meaning: not timed, the search model is not available")
        : t("Search by meaning: {typical} ms typical, {slow} ms at the slow end", { typical: ms(r.semanticP50Ms, locale), slow: ms(r.semanticP95Ms, locale) }),
    );
  }
  lines.push(t("This PC: {arch}, {ram} GB of memory, {os}", { arch: r.cpuArch, ram: r.ramGib != null ? String(r.ramGib) : t("not measured"), os: r.os }));
  return lines;
}

/** Everything the encrypt wizard shows that names what is encrypted, for the redesigned card to pass to `EncryptSection`.
 *  Built when asked, never kept, so the card draws it again in the new language after a change of language. */
export function libraryEncryptWords(): EncryptSectionWords {
  const wordsWarning = t(
    "These words are the key itself. Your passphrase does not protect them. Anyone who has these words or this QR code can read your library. Write them down or print them, and keep them somewhere safe and offline.",
  );
  return {
    refusal: libraryRefusal,
    encrypt: libraryEncryptMessage,
    atNextLaunch: t("Whimbrel will finish encrypting your library the next time it starts."),
    continueTitle: t("Encrypt your library"),
    startButton: t("Encrypt your library…"),
    setup: {
      intro: t(
        "Whimbrel will encrypt your library on this PC with a key kept in Windows Credential Manager. If that key is ever lost, your recovery key is the only way back into your library. Make it now, before anything is encrypted.",
      ),
      sameDisk: t("This file is on the same disk as your library, so it may be lost with it. Keep a copy somewhere other than this PC as well."),
      wordsWarning,
      // The printed page says the same thing about the words as the screen does.
      printNotice: wordsWarning,
      printFooter: t("Keep this page somewhere safe and offline. Anyone who has it can read your library on the PC it was made on."),
      footer: t(
        "You only need the recovery key if this PC loses its own key. If that happens and you have neither the 24 words nor the recovery file and its passphrase, nobody can unlock your library, including Whimbrel. If you use sync, your other devices keep their own copies of what they synced, but not the key.",
      ),
    },
  };
}
