// The recovery setup's rules (brief 3.3.1, port of the Mac's EncryptionSetupModel): one model for
// the new-install onboarding and the Settings opt-in. The key and the recovery file's text stay in
// the shell; this model holds the 24 words only while they are shown, and clears them on Continue,
// Start over and unmount. No copy, no clipboard, no logging.

import type { SaveFileResult, SetupBegun } from "../generated/types";
import { t } from "../l10n";

/** Core's minimum, in characters after NFC (`api::encrypt_setup`). */
export const MINIMUM_LENGTH = 12;

/** The passphrase's length as core counts it: Unicode scalar values after NFC. */
export function passphraseLength(s: string): number {
  return [...s.normalize("NFC")].length;
}

/** The shell side of the setup (`setup_*` commands, bound to a scope), or fakes in tests. */
export interface SetupActions {
  begin(passphrase: string): Promise<SetupBegun>;
  saveFile(): Promise<SaveFileResult>;
  verify(passphrase: string): Promise<boolean>;
  startOver(): Promise<boolean>;
  /** Where the shell's setup saved a recovery file, if it did (`setup_saved_to`): after the
   *  window reloaded mid-setup the shell still holds words this screen no longer shows. */
  savedTo?(): Promise<string | null>;
  /** Whether the shell holds words for a fresh key (`setup_has_fresh_material`). Quit asks it, as
   *  closing the window does: after a reload this screen no longer shows the words it holds. */
  hasFreshMaterial?(): Promise<boolean>;
  /** Opens the print dialog over the print-only page. On Windows a print never counts as keeping
   *  the key: WebView2 cannot tell a paper printer from Print to PDF (brief question 2). */
  print(): void;
  /** The setup wraps a key that already exists (the pending key): Start over keeps the words. */
  reusesKey: boolean;
}

export interface SetupState {
  passphrase: string;
  confirmation: string;
  /** The 24 words, while shown. They ARE the key. */
  words: string[] | null;
  creating: boolean;
  createError: string | null;
  savedPath: string | null;
  savedOnSameDisk: boolean;
  saveFailed: boolean;
  saving: boolean;
  wroteDown: boolean;
  testPassphrase: string;
  testing: boolean;
  testPassed: boolean;
  testFailed: boolean;
  savedConfirmed: boolean;
  startOverPending: boolean;
  /** The shell already made words this screen does not show (the window reloaded mid-setup).
   *  Only Start over goes on from here; `savedPath` is a recovery file saved before the reload. */
  alreadyMade: { savedPath: string | null } | null;
}

const EMPTY: SetupState = {
  passphrase: "",
  confirmation: "",
  words: null,
  creating: false,
  createError: null,
  savedPath: null,
  savedOnSameDisk: false,
  saveFailed: false,
  saving: false,
  wroteDown: false,
  testPassphrase: "",
  testing: false,
  testPassed: false,
  testFailed: false,
  savedConfirmed: false,
  startOverPending: false,
  alreadyMade: null,
};

export class SetupModel {
  private st: SetupState = EMPTY;
  private listeners = new Set<() => void>();
  /** Bumped when the passphrase changes or the words are discarded, so a slow answer for an
   *  older passphrase or older words is dropped (the Mac's `generation`). */
  private generation = 0;

  constructor(readonly actions: SetupActions) {}

