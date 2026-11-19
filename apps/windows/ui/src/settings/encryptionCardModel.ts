// Settings > Capture & Privacy > Database encryption (brief 3.3.7, port of the Mac's
// EncryptionSettingsPane, EncryptionNotesView and SpeedCheckModel). The card never migrates: the
// Settings opt-in only stores the pending key and sets "migrate at the next launch"; the launch
// does the rest before anything opens. The notes come only from the launch coordinator, each with
// only its own button.

import type {
  CardBadge,
  EncryptOutcome,
  EncryptionSkip,
  NoteAction,
  NoteActionResult,
  NoteRow,
  SetupRefusal,
  SpeedCheckResult,
  SpeedCheckView,
} from "../generated/types";
import { t } from "../l10n";

export function badgeLabel(badge: CardBadge): string {
  if (badge === "encrypted") return t("Encrypted");
  if (badge === "notEncrypted") return t("Not encrypted");
  return t("Needs attention");
}

/** The card's body, by how the launch opened the database. */
export function cardBody(badge: CardBadge): string[] {
  if (badge === "needsAttention") return [t("The memory database could not be opened.")];
  return [
    badge === "encrypted"
      ? t(
          "The memory database on this PC is encrypted with a key kept in Windows Credential Manager. A copy of the database file is unreadable without it. Backups made before you turned this on still hold a readable copy.",
        )
      : t(
          "The memory database is an ordinary file on this PC. Encrypting it makes a copy of the file unreadable without a key kept in Windows Credential Manager.",
        ),
    t("Not encrypted: recordings, slide images, exports, the Obsidian vault, captured screen text files, settings, logs and diagnostics. These stay ordinary files."),
    t("Encryption does not protect against malware running as you, or someone using your PC while it is unlocked."),
  ];
}

/** Why the precheck refused to start the setup (before any key or words exist). */
export function refusalMessage(r: SetupRefusal): string {
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
      return t("There is not enough free disk space to encrypt the memory database. Free up some space and try again.");
    case "openElsewhere":
      return t("Another program has the memory database open. Close it and try again.");
    case "failedBefore":
      return t("Encryption could not be completed. Your data is unchanged.");
    default:
      return t("Encryption did not start. Your data is unchanged. Try again later.");
  }
}

/** What the encrypt step said, when it did not schedule the migration. */
export function encryptMessage(o: EncryptOutcome): string | null {
  if (o === "atNextLaunch") return null;
  if (o === "twoKeysFound") return refusalMessage("twoKeysFound");
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
      return t("Encryption did not start because another program had the memory database open. Whimbrel will try again the next time it starts.");
    default:
      return t("Encryption did not start at this launch. Your data is unchanged. Whimbrel will try again the next time it starts.");
  }
}

export interface NoteLines {
  texts: string[];
  /** The note's only button. */
  action: { action: NoteAction; label: string } | null;
}

function actionLabel(a: NoteAction): string {
  if (a === "retryAfterFailedBefore") return t("Try again");
  if (a === "resetEncryptionStatus") return t("Reset encryption status");
  return t("Delete the unencrypted copy");
}

