// Delete everything, drawn with the confirm dialog: the preview, the lines, "Also delete my sync account" when sync is
// on, the typed word, then the same command as the old card. The dialog opens on the typed box, never on the
// destructive button, and Return in the box runs nothing.

import { useReducer, useState } from "react";
import { deleteEverythingNow, deleteEverythingPreview } from "../../../../../api";
import { describeError, isCommandError } from "../../../../../errors";
import type { DeleteEverythingPreview } from "../../../../../generated/types";
import { useL10n } from "../../../../../l10n";
import { WConfirmDialog, WSettingsRow, WSettingsToggleRow } from "../../../../../system";
import { useJump } from "../../../jump";
import { CONFIRM_WORD, canDelete, closed, confirmArgs, dialogReducer } from "../../../../deleteEverythingModel";
import { deleteIntro, redesignDeleteLines, syncAccountLine } from "./memoryTopModel";
import "./memtop.css";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** Delete everything, as the last rows of the Your data card: the row, and the dialog it opens. The anchor is on the row. */
export function DeleteEverythingRows() {
  const { t } = useL10n();
  const [state, dispatch] = useReducer(dialogReducer, closed);
  const [preview, setPreview] = useState<DeleteEverythingPreview | null>(null);

  const open = () => {
    dispatch({ kind: "open" });
    setPreview(null);
    deleteEverythingPreview().then(setPreview, (e: unknown) => dispatch({ kind: "failed", error: message(e) }));
  };
  // The word is typed inside the dialog, which turns its button on only for an exact match. What was typed is what is
  // sent, as the old card did, so the shell's own check of the word still applies.
  const confirm = async (typedText: string) => {
    const typed = { ...state, typed: typedText };
    if (!preview || !canDelete(typed, preview)) return;
    dispatch({ kind: "busy" });
    try {
      // On success the app restarts inside this call and never returns here.
      await deleteEverythingNow(confirmArgs(typed, preview));
    } catch (e: unknown) {
      dispatch({ kind: "failed", error: message(e) });
    }
  };

  const jumped = useJump("capture.delete");
  return (
    <>
      <WSettingsRow id="capture.delete" highlighted={jumped} variant="destructive" label={t("All data on this PC")} actionLabel={t("Delete everything…")} description={deleteIntro()} onPress={open} />
      {state.open ? (
        <WConfirmDialog
          title={t("Delete everything on this PC?")}
          confirmLabel={state.busy ? t("Deleting…") : t("Delete and restart")}
          typed={{ word: CONFIRM_WORD, label: t("Type DELETE to confirm.") }}
          confirmDisabled={preview === null}
          confirmDisabledReason={t("The button turns on when the list of what goes has loaded.")}
          busy={state.busy}
          error={state.error}
          onCancel={() => dispatch({ kind: "cancel" })}
          onConfirm={(typedText) => void confirm(typedText)}
          fields={
            <>
              {preview === null ? (
                !state.error && <p className="w-setting-note">{t("Loading…")}</p>
              ) : (
                redesignDeleteLines(preview, state.alsoDeleteSyncAccount).map((line) => <p key={line}>{line}</p>)
              )}
              {preview?.syncOn ? (
                <>
                  <WSettingsToggleRow
                    kind="checkbox"
                    label={t("Also delete my sync account")}
                    checked={state.alsoDeleteSyncAccount}
                    disabled={state.busy}
                    onToggle={(on) => dispatch({ kind: "syncAccount", on })}
                  />
                  <p className="w-setting-note">{syncAccountLine(state.alsoDeleteSyncAccount)}</p>
                </>
              ) : null}
            </>
          }
        />
      ) : null}
    </>
  );
}
