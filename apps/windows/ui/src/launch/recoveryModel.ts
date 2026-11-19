// The recovery screen's rules (brief 3.3.4, port of the Mac's EncryptionRecoveryModel). Its only
// actions restore a key: from the recovery file and its passphrase, or from the 24 words. The shell
// restores the key, hands it to the waiting launch, which proves it opens the database before
// anything is written, and answers with what the launch did. Nothing here starts fresh, creates a
// database or deletes one.

import type { ChosenFile, RecoveryReasonView, UseKeyResult } from "../generated/types";
import { t } from "../l10n";

export type RecoveryMethod = "file" | "words";

export interface RecoveryActions {
  chooseFile(): Promise<ChosenFile>;
  unlockFile(passphrase: string): Promise<UseKeyResult>;
  unlockWords(words: string): Promise<UseKeyResult>;
}

export interface RecoveryState {
  method: RecoveryMethod;
  fileName: string | null;
  passphrase: string;
  words: string;
  working: boolean;
  error: string | null;
  /** The key opened the database: the launch goes on. */
  restored: boolean;
}

const EMPTY: RecoveryState = { method: "file", fileName: null, passphrase: "", words: "", working: false, error: null, restored: false };

/** Why the screen shows, as the Mac explains it. */
export function recoveryExplanation(reason: RecoveryReasonView): string {
  return reason === "needsKey"
    ? t(
        "This PC has no key for the encrypted memory database. That happens after a new Windows user account, when Credential Manager was cleared, or when the data folder came from another PC.",
      )
    : t("The key in Windows Credential Manager does not open this memory database.");
}

/** The copy for an unlock that did not open the database, or null when it did. */
export function unlockMessage(result: UseKeyResult): string | null {
  switch (result) {
    case "opened":
      return null;
    case "keyRejected":
      return t("That recovery key does not open this memory database. It may belong to another PC. Try another recovery file or set of words.");
    case "wrongPassphrase":
      return t("That passphrase does not open this recovery file, or the file is damaged. Check the passphrase and try again.");
    case "notARecoveryFile":
      return t("This is not a Whimbrel recovery file. Choose the file named Whimbrel recovery key.txt.");
    case "badWords":
      return t("Those words are not a valid recovery key. Check that all 24 words are there, in order, and spelled correctly.");
    case "noFileChosen":
      return t("That file could not be read. Choose it again.");
    default:
      return t("The recovery key could not be used. Your data has not been changed. Try again.");
  }
}

export class RecoveryModel {
  private st: RecoveryState = EMPTY;
  private listeners = new Set<() => void>();

  constructor(readonly actions: RecoveryActions) {}

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  };
  getState = () => this.st;
  private set(patch: Partial<RecoveryState>) {
    this.st = { ...this.st, ...patch };
    this.listeners.forEach((l) => l());
  }

  get canUnlockFile() {
    return !this.st.working && this.st.fileName !== null && this.st.passphrase !== "";
  }
  get canUnlockWords() {
    return !this.st.working && this.st.words.trim() !== "";
  }

  /** Switching the way in clears the error. */
  setMethod(method: RecoveryMethod) {
    if (method !== this.st.method) this.set({ method, error: null });
  }
  setPassphrase(v: string) {
    this.set({ passphrase: v });
  }
  setWords(v: string) {
    this.set({ words: v });
  }

  async chooseFile() {
    if (this.st.working) return;
    let chosen: ChosenFile;
    try {
      chosen = await this.actions.chooseFile();
    } catch {
      chosen = { kind: "unreadable" };
    }
    if (chosen.kind === "chosen") this.set({ fileName: chosen.name, error: null });
    else if (chosen.kind === "unreadable") this.set({ fileName: null, error: t("That file could not be read. Choose it again.") });
    else if (chosen.kind === "refused") this.set({ error: t("The recovery key could not be used. Your data has not been changed. Try again.") });
  }

  unlockFile() {
    if (!this.canUnlockFile) return Promise.resolve();
    return this.unlock(() => this.actions.unlockFile(this.st.passphrase));
  }
  unlockWords() {
    if (!this.canUnlockWords) return Promise.resolve();
    return this.unlock(() => this.actions.unlockWords(this.st.words));
  }

  private async unlock(run: () => Promise<UseKeyResult>) {
    this.set({ working: true, error: null });
    let result: UseKeyResult;
    try {
      result = await run();
    } catch {
      result = "failed";
    }
    if (result === "opened") {
      // The launch goes on with the key: forget what was typed.
      this.st = { ...EMPTY, method: this.st.method, restored: true };
      this.listeners.forEach((l) => l());
      return;
    }
    this.set({ working: false, error: unlockMessage(result) });
  }

  clear() {
    this.st = EMPTY;
    this.listeners.forEach((l) => l());
  }
}