/** A coordinator note as the card shows it (brief 3.3.7 table). */
export function noteLines(row: NoteRow): NoteLines {
  const n = row.note;
  const action = row.action ? { action: row.action, label: actionLabel(row.action) } : null;
  const texts = ((): string[] => {
    switch (n.kind) {
      case "keyStoreUnavailable":
        return [t("Whimbrel could not read Credential Manager when it started, so encryption cannot start yet. The memory database opened as usual.")];
      case "failedBefore":
        return [t("Encryption could not be completed. Your data is unchanged.")];
      case "migrationSkipped":
        return [n.stateUnreadable ? t("Encryption did not start because Whimbrel cannot read its record of an earlier encryption attempt. Your data is unchanged.") : skipNote(n.skip)];
      case "migrationFailed":
        return [t("Encryption could not be completed at this launch. Your data is unchanged. Whimbrel will try again the next time it starts.")];
      case "encryptionAvailable":
        return [t("This PC already has an encryption key from an earlier attempt. Encrypting the database uses it, so recovery words you kept for it still work.")];
      case "libraryMissing":
        // The app's banner says it (LibraryMissingBanner); the shell keeps it out of the card.
        return [];
      case "plainBackupKept":
        return [
          t("An unencrypted copy of the memory database from before encryption is still on this PC."),
          t("Deleting a file does not guarantee its contents are gone from the disk."),
        ];
      case "restoredCopy":
        return [
          t("Whimbrel put back the unencrypted copy of the memory database. It is not encrypted."),
          ...(n.movedAside > 0 ? [t("Leftover files that were in the way were moved aside next to it.")] : []),
          ...(n.warning === "stateNotMovedAside" ? [t("If encryption does not start later, use Reset encryption status.")] : []),
        ];
      case "pendingKeyNotPromoted":
        return [t("Whimbrel could not finish saving its encryption key in Credential Manager. It will try again the next time it starts. The memory database opened normally.")];
      case "pendingKeyNotDeleted":
        return [t("Whimbrel kept a spare encryption key it found in Credential Manager. The memory database opened normally.")];
      case "recoveredKeyNotStored":
        return [
          n.failure === "noPreviousSlot"
            ? t(
                "Your recovery key opened the memory database, but Credential Manager already holds as many old Whimbrel keys as Whimbrel keeps, so it was not saved. Whimbrel will ask for it again the next time it starts.",
              )
            : t(
                "Your recovery key opened the memory database, but Whimbrel could not save it in Credential Manager. Keep your recovery key at hand: Whimbrel will ask for it again the next time it starts.",
              ),
        ];
      case "recoveredKeyStoreUnverified":
        return [
          t(
            "Your recovery key opened the memory database, but Whimbrel could not confirm it was saved in Credential Manager. If Whimbrel asks for it the next time it starts, use it again.",
          ),
        ];
      case "memoryDatabaseUnavailable":
        return [t("Whimbrel is running without the memory database, so nothing new is saved. Restart Whimbrel to try again.")];
    }
  })();
  return { texts, action };
}

/** What a note's button did, when it did not work (or, for Try again, the scheduled retry). */
export function noteActionMessage(action: NoteAction, result: NoteActionResult): { text: string; error: boolean } | null {
  if (action === "retryAfterFailedBefore") {
    return result === "done"
      ? { text: t("Whimbrel will try again the next time it starts."), error: false }
      : { text: t("Whimbrel could not clear the earlier attempts. Try again."), error: true };
  }
  if (result === "done") return null;
  if (action === "resetEncryptionStatus") return { text: t("The encryption status could not be reset. Try again."), error: true };
  return { text: t("The unencrypted copy could not be deleted. Try again."), error: true };
}

