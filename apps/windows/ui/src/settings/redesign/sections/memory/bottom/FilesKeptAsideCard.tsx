import { useEffect, useState } from "react";
import { foreignFilesDelete, foreignFilesList } from "../../../../../api";
import { describeError, isCommandError } from "../../../../../errors";
import type { ForeignFilesView } from "../../../../../generated/types";
import { useL10n } from "../../../../../l10n";
import { WConfirmDialog, WNoticeBox, WSettingsCard, WSettingsRow } from "../../../../../system";
import { resultLines, showCard } from "../../../../foreignFilesModel";
import { keptAsideConfirmBody, keptAsideExplainer, keptAsideSummary } from "./foreignFilesWording";
import "./bottom.css";

/**
 * Files kept aside: shown only while the encryption work has set files aside beside the library. Deleted only after
 * the person confirms, never automatically. The shell deletes only while it still finds the count the confirmation named.
 */
export function FilesKeptAsideCard() {
  const { t } = useL10n();
  const [view, setView] = useState<ForeignFilesView | null>(null);
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string[]>([]);
  const [problems, setProblems] = useState<string[]>([]);

  const refresh = () =>
    foreignFilesList().then(
      (v) => setView(v),
      () => setView(null),
    );
  useEffect(() => {
    void refresh();
  }, []);

  const confirm = async (confirmed: number) => {
    setBusy(true);
    try {
      const result = resultLines(await foreignFilesDelete(confirmed));
      setDone(result.done);
      setProblems(result.problems);
    } catch (e: unknown) {
      setDone([]);
      setProblems([isCommandError(e) ? describeError(e) : String(e)]);
    } finally {
      setBusy(false);
      setAsking(false);
      await refresh();
    }
  };

  if (!showCard(view) && done.length === 0 && problems.length === 0) return null;
  return (
    <WSettingsCard id="capture.foreignFiles" title={t("Files kept aside")} headingLevel={3} description={keptAsideExplainer()}>
      {view && showCard(view) && (
        <>
          <WSettingsRow variant="destructive" label={keptAsideSummary(view)} actionLabel={t("Delete these files")} onPress={() => setAsking(true)} />
          {asking && (
            <WConfirmDialog
              title={t("Delete these files?")}
              body={keptAsideConfirmBody(view)}
              confirmLabel={t("Delete these files")}
              busy={busy}
              onCancel={() => setAsking(false)}
              onConfirm={() => void confirm(view.count)}
            />
          )}
        </>
      )}
      {done.map((l) => (
        <div className="rs-membottom-block" key={l}>
          <WNoticeBox tone="ok" live>
            {l}
          </WNoticeBox>
        </div>
      ))}
      {problems.map((l) => (
        <div className="rs-membottom-block" key={l}>
          <WNoticeBox tone="error">{l}</WNoticeBox>
        </div>
      ))}
    </WSettingsCard>
  );
}
