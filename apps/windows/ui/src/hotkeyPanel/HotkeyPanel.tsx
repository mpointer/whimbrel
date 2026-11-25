import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { hotkeyPanelClose, hotkeyPanelRead, openAiSettings, windowAsk } from "../api";
import { describeError, isCommandError } from "../errors";
import { useL10n } from "../l10n";
import { useRedesign } from "../redesign";
import { WButton, WIconButton, WRoot } from "../system";
import { askErrorText, contextLine, initialState, reduce } from "./hotkeyPanelModel";

/**
 * The hotkey panel: one field, what it reads, where the text will go, and the answer. Opening reads the
 * window but sends nothing; Return asks. Escape closes and the shell returns to the previous window.
 */
export function HotkeyPanel() {
  const { t } = useL10n();
  const redesign = useRedesign();
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

  // The same states and buttons, drawn in the design system (redesign only). The text sent, where it goes and the answer
  // are exactly what the classic panel shows; the panel is also hidden from a shared screen (the shell, window.rs).
  if (redesign)
    return (
      <WRoot className="w-win-root">
        <main className="w-win w-hk" aria-label={t("Ask about this window")}>
          <div className="w-hk-row">
            <input
              className="w-hk-input"
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
            <WIconButton icon="close" label={t("Dismiss")} onClick={close} />
          </div>
          {line?.reading ? <p className="w-hk-line">{line.reading}</p> : null}
          {/* Reading: nothing is known yet, so nothing is said about where text goes; the line below comes with the read. */}
          {state.phase === "reading" ? (
            <p role="status" className="w-hk-line">
              {t("Reading this window…")}
            </p>
          ) : null}
          {blocked ? (
            <>
              <p role="status" className="w-hk-line w-hk-strong">
                {state.blocked}
              </p>
              {state.read?.cannotAnswer?.kind === "local_model_missing" ? (
                <div className="w-win-actions">
                  <WButton onClick={() => void openAiSettings()}>{t("Install a local model")}</WButton>
                </div>
              ) : null}
            </>
          ) : line ? (
            <p role="status" className="w-hk-line">
              {line.destination}
              {state.destinationChanged ? ` ${t("Press Return to ask.")}` : ""}
            </p>
          ) : null}
          {state.phase === "failed" && state.error ? (
            <p role="alert" className="w-hk-error">
              {state.error}
            </p>
          ) : null}
          {state.answer ? (
            <div className="w-hk-answer" aria-live="polite">
              {state.answer}
            </div>
          ) : null}
          {state.phase === "answered" ? (
            <div className="w-win-actions">
              <WButton icon="copy" onClick={copy}>
                {copied ? t("Copied") : t("Copy")}
              </WButton>
            </div>
          ) : null}
        </main>
      </WRoot>
    );

  return (
    <div className="alert hotkey-panel">
      <div className="hotkey-panel-row">
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
        <button className="hotkey-panel-close" onClick={close} aria-label={t("Dismiss")} title={t("Dismiss")}>
          ×
        </button>
      </div>
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
