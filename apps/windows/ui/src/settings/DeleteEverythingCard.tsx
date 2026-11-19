import { useReducer, useState } from "react";
import { deleteEverythingNow, deleteEverythingPreview } from "../api";
import { describeError, isCommandError } from "../errors";
import type { DeleteEverythingPreview } from "../generated/types";
import { useL10n } from "../l10n";
import { canDelete, closed, CONFIRM_WORD, confirmArgs, confirmLines, dialogReducer, syncAccountLine, type DialogState } from "./deleteEverythingModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** The confirmation (the Mac's Delete everything sheet): what goes and what stays, the sync-account choice, then DELETE typed to confirm. */
export function DeleteEverythingDialog({
  preview,
  state,
  onTyped,
  onSyncAccount,
  onCancel,
  onConfirm,
}: {
  preview: DeleteEverythingPreview | null;
  state: DialogState;
  onTyped: (text: string) => void;
  onSyncAccount: (on: boolean) => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useL10n();
  return (
    <div role="alertdialog" aria-labelledby="delete-everything-title" className="field">
      <p id="delete-everything-title" className="row-title">
        {t("Delete everything on this PC?")}
      </p>
      {preview === null ? (
        !state.error && <p className="muted small">{t("Loading…")}</p>
      ) : (
        confirmLines(preview, state.alsoDeleteSyncAccount).map((line) => <p key={line}>{line}</p>)
      )}
      {preview?.syncOn && (
        <>
          <label className="row">
            <input
              type="checkbox"
              checked={state.alsoDeleteSyncAccount}
              disabled={state.busy}
              onChange={(e) => onSyncAccount(e.target.checked)}
            />{" "}
            {t("Also delete my sync account")}
          </label>
          <p className={state.alsoDeleteSyncAccount ? "muted small" : "small"} role={state.alsoDeleteSyncAccount ? undefined : "note"}>
            {syncAccountLine(state.alsoDeleteSyncAccount)}
          </p>
        </>
      )}
      <label htmlFor="delete-everything-typed" className="muted small">
        {t("Type DELETE to confirm.")}
      </label>
      <input
        id="delete-everything-typed"
        className="input"
        value={state.typed}
        placeholder={CONFIRM_WORD}
        disabled={state.busy}
        autoComplete="off"
        spellCheck={false}
        onChange={(e) => onTyped(e.target.value)}
      />
      {state.error && <p role="alert">{state.error}</p>}
      <div className="toolbar">
        <button className="mode" disabled={state.busy} onClick={onCancel}>
          {t("Cancel")}
        </button>
        <button className="mode mode--active" disabled={!canDelete(state, preview)} onClick={onConfirm}>
          {state.busy ? t("Deleting…") : t("Delete and restart")}
        </button>
      </div>
    </div>
  );
}

/**
 * Settings → Capture → Delete everything (X4 Task 12). The slide images go at once; the data folder
 * goes at the restart, before anything opens the database, and the logs with it. Downloaded models, settings.json and API keys stay.
 */
export function DeleteEverythingCard() {
  const { t } = useL10n();
  const [state, dispatch] = useReducer(dialogReducer, closed);
  const [preview, setPreview] = useState<DeleteEverythingPreview | null>(null);

  const open = () => {
    dispatch({ kind: "open" });
    setPreview(null);
    deleteEverythingPreview().then(setPreview, (e: unknown) => dispatch({ kind: "failed", error: message(e) }));
  };
  const confirm = async () => {
    if (!preview) return;
    dispatch({ kind: "busy" });
    try {
      // On success the app restarts inside this call and never returns here.
      await deleteEverythingNow(confirmArgs(state, preview));
    } catch (e: unknown) {
      dispatch({ kind: "failed", error: message(e) });
    }
  };

  return (
    <section className="card about" aria-labelledby="delete-everything-heading">
      <h2 id="delete-everything-heading" className="section-label">
        {t("Delete everything")}
      </h2>
      {state.open ? (
        <DeleteEverythingDialog
          preview={preview}
          state={state}
          onTyped={(text) => dispatch({ kind: "typed", text })}
          onSyncAccount={(on) => dispatch({ kind: "syncAccount", on })}
          onCancel={() => dispatch({ kind: "cancel" })}
          onConfirm={() => void confirm()}
        />
      ) : (
        <>
          <p className="muted small">{t("Removes your meetings, memory, recordings and logs from this PC. Your settings (settings.json), downloaded models and API keys stay.")}</p>
          <button className="mode" onClick={open}>
            {t("Delete everything on this PC…")}
          </button>
        </>
      )}
    </section>
  );
}
