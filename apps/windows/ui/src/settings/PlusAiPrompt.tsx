import { useEffect, useRef, useState } from "react";
import { plusAiPrompt, plusAiPromptAnswer, plusAiPromptDisplayed } from "../api";
import { describeError, isCommandError } from "../errors";
import type { PlusAiPromptAnswer, PlusAiPromptState, PlusAiPromptView } from "../generated/types";
import { t } from "../l10n";
import { WButton, WDialog, WRoot } from "../system";
import { ROUTING_CHANGED_EVENT } from "./aiModel";

/** While the question can't show yet (signing in, a recording, another dialog): check again this often. */
export const WAIT_MS = 60_000;
/** While it shows: check this often whether it still applies (a recording starting closes it). */
export const SHOWING_MS = 5_000;
/** Not a Plus account by this session's plan read: check again this rarely (and on window focus), so a
 *  plan bought meanwhile, read by sync's own plan polls, is asked about. */
export const NOT_PLUS_MS = 5 * 60_000;

/** What the window does after a check. */
export type Next = { kind: "show" } | { kind: "again"; ms: number; onFocus?: boolean } | { kind: "stop" };

/**
 * After a check: show it (unless another dialog is up: it never stacks, so wait), check again later
 * (no plan read yet, signing in, a recording: soon; not Plus: rarely, and on window focus, so a plan
 * bought in the browser is asked about when the user comes back), or stop (answered on this PC, or
 * dismissed this session).
 */
export function nextAfter(state: PlusAiPromptState, otherDialogOpen: boolean): Next {
  switch (state) {
    case "show":
      return otherDialogOpen ? { kind: "again", ms: SHOWING_MS } : { kind: "show" };
    case "wait":
      return { kind: "again", ms: WAIT_MS };
    case "not_plus":
      return { kind: "again", ms: NOT_PLUS_MS, onFocus: true };
    case "done":
      return { kind: "stop" };
  }
}

/** Typing in a field when the question opens: focus the dialog, not its default button. */
export function wasTyping(el: { tagName: string; type?: string; isContentEditable?: boolean } | null): boolean {
  if (!el) return false;
  const tag = el.tagName.toLowerCase();
  if (tag === "textarea" || tag === "select") return true;
  if (tag === "input") return !["button", "checkbox", "radio", "submit", "reset", "range", "color", "file"].includes(el.type ?? "text");
  return el.isContentEditable === true;
}

/**
 * After an answer the shell applied: Use Whimbrel AI changed the provider, so tell an open Settings → AI
 * form to read routing again (otherwise its next save would write the old provider back).
 */
export function afterAnswer(choice: PlusAiPromptAnswer, applied: boolean, target: EventTarget) {
  if (applied && choice === "use_whimbrel") target.dispatchEvent(new Event(ROUTING_CHANGED_EVENT));
}

const otherDialogOpen = () => document.querySelector('[aria-modal="true"]') !== null;

/** What the check loop needs from the window (a test passes fakes). */
export interface CheckHost {
  check: () => Promise<PlusAiPromptView>;
  /** The question is on screen. */
  showing: boolean;
  /** An answer is saving: a re-check doesn't close the dialog under it. */
  busy: () => boolean;
  otherDialogOpen: () => boolean;
  /** A check said show: display it. */
  show: (view: PlusAiPromptView) => void;
  /** On screen, and a re-check says it no longer applies: close it. */
  close: () => void;
  setTimer: (f: () => void, ms: number) => ReturnType<typeof setTimeout>;
  clearTimer: (t: ReturnType<typeof setTimeout> | undefined) => void;
  onFocus: (f: () => void) => void;
  offFocus: (f: () => void) => void;
}

/**
 * Checks at once (or, while showing, after SHOWING_MS) and then as `nextAfter` says. One chain at a
 * time: a check cancels any pending timer and focus listener first, so focus events can't start
 * parallel chains. Returns the stop function, which leaves no timer or listener behind.
 */
