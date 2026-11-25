// Asking before something is lost: a dialog (with an optional typed word) and a two-step confirm inside a row.

import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { t } from "../l10n";
import { WButton } from "./controls";
import { WDialog } from "./flow";
import { WIcon } from "./WIcon";
import { typedConfirmMatches } from "./settingsLogic";
import "./system.css";
import "./settings.css";

export interface WConfirmDialogProps {
  title: string;
  body?: string;
  /** The words on the button that goes ahead: "Delete everything". */
  confirmLabel: string;
  cancelLabel?: string;
  /** Called with what was typed in the box (empty when there is none), exactly as typed, so a command that checks the word itself sees what the person wrote. */
  onConfirm: (typed: string) => void;
  onCancel: () => void;
  /** Draws the confirm button as destructive (the default). Either way it is never the one focus starts on. */
  destructive?: boolean;
  /** The person must type this word before the confirm button turns on. Focus starts in the box. */
  typed?: { word: string; label: string; hint?: string };
  /** Fields and notes between the body and the buttons (a checkbox, a select). They take part in the Tab trap. */
  fields?: ReactNode;
  /** The confirm button stays off for another reason (a required checkbox is not ticked). */
  confirmDisabled?: boolean;
  /** Said in words under the fields while the confirm button is off for `confirmDisabled`, and read with the button. */
  confirmDisabledReason?: string;
  /** Working: both buttons are off. */
  busy?: boolean;
  /** What went wrong, under the fields. */
  error?: string | null;
}

/** A question that must be answered, built on WDialog. Focus starts on Cancel (or in the typed box), Return in a box
 *  does nothing, Escape cancels, and the confirm button is off until what is typed matches. */
export function WConfirmDialog({ title, body, confirmLabel, cancelLabel, onConfirm, onCancel, destructive = true, typed, fields, confirmDisabled, confirmDisabledReason, busy, error }: WConfirmDialogProps) {
  const id = useId();
  const [text, setText] = useState("");
  const matches = !typed || typedConfirmMatches(text, typed.word);
  const ready = matches && !confirmDisabled && !busy;
  return (
    <WDialog
      title={title}
      body={body}
      onCancel={() => {
        if (!busy) onCancel();
      }}
      details={
        <>
          {fields}
          {typed ? (
            <div className="w-field w-confirm-typed">
              <label className="w-field-label" id={`${id}-typed-label`} htmlFor={`${id}-typed`}>
                {typed.label}
              </label>
              {typed.hint ? (
                <p className="w-field-help" id={`${id}-hint`}>
                  {typed.hint}
                </p>
              ) : null}
              <input
                id={`${id}-typed`}
                className="w-input"
                type="text"
                autoComplete="off"
                spellCheck={false}
                data-initial
                value={text}
                // Read-only while working, never disabled: a disabled box would take focus out of the dialog.
                readOnly={busy}
                aria-disabled={busy ? true : undefined}
                aria-describedby={typed.hint ? `${id}-hint` : undefined}
                onChange={(e) => {
                  if (!busy) setText(e.target.value);
                }}
              />
            </div>
          ) : null}
          {confirmDisabled && confirmDisabledReason ? (
            <p className="w-setting-note" id={`${id}-why`}>
              {confirmDisabledReason}
            </p>
          ) : null}
          <div aria-live="polite">
            {error ? (
              <p className="w-field-error">
                <WIcon name="warning" size={16} />
                <span>{error}</span>
              </p>
            ) : null}
          </div>
        </>
      }
    >
      {/* While working the buttons are aria-disabled and ignore a press instead of being disabled: a disabled button
          that has focus drops it out of the dialog. */}
      <WButton data-initial={typed ? undefined : true} onClick={() => !busy && onCancel()} aria-disabled={busy ? true : undefined}>
        {cancelLabel ?? t("Cancel")}
      </WButton>
      <WButton
        variant={destructive ? "destructive" : "primary"}
        disabled={!busy && !ready}
        aria-disabled={busy ? true : undefined}
        aria-describedby={!matches && typed ? `${id}-typed-label` : confirmDisabled && confirmDisabledReason ? `${id}-why` : undefined}
        onClick={() => ready && onConfirm(text)}
      >
        {confirmLabel}
      </WButton>
    </WDialog>
  );
}


export interface WInlineConfirmProps {
  /** "Delete this skin?" It also names the group. */
  question: string;
  /** The destructive button's words. */
  confirmLabel: string;
  cancelLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
  /** The element id to give focus back to after Cancel or Escape (the button this replaced). */
  returnFocusTo?: string;
  busy?: boolean;
}

/** The two-step confirm that takes the place of a row's button: the question, the destructive button, and Cancel.
 *  Focus starts on Cancel, so Return or Space does not delete; Escape cancels. */
export function WInlineConfirm({ question, confirmLabel, cancelLabel, onConfirm, onCancel, returnFocusTo, busy }: WInlineConfirmProps) {
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    box.current?.querySelector<HTMLElement>("[data-cancel]")?.focus({ preventScroll: true });
  }, []);
  const back = () => {
    onCancel();
    if (returnFocusTo) setTimeout(() => document.getElementById(returnFocusTo)?.focus(), 0);
  };
  return (
    <div
      className="w-iconfirm"
      ref={box}
      role="group"
      aria-label={question}
      onKeyDown={(e: KeyboardEvent<HTMLElement>) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          back();
        }
      }}
    >
      <p className="w-iconfirm-q">{question}</p>
      <div className="w-iconfirm-actions">
        <WButton variant="destructive" onClick={() => !busy && onConfirm()} aria-disabled={busy ? true : undefined}>
          {confirmLabel}
        </WButton>
        <WButton data-cancel="" onClick={() => !busy && back()} aria-disabled={busy ? true : undefined}>
          {cancelLabel ?? t("Cancel")}
        </WButton>
      </div>
    </div>
  );
}