/** A duration in milliseconds with one decimal, in `locale`. */
function ms(value: number | null | undefined, locale: string): string {
  if (value == null || !Number.isFinite(value)) return t("not measured");
  try {
    return new Intl.NumberFormat(locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(value);
  } catch {
    return value.toFixed(1);
  }
}

/** The speed report's lines (Mac `SpeedCheckModel.lines`): timings and the kind of PC only. */
export function speedLines(r: SpeedCheckView, locale = "en"): string[] {
  const lines = [r.encrypted ? t("Memory database: encrypted") : t("Memory database: not encrypted")];
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

// ---- The encrypt flow (phases) ----

export type EncryptPhase =
  | { kind: "idle"; message: string | null }
  | { kind: "checking" }
  | { kind: "settingUp" }
  | { kind: "finishing" }
  | { kind: "atNextLaunch" }
  | { kind: "failed"; message: string };

export interface EncryptActions {
  /** The precheck; null means the setup may start (the shell opened the Settings setup). */
  precheck(): Promise<SetupRefusal | null>;
  /** Hands the tested key to the encrypt step (the shell checks the test passed). */
  continueSetup(): Promise<boolean>;
  /** Stores the pending key or confirms the existing one, then sets the flag. Never migrates. */
  encrypt(): Promise<EncryptOutcome>;
  /** Whether the opened setup wraps an existing key (`db-key` or `db-key-pending`), from the shell. */
  reusesKey?(): Promise<boolean | null>;
  /** Drops the setup's words and recovery text in the shell (Cancel). */
  discard?(): Promise<boolean>;
}

/** idle -> checking -> setting up -> (finishing) -> at next launch, or failed. A precheck answer
 *  that arrives after Cancel is ignored; a refusal goes back to idle with its message. */
export class EncryptFlow {
  private phase: EncryptPhase = { kind: "idle", message: null };
  private listeners = new Set<() => void>();
  private run = 0;
  /** The open setup wraps an existing key: Start over keeps the words. Set by the shell's answer
   *  after the precheck, never guessed from the notes. */
  reusesKey = false;

  constructor(readonly actions: EncryptActions) {}

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  };
  getState = () => this.phase;
  private set(p: EncryptPhase) {
    this.phase = p;
    this.listeners.forEach((l) => l());
  }

  async start() {
    if (this.phase.kind !== "idle") return;
    const run = ++this.run;
    this.set({ kind: "checking" });
    let refusal: SetupRefusal | null;
    try {
      refusal = await this.actions.precheck();
    } catch {
      refusal = "other";
    }
    if (run !== this.run) return;
    if (refusal) return this.set({ kind: "idle", message: refusalMessage(refusal) });
    let reuses: boolean | null = false;
    try {
      reuses = (await this.actions.reusesKey?.()) ?? false;
    } catch {
      reuses = false;
    }
    if (run !== this.run) return;
    this.reusesKey = reuses === true;
    this.set({ kind: "settingUp" });
  }

  /** Cancel: back to idle, and the shell forgets the words and recovery text it made. */
  cancel() {
    if (this.phase.kind === "checking" || this.phase.kind === "settingUp") {
      const opened = this.phase.kind === "settingUp";
      this.run++;
      this.reusesKey = false;
      this.set({ kind: "idle", message: null });
      if (opened) void this.actions.discard?.().catch(() => false);
    }
  }

  /** The setup's Continue ("Encrypt the memory database"). */
  async finish() {
    if (this.phase.kind !== "settingUp") return;
    const run = ++this.run;
    this.set({ kind: "finishing" });
    let outcome: EncryptOutcome;
    try {
      outcome = (await this.actions.continueSetup()) ? await this.actions.encrypt() : "notReady";
    } catch {
      outcome = "failed";
    }
    if (run !== this.run) return;
    const message = encryptMessage(outcome);
    this.set(message === null ? { kind: "atNextLaunch" } : { kind: "failed", message });
  }

  /** OK on a failure, back to idle. */
  dismiss() {
    if (this.phase.kind === "failed") this.set({ kind: "idle", message: null });
  }
}

// ---- The speed check ----

export type SpeedState =
  | { kind: "idle"; message: string | null }
  | { kind: "running"; progress: number }
  | { kind: "done"; report: SpeedCheckView; savedTo: string | null };

export interface SpeedActions {
  run(onProgress: (p: number) => void): Promise<SpeedCheckResult>;
  cancel(): Promise<void>;
  save(): Promise<string | null>;
}

/** Local only: no network. Cancel returns at once and drops the result; no new check starts until
 *  the dropped run has returned (`draining`; core has no stop hook, open decision 5). */
export class SpeedCheck {
  private st: SpeedState = { kind: "idle", message: null };
  private listeners = new Set<() => void>();
  private run = 0;
  /** A cancelled run that has not returned yet. */
  draining = false;

  constructor(readonly actions: SpeedActions) {}

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  };
  getState = () => this.st;
  private set(s: SpeedState) {
    this.st = s;
    this.listeners.forEach((l) => l());
  }

  get canStart() {
    return this.st.kind !== "running" && !this.draining;
  }

  async start() {
    if (!this.canStart) return;
    const run = ++this.run;
    this.set({ kind: "running", progress: 0 });
    let result: SpeedCheckResult;
    try {
      result = await this.actions.run((p) => {
        if (run === this.run && this.st.kind === "running") this.set({ kind: "running", progress: Math.max(this.st.progress, p) });
      });
    } catch {
      result = { kind: "failed" };
    }
    if (run !== this.run) {
      // This run was cancelled: its result is dropped, and a new check may start now.
      this.draining = false;
      this.set({ ...this.st });
      return;
    }
    if (result.kind === "done") this.set({ kind: "done", report: result.report, savedTo: null });
    else if (result.kind === "recording") this.set({ kind: "idle", message: t("A meeting is being recorded. Check the speed when it ends.") });
    else if (result.kind === "cancelled" || result.kind === "draining") this.set({ kind: "idle", message: null });
    else this.set({ kind: "idle", message: t("The speed check could not finish. Try again.") });
  }

  cancel() {
    if (this.st.kind !== "running") return;
    this.run++;
    this.draining = true;
    void this.actions.cancel().catch(() => {});
    this.set({ kind: "idle", message: null });
  }

  async save() {
    if (this.st.kind !== "done") return;
    const report = this.st.report;
    let path: string | null = null;
    try {
      path = await this.actions.save();
    } catch {
      path = null;
    }
    if (path && this.st.kind === "done" && this.st.report === report) this.set({ kind: "done", report, savedTo: path });
  }
}
