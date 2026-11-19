// The recovery screen in the Thread look: the key is gone or does not open the database. Its only actions restore
// a key, and Quit. The same words and calls as ../RecoveryView.tsx; the rules are in ../recoveryModel.ts.

import { useEffect, useSyncExternalStore } from "react";
import type { RecoveryReasonView } from "../../generated/types";
import { t } from "../../l10n";
import { WButton, WFlowPage, WNotice, WPanel, WSegmented, WTextField } from "../../system";
import { recoveryExplanation, type RecoveryMethod, type RecoveryModel } from "../recoveryModel";

export function RecoveryScreen({ model, reason, onQuit }: { model: RecoveryModel; reason: RecoveryReasonView; onQuit: () => void }) {
  const s = useSyncExternalStore(model.subscribe, model.getState, model.getState);
  useEffect(() => () => model.clear(), [model]);
  const file = s.method === "file";
  const can = file ? model.canUnlockFile : model.canUnlockWords;
  const unlock = () => void (file ? model.unlockFile() : model.unlockWords());
  const unlockLabel = file ? t("Unlock with file") : t("Unlock with words");

  return (
    <WFlowPage
      title={t("Whimbrel can't open your memory database")}
      intro={
        <>
          <p>{recoveryExplanation(reason)}</p>
          <p>{t("Your data has not been changed. Use your recovery key to open it.")}</p>
        </>
      }
      busyLabel={s.working ? t("Checking…") : undefined}
      actions={
        <>
          <WButton disabled={s.working} onClick={onQuit}>
            {t("Quit Whimbrel")}
          </WButton>
          <WButton variant="primary" disabled={!can} onClick={unlock}>
            {s.working ? t("Checking…") : unlockLabel}
          </WButton>
        </>
      }
      finePrint={
        <>
          <p>{t("Whimbrel will not delete or replace this database. It stays where it is, unchanged, until you unlock it.")}</p>
          <p>
            {t(
              "You only need the recovery key if this PC loses its own key. If that happens and you have neither the 24 words nor the recovery file and its passphrase, nobody can unlock the memory database, including Whimbrel. If you use sync, your other devices keep their own copies of what they synced, but not the key.",
            )}
          </p>
        </>
      }
      onPrimary={can ? unlock : undefined}
    >
      <WPanel>
        <WSegmented<RecoveryMethod>
          label={t("Recovery key type")}
          value={s.method}
          options={[
            { value: "file", label: t("Recovery file") },
            { value: "words", label: t("24 words") },
          ]}
          onChange={(m) => model.setMethod(m)}
        />
        {file ? (
          <>
            <div className="tl-file-row">
              <WButton disabled={s.working} onClick={() => void model.chooseFile()}>
                {t("Choose recovery file…")}
              </WButton>
              {s.fileName ? <span className="tl-file-name">{s.fileName}</span> : null}
            </div>
            <WTextField label={t("Passphrase")} type="password" value={s.passphrase} onChange={(v) => model.setPassphrase(v)} error={s.error} />
          </>
        ) : (
          <WTextField
            label={t("24 words")}
            help={t("Type or paste your 24 words. Spaces, line breaks and capitals do not matter.")}
            rows={6}
            value={s.words}
            onChange={(v) => model.setWords(v)}
            error={s.error}
          />
        )}
        {s.restored ? <WNotice tone="ok">{t("Unlocked. Whimbrel is opening your memory database.")}</WNotice> : null}
      </WPanel>
    </WFlowPage>
  );
}