  // ---- the store, for useSyncExternalStore ----
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  };
  getState = () => this.st;
  private set(patch: Partial<SetupState>) {
    this.st = { ...this.st, ...patch };
    // Losing the last route, or the test, unticks the confirmation.
    if (this.st.savedConfirmed && !this.canConfirmSaved) this.st = { ...this.st, savedConfirmed: false };
    this.listeners.forEach((l) => l());
  }

  // ---- derived rules ----
  get count() {
    return passphraseLength(this.st.passphrase);
  }
  get meetsMinimum() {
    return this.count >= MINIMUM_LENGTH;
  }
  get confirmationMatches() {
    return this.st.confirmation !== "" && this.st.confirmation === this.st.passphrase;
  }
  /** The passphrase fields are read-only once the words exist (Start over to change them). */
  get locked() {
    return this.st.words !== null;
  }
  /** What is wrong with the passphrase pair, or null (also while nothing is typed or locked). */
  get problem(): string | null {
    if (this.st.passphrase === "" || this.locked) return null;
    if (!this.meetsMinimum) return t("Use at least {minimum} characters.", { minimum: MINIMUM_LENGTH });
    if (this.st.confirmation !== "" && this.st.confirmation !== this.st.passphrase) return t("The two passphrases do not match.");
    return null;
  }
  get canCreate() {
    return this.meetsMinimum && this.confirmationMatches && !this.st.creating && !this.locked && !this.st.alreadyMade;
  }
  get canTest() {
    return this.locked && this.st.testPassphrase !== "" && !this.st.testing;
  }
  /** The key has been kept somewhere other than the PC: the file saved, or the words written down.
   *  Printing does not count on Windows (see `SetupActions.print`). */
  get keptSomewhere() {
    return this.st.savedPath !== null || this.st.wroteDown;
  }
  get canConfirmSaved() {
    return this.st.testPassed && this.keptSomewhere;
  }
  get canContinue() {
    return this.locked && this.canConfirmSaved && this.st.savedConfirmed;
  }
  /** Quit asks first only when quitting would waste words for a fresh key. */
  get quitNeedsConfirm() {
    return !this.actions.reusesKey && this.locked;
  }
  /** What Quit decides on: the words on screen, else the shell's own answer (after a reload the
   *  shell still holds words this screen lost). A shell that cannot answer means ask first. */
  async quitAsksFirst(): Promise<boolean> {
    if (this.quitNeedsConfirm) return true;
    if (!this.actions.hasFreshMaterial) return false;
    try {
      return await this.actions.hasFreshMaterial();
    } catch {
      return true;
    }
  }
  get startOverTitle() {
    return this.actions.reusesKey ? t("Change the passphrase for your recovery file?") : t("Start over with a new recovery key?");
  }
  get startOverBody() {
    return this.actions.reusesKey
      ? t("Your 24 words stay the same. A recovery file you already saved still opens with its old passphrase.")
      : t("Any recovery file or printout you already made will not work with the new one.");
  }

  // ---- edits ----
  setPassphrase(v: string) {
    if (this.locked || v === this.st.passphrase) return;
    this.generation++;
    this.set({ passphrase: v, createError: null });
  }
  setConfirmation(v: string) {
    if (this.locked || v === this.st.confirmation) return;
    this.generation++;
    this.set({ confirmation: v, createError: null });
  }
  setWroteDown(v: boolean) {
    if (!this.locked) return;
    this.set({ wroteDown: v });
  }
  /** Editing the test passphrase clears the result (and so the confirmation). */
  setTestPassphrase(v: string) {
    if (v === this.st.testPassphrase) return;
    this.set({ testPassphrase: v, testPassed: false, testFailed: false });
  }
  /** Refuses to tick until the test passed and the key is kept somewhere. */
  setSavedConfirmed(v: boolean) {
    this.set({ savedConfirmed: v && this.canConfirmSaved });
  }

  // ---- actions ----
  async create() {
    if (!this.canCreate) return;
    const started = this.generation;
    this.set({ creating: true, createError: null });
    let made: SetupBegun;
    try {
      made = await this.actions.begin(this.st.passphrase);
    } catch {
      made = { kind: "failed" };
    }
    if (started !== this.generation) {
      this.set({ creating: false });
      return;
    }
    if (made.kind === "made") this.set({ creating: false, words: made.words, alreadyMade: null });
    else if (made.kind === "alreadyMade") {
      let savedPath: string | null = null;
      try {
        savedPath = (await this.actions.savedTo?.()) ?? null;
      } catch {
        savedPath = null;
      }
      if (started !== this.generation) return this.set({ creating: false });
      this.set({ creating: false, alreadyMade: { savedPath } });
    }
    else if (made.kind === "passphraseTooShort") this.set({ creating: false, createError: t("Use at least {minimum} characters.", { minimum: MINIMUM_LENGTH }) });
    else this.set({ creating: false, createError: t("The recovery key could not be made. Try again.") });
  }

  async saveFile() {
    if (!this.locked || this.st.saving) return;
    const started = this.generation;
    this.set({ saving: true });
    let r: SaveFileResult;
    try {
      r = await this.actions.saveFile();
    } catch {
      r = { kind: "failed" };
    }
    if (started !== this.generation) return this.set({ saving: false });
    if (r.kind === "saved") this.set({ saving: false, savedPath: r.path, savedOnSameDisk: r.sameDiskAsDatabase, saveFailed: false });
    else if (r.kind === "cancelled") this.set({ saving: false });
    else this.set({ saving: false, saveFailed: true });
  }

  print() {
    if (this.locked) this.actions.print();
  }

  async test() {
    if (!this.canTest) return;
    const typed = this.st.testPassphrase;
    const started = this.generation;
    this.set({ testing: true });
    let ok = false;
    try {
      ok = await this.actions.verify(typed);
    } catch {
      ok = false;
    }
    // A result for text the user has since changed, or for discarded words, is dropped.
    if (typed !== this.st.testPassphrase || started !== this.generation) return this.set({ testing: false });
    this.set({ testing: false, testPassed: ok, testFailed: !ok });
  }

  /** Start over asks first, both with the words shown and from the reload notice (that one voids
   *  a saved recovery file too). */
  requestStartOver() {
    if (this.locked || (this.st.alreadyMade && !this.st.creating)) this.set({ startOverPending: true });
  }
  cancelStartOver() {
    this.set({ startOverPending: false });
  }
  /** Drops the words (the shell drops its material) and clears every input and result. */
  async confirmStartOver() {
    if (!this.st.startOverPending) return;
    if (this.st.alreadyMade) {
      this.set({ startOverPending: false });
      return this.startOverAfterReload();
    }
    this.generation++;
    this.clear();
    try {
      await this.actions.startOver();
    } catch {
      // The shell refusing leaves nothing on screen to act on; Create asks again.
    }
  }

  /** After a reload mid-setup: the shell drops the words it made (a reused key keeps them), and
   *  Create runs again with the passphrase still typed. */
  async startOverAfterReload() {
    if (!this.st.alreadyMade || this.st.creating) return;
    this.set({ alreadyMade: null });
    try {
      await this.actions.startOver();
    } catch {
      // Create below answers again (alreadyMade, or a failure) if the shell refused.
    }
    await this.create();
  }

  /** What the reload notice says: why only Start over is offered, and which saved file it voids. */
  get alreadyMadeLines(): string[] {
    const a = this.st.alreadyMade;
    if (!a) return [];
    const lines = [t("A recovery key was already made before this window reloaded. Start over to make it again.")];
    if (a.savedPath && !this.actions.reusesKey)
      lines.push(t("The recovery file saved to {path} will not work with the new key.", { path: a.savedPath }));
    return lines;
  }

  /** Forgets everything the screen showed or was typed (Continue, Start over, unmount). */
  clear() {
    this.generation++;
    this.st = EMPTY;
    this.listeners.forEach((l) => l());
  }
}
