// The recovery screen (brief 3.3.4, port of the Mac's EncryptionRecoveryView): the key is gone or
// does not open the database. Its only actions restore a key, and Quit.

import { useEffect, useSyncExternalStore } from "react";
import type { RecoveryReasonView } from "../generated/types";
import { t } from "../l10n";
import { recoveryExplanation, type RecoveryModel } from "./recoveryModel";

export function RecoveryView({ model, reason, onQuit }: { model: RecoveryModel; reason: RecoveryReasonView; onQuit: () => void }) {
  const s = useSyncExternalStore(model.subscribe, model.getState, model.getState);
  useEffect(() => () => model.clear(), [model]);

  return (
    <div className="launch-screen stack">
      <div>
        <h1 className="display">{t("Whimbrel can't open your memory database")}</h1>
        <p>{recoveryExplanation(reason)}</p>
        <p>{t("Your data has not been changed. Use your recovery key to open it.")}</p>
      </div>

      <div className="card stack-sm">
        <div className="mode-picker" role="radiogroup">
          <button role="radio" aria-checked={s.method === "file"} className={s.method === "file" ? "mode mode--active" : "mode"} onClick={() => model.setMethod("file")}>
            {t("Recovery file")}
          </button>
          <button role="radio" aria-checked={s.method === "words"} className={s.method === "words" ? "mode mode--active" : "mode"} onClick={() => model.setMethod("words")}>
            {t("24 words")}
          </button>
        </div>

        {s.method === "file" ? (
          <form
            className="stack-sm"
            onSubmit={(e) => {
              e.preventDefault();
              void model.unlockFile();
            }}
          >
            <div className="toolbar">
              <button type="button" className="mode" disabled={s.working} onClick={() => void model.chooseFile()}>
                {t("Choose recovery file…")}
              </button>
              {s.fileName && <span className="mono">{s.fileName}</span>}
            </div>
            <label className="field">
              {t("Passphrase")}
              <input className="input" type="password" autoComplete="off" value={s.passphrase} onChange={(e) => model.setPassphrase(e.target.value)} />
            </label>
            <div className="toolbar">
              <button type="submit" className="mode mode--active" disabled={!model.canUnlockFile}>
                {s.working ? t("Checking…") : t("Unlock with file")}
              </button>
            </div>
          </form>
        ) : (
          <div className="stack-sm">
            <p className="muted small">{t("Type or paste your 24 words. Spaces, line breaks and capitals do not matter.")}</p>
            <textarea
              className="input mono recovery-words-input"
              rows={6}
              spellCheck={false}
              autoCorrect="off"
              autoCapitalize="off"
              autoComplete="off"
              aria-label={t("24 words")}
              value={s.words}
              onChange={(e) => model.setWords(e.target.value)}
            />
            <div className="toolbar">
              <button className="mode mode--active" disabled={!model.canUnlockWords} onClick={() => void model.unlockWords()}>
                {s.working ? t("Checking…") : t("Unlock with words")}
              </button>
            </div>
          </div>
        )}
        {s.error && <p className="small setup-error">{s.error}</p>}
        {s.restored && <p className="small setup-ok">{t("Unlocked. Whimbrel is opening your memory database.")}</p>}
      </div>

      <p className="muted small">{t("Whimbrel will not delete or replace this database. It stays where it is, unchanged, until you unlock it.")}</p>
      <p className="muted small">
        {t(
          "You only need the recovery key if this PC loses its own key. If that happens and you have neither the 24 words nor the recovery file and its passphrase, nobody can unlock the memory database, including Whimbrel. If you use sync, your other devices keep their own copies of what they synced, but not the key.",
        )}
      </p>
      <div className="toolbar">
        <button className="mode" disabled={s.working} onClick={onQuit}>
          {t("Quit Whimbrel")}
        </button>
      </div>
    </div>
  );
}