export function startChecking(h: CheckHost): () => void {
  let live = true;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const look = () => {
    h.clearTimer(timer);
    timer = undefined;
    h.offFocus(look);
    h.check().then(
      (v) => {
        if (!live) return;
        if (h.showing) {
          // On screen: if it no longer applies, close it (as on the Mac, closed for this session);
          // never under an answer that is saving, whose error must still show.
          if (v.state !== "show" && !h.busy()) h.close();
          else timer = h.setTimer(look, SHOWING_MS);
          return;
        }
        const next = nextAfter(v.state, h.otherDialogOpen());
        if (next.kind === "show") h.show(v);
        else if (next.kind === "again") {
          timer = h.setTimer(look, next.ms);
          if (next.onFocus) h.onFocus(look);
        }
      },
      () => {
        if (live) timer = h.setTimer(look, WAIT_MS);
      },
    );
  };
  if (h.showing) timer = h.setTimer(look, SHOWING_MS);
  else look();
  return () => {
    live = false;
    h.clearTimer(timer);
    h.offFocus(look);
  };
}

/**
 * The one-time "Use Whimbrel AI?" question for a Plus account (the Mac's PlusAiPrompt; the shell's
 * `plus_prompt` decides). Checked at launch, then as `nextAfter` says. While it shows it is checked
 * again every few seconds and closes if it no longer applies (a recording started).
 */
export function PlusAiPrompt() {
  const [view, setView] = useState<PlusAiPromptView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const focusDialog = useRef(false);
  const busyNow = useRef(false);
  busyNow.current = busy;
  const showing = view?.state === "show";
  useEffect(
    () =>
      startChecking({
        check: plusAiPrompt,
        showing,
        busy: () => busyNow.current,
        otherDialogOpen,
        show: (v) => {
          focusDialog.current = wasTyping(document.activeElement);
          setView(v);
          void plusAiPromptDisplayed().catch(() => {});
        },
        close: () => {
          setView(null);
          void plusAiPromptAnswer("not_now").catch(() => {});
        },
        setTimer: (f, ms) => setTimeout(f, ms),
        clearTimer: (t) => clearTimeout(t),
        onFocus: (f) => window.addEventListener("focus", f),
        offFocus: (f) => window.removeEventListener("focus", f),
      }),
    [showing],
  );
  if (!view || view.state !== "show" || !view.note) return null;
  const answer = (choice: PlusAiPromptAnswer) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    plusAiPromptAnswer(choice).then(
      (applied) => {
        setBusy(false);
        setView(null);
        afterAnswer(choice, applied, window);
      },
      (e: unknown) => {
        setBusy(false);
        setError(isCommandError(e) ? describeError(e) : String(e));
      },
    );
  };
  return <PlusAiPromptDialog note={view.note} busy={busy} error={error} focusDialog={focusDialog.current} onAnswer={answer} />;
}

/**
 * The dialog: the question, core's Whimbrel AI data note (its description; core's English is the
 * catalog key, so it reads in the user's language), where to change it, the three answers. While an
 * answer is saving the buttons are aria-disabled, not disabled, so focus stays in the dialog and Esc works.
 */
export function PlusAiPromptDialog(props: {
  note: string;
  busy: boolean;
  error: string | null;
  focusDialog: boolean;
  onAnswer: (choice: PlusAiPromptAnswer) => void;
}) {
  return (
    <WRoot>
      <WDialog
        title={t("Plus includes a monthly Whimbrel AI allowance. Use it for your AI features?")}
        body={t(props.note)}
        focusDialog={props.focusDialog}
        onCancel={() => {
          if (!props.busy) props.onAnswer("not_now");
        }}
        details={
          <>
            <p className="w-dialog-body">
              {t(
                "You can change this any time in Settings → AI. To keep a particular client's material away from hosted models, use Exclude from hosted models on that client's page.",
              )}
            </p>
            {props.error && (
              <p role="alert" className="w-dialog-body">
                {props.error}
              </p>
            )}
          </>
        }
      >
        <WButton variant="quiet" aria-disabled={props.busy} onClick={() => !props.busy && props.onAnswer("not_now")}>
          {t("Not now")}
        </WButton>
        <WButton variant="secondary" aria-disabled={props.busy} onClick={() => !props.busy && props.onAnswer("keep")}>
          {t("Keep my current setup")}
        </WButton>
        {/* The default: focused when the dialog opens, unless the user was typing elsewhere. */}
        <WButton variant="primary" data-initial="" aria-disabled={props.busy} onClick={() => !props.busy && props.onAnswer("use_whimbrel")}>
          {t("Use Whimbrel AI")}
        </WButton>
      </WDialog>
    </WRoot>
  );
}
