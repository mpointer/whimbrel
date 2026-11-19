// The recovery setup in the Thread look: one step on each page, with "Step 2 of 4". The steps are the four the old
// screen shows together (SetupView.tsx), in the same order, and everything they do is still the model's: the
// passphrase rules, the words, the file, the test and the two ticks. Going between the steps is only what is on
// screen; nothing is gated on it except that the words must exist before step 2. The props are the old screen's.

import { useEffect, useState, useSyncExternalStore } from "react";
import { t } from "../../l10n";
import { WButton, WCheckbox, WDialog, WFlowPage, WNotice, WNumberedList, WPanel, WTextField } from "../../system";
import { PrintPortal, WordsQr } from "../RecoveryWords";
import { MINIMUM_LENGTH } from "../setupModel";
import type { SetupViewProps } from "../SetupView";

const STEPS = 4;

export interface SetupScreenProps extends SetupViewProps {
  /** Tests and the gallery open on a later step; it only counts once the words exist. */
  initialStep?: number;
}

export function SetupScreen({ model, continueTitle, onContinue, continuing = false, onQuit, quitSignal = 0, onCancel, notice = null, initialStep = 1 }: SetupScreenProps) {
  const s = useSyncExternalStore(model.subscribe, model.getState, model.getState);
  const [askingQuit, setAskingQuit] = useState(false);
  const [wanted, setWanted] = useState(initialStep);
  const hasWords = s.words !== null;
  // The words appearing moves to step 2; losing them (Start over) moves back to step 1. Decided while rendering,
  // so there is no frame in between.
  const [hadWords, setHadWords] = useState(hasWords);
  if (hasWords !== hadWords) {
    setHadWords(hasWords);
    setWanted(hasWords ? 2 : 1);
  }
  const step = hasWords ? wanted : 1;

  // Leaving the screen forgets what it showed and what was typed.
  useEffect(() => () => model.clear(), [model]);
  useEffect(() => {
    if (quitSignal > 0) setAskingQuit(true);
  }, [quitSignal]);

  const quit = () => {
    if (!onQuit) return;
    // Ask the shell too, as closing the window does: after a reload it still holds words this screen no longer shows.
    void model.quitAsksFirst().then((ask) => (ask ? setAskingQuit(true) : onQuit()));
  };
  const busy = s.creating || continuing;
  const problem = model.problem;
  const differs = problem === t("The two passphrases do not match.");

  const next = () => setWanted(Math.min(STEPS, step + 1));
  const back = () => setWanted(Math.max(1, step - 1));

  // One confirmation for both Start overs: with the words shown, and from the reload notice.
  const startOverDialog = s.startOverPending ? (
    <WDialog title={model.startOverTitle} body={model.startOverBody} onCancel={() => model.cancelStartOver()}>
      <WButton variant="destructive" onClick={() => void model.confirmStartOver()}>
        {t("Start over")}
      </WButton>
      <WButton variant="primary" data-initial="" onClick={() => model.cancelStartOver()}>
        {t("Keep this one")}
      </WButton>
    </WDialog>
  ) : null;
  const quitDialog =
    askingQuit && onQuit ? (
      <WDialog
        title={t("Your recovery file or printout will not work if you quit now. Quit anyway?")}
        body={t("The next time Whimbrel starts, it makes a new recovery key with new words.")}
        onCancel={() => setAskingQuit(false)}
      >
        <WButton variant="destructive" onClick={onQuit}>
          {t("Quit Whimbrel")}
        </WButton>
        <WButton variant="primary" data-initial="" onClick={() => setAskingQuit(false)}>
          {t("Keep setting up")}
        </WButton>
      </WDialog>
    ) : null;

  const leaving = (
    <>
      {onCancel ? (
        <WButton disabled={continuing} onClick={onCancel}>
          {t("Cancel")}
        </WButton>
      ) : null}
      {onQuit ? (
        <WButton disabled={continuing} onClick={quit}>
          {t("Quit Whimbrel")}
        </WButton>
      ) : null}
    </>
  );
  const backButton = (
    <WButton disabled={continuing} onClick={back}>
      {t("Back")}
    </WButton>
  );
  const common = {
    step: { current: step, total: STEPS },
    lead: notice && step === 1 ? <WNotice>{notice}</WNotice> : undefined,
    finePrint: (
      <p>
        {t(
          "You only need the recovery key if this PC loses its own key. If that happens and you have neither the 24 words nor the recovery file and its passphrase, nobody can unlock the memory database, including Whimbrel. If you use sync, your other devices keep their own copies of what they synced, but not the key.",
        )}
      </p>
    ),
  };
  const goBackOrCancel = step > 1 ? back : onCancel && !continuing ? onCancel : undefined;
  // The printed page stays mounted for as long as the words exist, whichever step is showing.
  const dialogs = (
    <>
      {s.words ? <PrintPortal words={s.words} /> : null}
      {startOverDialog}
      {quitDialog}
    </>
  );

  if (step === 1) {
    return (
      <WFlowPage
        {...common}
        title={t("Create your recovery key")}
        intro={
          <p>
            {t(
              "Whimbrel will encrypt the memory database on this PC with a key kept in Windows Credential Manager. If that key is ever lost, your recovery key is the only way back into the memory database. Make it now, before anything is encrypted.",
            )}
          </p>
        }
        busyLabel={s.creating ? t("Creating the recovery key…") : undefined}
        actions={
          <>
            {leaving}
            {hasWords ? (
              <WButton variant="primary" onClick={next}>
                {t("Next step")}
              </WButton>
            ) : (
              <WButton variant="primary" disabled={!model.canCreate} onClick={() => void model.create()}>
                {s.creating ? t("Creating the recovery key…") : t("Create recovery key")}
              </WButton>
            )}
          </>
        }
        onPrimary={hasWords ? next : model.canCreate ? () => void model.create() : undefined}
        onBack={goBackOrCancel}
      >
        <WPanel>
          <WTextField
            label={t("Passphrase")}
            help={t("At least 12 characters. A short sentence you will remember works well. The passphrase locks the recovery file and nothing else.")}
            type="password"
            autoComplete="new-password"
            value={s.passphrase}
            disabled={model.locked}
            onChange={(v) => model.setPassphrase(v)}
            status={model.locked ? undefined : { text: t("{count} of at least {minimum} characters", { count: model.count, minimum: MINIMUM_LENGTH }), tone: model.meetsMinimum ? "ok" : "muted" }}
            error={differs ? null : problem}
          />
          <WTextField
            label={t("Type it again")}
            type="password"
            autoComplete="new-password"
            value={s.confirmation}
            disabled={model.locked}
            onChange={(v) => model.setConfirmation(v)}
            error={differs ? problem : null}
          />
          {s.createError ? <WNotice tone="error">{s.createError}</WNotice> : null}
          {s.alreadyMade ? (
            <>
              {model.alreadyMadeLines.map((line) => (
                <WNotice key={line} tone="error">
                  {line}
                </WNotice>
              ))}
              <div>
                <WButton variant="destructive" disabled={busy || !model.meetsMinimum || !model.confirmationMatches} onClick={() => model.requestStartOver()}>
                  {t("Start over…")}
                </WButton>
              </div>
            </>
          ) : null}
          {model.locked ? (
            <>
              <p className="w-flow-secondary">{t("To use a different passphrase, start over.")}</p>
              <div>
                <WButton disabled={busy} onClick={() => model.requestStartOver()}>
                  {t("Start over…")}
                </WButton>
              </div>
            </>
          ) : null}
        </WPanel>
        {dialogs}
      </WFlowPage>
    );
  }

  if (step === 2) {
    return (
      <WFlowPage
        {...common}
        title={t("Save the recovery file")}
        intro={
          <p>
            {t(
              "Save Whimbrel recovery key.txt somewhere other than this PC, such as a USB drive, or attached to an entry in your password manager. It is locked with your passphrase. If you forget the passphrase, the file cannot be opened.",
            )}
          </p>
        }
        actions={
          <>
            {leaving}
            {backButton}
            <WButton variant="primary" onClick={next}>
              {t("Next step")}
            </WButton>
          </>
        }
        onPrimary={next}
        onBack={goBackOrCancel}
      >
        <WPanel>
          <div>
            <WButton disabled={s.saving} onClick={() => void model.saveFile()}>
              {t("Save recovery file…")}
            </WButton>
          </div>
          {s.savedPath ? <WNotice tone="ok">{t("Saved to {path}", { path: s.savedPath })}</WNotice> : null}
          {s.savedPath && s.savedOnSameDisk ? (
            <WNotice tone="warning">{t("This file is on the same disk as your memory database, so it may be lost with it. Keep a copy somewhere other than this PC as well.")}</WNotice>
          ) : null}
          {s.saveFailed ? <WNotice tone="error">{t("The file could not be saved there. Try another folder.")}</WNotice> : null}
        </WPanel>
        {dialogs}
      </WFlowPage>
    );
  }

  if (step === 3) {
    return (
      <WFlowPage
        {...common}
        title={t("Write down your 24 recovery words")}
        actions={
          <>
            {leaving}
            {backButton}
            <WButton variant="primary" onClick={next}>
              {t("Next step")}
            </WButton>
          </>
        }
        onPrimary={next}
        onBack={goBackOrCancel}
      >
        <WNotice tone="warning" strong>
          {t(
            "These words are the key itself. Your passphrase does not protect them. Anyone who has these words or this QR code can read the memory database. Write them down or print them, and keep them somewhere safe and offline.",
          )}
        </WNotice>
        <WPanel>
          {s.words ? (
            <>
              <WNumberedList items={s.words} label={t("Your 24 recovery words")} protectedFromCopy />
              <div className="tl-qr-row">
                <WordsQr words={s.words} px={150} />
                <WButton onClick={() => model.print()}>{t("Print words and QR code…")}</WButton>
              </div>
            </>
          ) : null}
          <WCheckbox label={t("I wrote down all 24 words, in order")} checked={s.wroteDown} onChange={(v) => model.setWroteDown(v)} />
        </WPanel>
        {dialogs}
      </WFlowPage>
    );
  }

  return (
    <WFlowPage
      {...common}
      title={t("Test your passphrase")}
      intro={<p>{t("Type your passphrase once more. Whimbrel checks it against the recovery file it just made, so you know your passphrase opens the recovery file.")}</p>}
      actions={
        <>
          {leaving}
          {backButton}
          <WButton variant="primary" disabled={!model.canContinue || continuing} onClick={onContinue}>
            {continueTitle}
          </WButton>
        </>
      }
      onPrimary={model.canContinue && !continuing ? onContinue : undefined}
      onBack={goBackOrCancel}
    >
      <WPanel>
        <WTextField
          label={t("Passphrase")}
          type="password"
          value={s.testPassphrase}
          onChange={(v) => model.setTestPassphrase(v)}
          onEnter={() => {
            if (model.canTest) void model.test();
          }}
          error={s.testFailed ? t("That passphrase does not open the recovery file. Check it and try again.") : null}
        />
        <div>
          <WButton disabled={!model.canTest} onClick={() => void model.test()}>
            {s.testing ? t("Checking…") : t("Test")}
          </WButton>
        </div>
        {s.testPassed ? <WNotice tone="ok">{t("That passphrase opens your recovery file.")}</WNotice> : null}
        <WCheckbox
          label={t("I have kept my recovery file or my 24 words somewhere other than this PC")}
          checked={s.savedConfirmed}
          disabled={!model.canConfirmSaved}
          onChange={(v) => model.setSavedConfirmed(v)}
        />
        {!model.canConfirmSaved ? <p className="w-flow-secondary">{t("First pass the test above, and save the recovery file or tick that you wrote the words down.")}</p> : null}
      </WPanel>
      {dialogs}
    </WFlowPage>
  );
}
