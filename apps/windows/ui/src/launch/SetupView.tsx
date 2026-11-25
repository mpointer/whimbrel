// The recovery setup screen (brief 3.3.1, port of the Mac's EncryptionSetupView): the new-install
// onboarding at launch and the Settings opt-in. The rules are in setupModel.ts; this draws them.

import { useEffect, useState, useSyncExternalStore } from "react";
import { t } from "../l10n";
import { MINIMUM_LENGTH, type SetupModel } from "./setupModel";
import { PrintPortal, WordsGrid, WordsQr } from "./RecoveryWords";

export interface SetupViewProps {
  model: SetupModel;
  /** "Continue" at launch, "Encrypt the memory database" in Settings. */
  continueTitle: string;
  /** Hands the key on (the shell does it); the caller clears the model. */
  onContinue: () => void;
  continuing?: boolean;
  /** Launch only: quits Whimbrel (after the guard). */
  onQuit?: () => void;
  /** Bumped when closing the window asked to quit (launch://quit-requested): shows the guard. */
  quitSignal?: number;
  /** Settings only: back to the card. */
  onCancel?: () => void;
  /** Launch only: one line above the title, such as the one after Delete everything. */
  notice?: string | null;
  /** The four sentences that name what is encrypted. Left out, the launch's words ("the memory database"); the
   *  redesigned Settings card passes its own, so the launch screen's strings are untouched. */
  words?: SetupWords;
  /** Inside another card (the redesigned Settings card): the title is a level-4 heading and the steps level 5, and
   *  the steps are flat sections, not cards inside a card. Left out, the launch screen's and the old tab's look. */
  embedded?: boolean;
}

export interface SetupWords {
  intro: string;
  sameDisk: string;
  wordsWarning: string;
  footer: string;
  /** The printed page's notice and footer. */
  printNotice: string;
  printFooter: string;
}

