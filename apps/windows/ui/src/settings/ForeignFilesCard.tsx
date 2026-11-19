import { useEffect, useState } from "react";
import { foreignFilesDelete, foreignFilesList } from "../api";
import { describeError, isCommandError } from "../errors";
import type { ForeignFilesView } from "../generated/types";
import { t } from "../l10n";
import { confirmLine, resultLines, showCard, summaryLine } from "./foreignFilesModel";

/**
 * Settings → Capture → Files kept aside (X4 Task 13c item 3; the Mac's Your data row). Shown only
 * while the encryption work has set files aside beside the memory database; deleted only after
 * the user confirms, never automatically.
 */
export function ForeignFilesCard() {
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
      // The shell deletes only while it still finds the count this confirmation named.
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
    <section className="card" aria-labelledby="foreign-files-heading">
      <h2 id="foreign-files-heading" className="section-label">
        {t("Files kept aside")}
      </h2>
      {view && showCard(view) && (
        <>
          <p>{summaryLine(view)}</p>
          <p className="muted small">{t("Files Whimbrel set aside beside the memory database while undoing or repairing encryption. Whimbrel does not use them.")}</p>
          {asking ? (
            <div role="alertdialog" aria-labelledby="foreign-files-confirm" className="field">
              <p id="foreign-files-confirm" className="row-title">
                {t("Delete these files?")}
              </p>
              <p>{confirmLine(view)}</p>
              <div className="toolbar">
                <button className="mode" disabled={busy} onClick={() => setAsking(false)}>
                  {t("Cancel")}
                </button>
                <button className="mode mode--active" disabled={busy} onClick={() => void confirm(view.count)}>
                  {t("Delete these files")}
                </button>
              </div>
            </div>
          ) : (
            <button className="mode" onClick={() => setAsking(true)}>
              {t("Delete these files")}
            </button>
          )}
        </>
      )}
      {done.map((l) => (
        <p key={l} role="status">
          {l}
        </p>
      ))}
      {problems.map((l) => (
        <p key={l} role="alert">
          {l}
        </p>
      ))}
    </section>
  );
}
