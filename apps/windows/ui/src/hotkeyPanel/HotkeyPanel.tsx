import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { hotkeyPanelClose, hotkeyPanelRead, openAiSettings, windowAsk } from "../api";
import { describeError, isCommandError } from "../errors";
import { useL10n } from "../l10n";
import { askErrorText, contextLine, initialState, reduce } from "./hotkeyPanelModel";

/**
 * The hotkey panel: one field, what it reads, where the text will go, and the answer. Opening reads the
 * window but sends nothing; Return asks. Escape closes and the shell returns to the previous window.
 */
export function HotkeyPanel() {
  const { t } = useL10n();
  const [state, dispatch] = useReducer(reduce, initialState);
  const [query, setQuery] = useState("");
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => void (copyTimer.current && clearTimeout(copyTimer.current)), []);
  const stateRef = useRef(state);
  stateRef.current = state;

  useEffect(() => {
    hotkeyPanelRead().then(
      (read) => dispatch({ type: "read", read }),
      (e) => dispatch({ type: "read_error", message: isCommandError(e) ? describeError(e) : String(e) }),
    );
  }, []);

  const close = useCallback(() => void hotkeyPanelClose().catch(() => {}), []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.isComposing) {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [close]);

  const ask = () => {
    const s = stateRef.current;
    const text = query.trim();
    if (!text || s.phase === "blocked" || s.phase === "reading" || s.phase === "asking" || s.requestId === null) return;
    // The reducer bumps askSeq on "ask"; events and the reply for this ask carry the new value.
    const mine = s.askSeq + 1;
    dispatch({ type: "ask" });
    windowAsk(s.requestId, text, (event) => dispatch({ type: "event", askSeq: mine, event })).then(
      (reply) => dispatch({ type: "reply", askSeq: mine, reply }),
      (err) => dispatch({ type: "ask_error", askSeq: mine, message: askErrorText(err, s.destination) }),
    );
  };

  const copy = () => {
    void navigator.clipboard.writeText(state.answer).then(
      () => {
        setCopied(true);
        if (copyTimer.current) clearTimeout(copyTimer.current);
        copyTimer.current = setTimeout(() => setCopied(false), 1500);
      },
      () => {},
    );
  };

  const line = state.read && state.destination ? contextLine(state.read.window, state.destination, state.truncated) : null;
  const blocked = state.phase === "blocked";

  return (
    <div className="alert hotkey-panel">
      <input
        className="input"
        autoFocus
        value={query}
        disabled={blocked}
        placeholder={t("Ask about this window")}
        aria-label={t("Ask about this window")}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            ask();
          }
        }}
      />
      {line?.reading ? <p className="muted small">{line.reading}</p> : null}
      {blocked ? (
        <>
          <p role="status" className="small">
            {state.blocked}
          </p>
          {state.read?.cannotAnswer?.kind === "local_model_missing" ? (
            <div className="popup-actions">
              <button className="mode" onClick={() => void openAiSettings()}>
                {t("Install a local model")}
              </button>
            </div>
          ) : null}
        </>
      ) : line ? (
        <p role="status" className="muted small">
          {line.destination}
          {state.destinationChanged ? ` ${t("Press Return to ask.")}` : ""}
        </p>
      ) : null}
      {state.phase === "failed" && state.error ? (
        <p role="alert" className="small">
          {state.error}
        </p>
      ) : null}
      {state.answer ? (
        <div className="hotkey-panel-answer" aria-live="polite">
          {state.answer}
        </div>
      ) : null}
      {state.phase === "answered" ? (
        <div className="popup-actions">
          <button className="mode" onClick={copy}>
            {copied ? t("Copied") : t("Copy")}
          </button>
        </div>
      ) : null}
    </div>
  );
}