export function SetupView({ model, continueTitle, onContinue, continuing = false, onQuit, quitSignal = 0, onCancel, notice = null, words: given, embedded = false }: SetupViewProps) {
  const s = useSyncExternalStore(model.subscribe, model.getState, model.getState);
  const [askingQuit, setAskingQuit] = useState(false);
  // The default sentences are the launch's own (literal t() calls, so they are extracted and unchanged).
  const words: SetupWords = given ?? {
    intro: t(
      "Whimbrel will encrypt the memory database on this PC with a key kept in Windows Credential Manager. If that key is ever lost, your recovery key is the only way back into the memory database. Make it now, before anything is encrypted.",
    ),
    sameDisk: t("This file is on the same disk as your memory database, so it may be lost with it. Keep a copy somewhere other than this PC as well."),
    wordsWarning: t(
      "These words are the key itself. Your passphrase does not protect them. Anyone who has these words or this QR code can read the memory database. Write them down or print them, and keep them somewhere safe and offline.",
    ),
    footer: t(
      "You only need the recovery key if this PC loses its own key. If that happens and you have neither the 24 words nor the recovery file and its passphrase, nobody can unlock the memory database, including Whimbrel. If you use sync, your other devices keep their own copies of what they synced, but not the key.",
    ),
    printNotice: "",
    printFooter: "",
  };
  // Only the redesigned card has its own print words; the launch's page keeps its own sentences.
  const printCopy = given ? { notice: given.printNotice, footer: given.printFooter } : undefined;
  const Title = embedded ? "h4" : "h1";
  const Step = embedded ? "h5" : "h2";
  // A step is a card on its own screen and a flat section inside another card.
  const stepClass = embedded ? "setup-step stack-sm" : "card stack-sm";
  // Leaving the screen forgets what it showed and what was typed.
  useEffect(() => () => model.clear(), [model]);
  useEffect(() => {
    if (quitSignal > 0) setAskingQuit(true);
  }, [quitSignal]);

  const quit = () => {
    if (!onQuit) return;
    // Ask the shell too, as closing the window does: after a reload it still holds words this
    // screen no longer shows.
    void model.quitAsksFirst().then((ask) => (ask ? setAskingQuit(true) : onQuit()));
  };
  const busy = s.creating || continuing;
  // One confirmation for both Start overs: with the words shown, and from the reload notice.
  const startOverDialog = s.startOverPending && (
    <div role="alertdialog" aria-label={model.startOverTitle} className={embedded ? "confirm setup-step stack-sm" : "card confirm stack-sm"}>
      <span className="row-title">{model.startOverTitle}</span>
      <span className="muted small">{model.startOverBody}</span>
      <div className="toolbar">
        <button className="mode setup-destructive" onClick={() => void model.confirmStartOver()}>
          {t("Start over")}
        </button>
        <button className="mode mode--active" onClick={() => model.cancelStartOver()}>
          {t("Keep this one")}
        </button>
      </div>
    </div>
  );

  return (
    <div className={embedded ? "setup setup-embedded stack" : "setup stack"}>
      {notice && (
        <p className="card" role="status">
          {notice}
        </p>
      )}
      <div>
        <Title className={embedded ? "setup-title" : "display"}>{t("Create your recovery key")}</Title>
        <p>{words.intro}</p>
      </div>

      <section className={stepClass} aria-labelledby="setup-step-passphrase">
        <Step id="setup-step-passphrase">{t("1. Choose a passphrase")}</Step>
        <p className="muted small">{t("At least 12 characters. A short sentence you will remember works well. The passphrase locks the recovery file and nothing else.")}</p>
        <label className="field">
          {t("Passphrase")}
          <input
            className="input"
            type="password"
            autoComplete="new-password"
            value={s.passphrase}
            disabled={model.locked}
            onChange={(e) => model.setPassphrase(e.target.value)}
          />
        </label>
        <label className="field">
          {t("Type it again")}
          <input
            className="input"
            type="password"
            autoComplete="new-password"
            value={s.confirmation}
            disabled={model.locked}
            onChange={(e) => model.setConfirmation(e.target.value)}
          />
        </label>
        {!model.locked && (
          <p className={model.meetsMinimum ? "small setup-ok" : "small muted"}>
            {t("{count} of at least {minimum} characters", { count: model.count, minimum: MINIMUM_LENGTH })}
          </p>
        )}
        {model.problem && <p className="small setup-error">{model.problem}</p>}
        {s.createError && <p className="small setup-error">{s.createError}</p>}
        {s.alreadyMade && (
          <div className="stack-sm">
            {model.alreadyMadeLines.map((line) => (
              <p key={line} className="small setup-error">
                {line}
              </p>
            ))}
            <div className="toolbar">
              <button className="mode setup-destructive" disabled={busy || !model.meetsMinimum || !model.confirmationMatches} onClick={() => model.requestStartOver()}>
                {t("Start over…")}
              </button>
            </div>
            {startOverDialog}
          </div>
        )}
        {model.locked ? (
          <>
            <p className="muted small">{t("To use a different passphrase, start over.")}</p>
            <div className="toolbar">
              <button className="mode" disabled={busy} onClick={() => model.requestStartOver()}>
                {t("Start over…")}
              </button>
            </div>
            {startOverDialog}
          </>
        ) : (
          <div className="toolbar">
            <button className="mode mode--active" disabled={!model.canCreate} onClick={() => void model.create()}>
              {s.creating ? t("Creating the recovery key…") : t("Create recovery key")}
            </button>
          </div>
        )}
      </section>

      {s.words && (
        <>
          <section className={stepClass} aria-labelledby="setup-step-file">
            <Step id="setup-step-file">{t("2. Save the recovery file")}</Step>
            <p className="muted small">
              {t(
                "Save Whimbrel recovery key.txt somewhere other than this PC, such as a USB drive, or attached to an entry in your password manager. It is locked with your passphrase. If you forget the passphrase, the file cannot be opened.",
              )}
            </p>
            <div className="toolbar">
              <button className="mode mode--active setup-primary" disabled={s.saving} onClick={() => void model.saveFile()}>
                {t("Save recovery file…")}
              </button>
            </div>
            {s.savedPath && <p className="small setup-ok">{t("Saved to {path}", { path: s.savedPath })}</p>}
            {s.savedPath && s.savedOnSameDisk && (
              <p className="small setup-warning">{words.sameDisk}</p>
            )}
            {s.saveFailed && <p className="small setup-error">{t("The file could not be saved there. Try another folder.")}</p>}
          </section>

          <section className={stepClass} aria-labelledby="setup-step-words">
            <Step id="setup-step-words">{t("3. Write down your 24 recovery words")}</Step>
            <p className="setup-warning setup-strong">{words.wordsWarning}</p>
            <div className="recovery-words-row">
              <WordsGrid words={s.words} />
              <WordsQr words={s.words} px={150} />
            </div>
            <div className="toolbar">
              <button className="mode" onClick={() => model.print()}>
                {t("Print words and QR code…")}
              </button>
            </div>
            <label className="toggle">
              <input type="checkbox" checked={s.wroteDown} onChange={(e) => model.setWroteDown(e.target.checked)} />
              {t("I wrote down all 24 words, in order")}
            </label>
            <PrintPortal words={s.words} copy={printCopy} />
          </section>

          <section className={stepClass} aria-labelledby="setup-step-test">
            <Step id="setup-step-test">{t("4. Test your passphrase")}</Step>
            <p className="muted small">
              {t("Type your passphrase once more. Whimbrel checks it against the recovery file it just made, so you know your passphrase opens the recovery file.")}
            </p>
            <form
              className="toolbar"
              onSubmit={(e) => {
                e.preventDefault();
                void model.test();
              }}
            >
              <input
                className="input grow"
                type="password"
                autoComplete="off"
                aria-label={t("Passphrase")}
                value={s.testPassphrase}
                onChange={(e) => model.setTestPassphrase(e.target.value)}
              />
              <button className="mode" type="submit" disabled={!model.canTest}>
                {s.testing ? t("Checking…") : t("Test")}
              </button>
            </form>
            {s.testPassed && <p className="small setup-ok">{t("That passphrase opens your recovery file.")}</p>}
            {s.testFailed && <p className="small setup-error">{t("That passphrase does not open the recovery file. Check it and try again.")}</p>}
          </section>

          <section className="stack-sm">
            <label className="toggle">
              <input
                type="checkbox"
                checked={s.savedConfirmed}
                disabled={!model.canConfirmSaved}
                onChange={(e) => model.setSavedConfirmed(e.target.checked)}
              />
              {t("I have kept my recovery file or my 24 words somewhere other than this PC")}
            </label>
            {!model.canConfirmSaved && (
              <p className="muted small">{t("First pass the test above, and save the recovery file or tick that you wrote the words down.")}</p>
            )}
          </section>
        </>
      )}

      <p className="muted small">{words.footer}</p>

      <div className="toolbar">
        <button className="mode mode--active" disabled={!model.canContinue || continuing} onClick={onContinue}>
          {continueTitle}
        </button>
        {onCancel && (
          <button className="mode" disabled={continuing} onClick={onCancel}>
            {t("Cancel")}
          </button>
        )}
        {onQuit && (
          <button className="mode" disabled={continuing} onClick={quit}>
            {t("Quit Whimbrel")}
          </button>
        )}
      </div>
      {askingQuit && onQuit && (
        <div role="alertdialog" aria-label={t("Your recovery file or printout will not work if you quit now. Quit anyway?")} className="card confirm stack-sm">
          <span className="row-title">{t("Your recovery file or printout will not work if you quit now. Quit anyway?")}</span>
          <span className="muted small">{t("The next time Whimbrel starts, it makes a new recovery key with new words.")}</span>
          <div className="toolbar">
            <button className="mode setup-destructive" onClick={onQuit}>
              {t("Quit Whimbrel")}
            </button>
            <button className="mode mode--active" onClick={() => setAskingQuit(false)}>
              {t("Keep setting up")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
