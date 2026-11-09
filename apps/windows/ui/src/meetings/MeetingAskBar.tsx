import { useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import { chatStop, meetingAsk, openAiSettings } from "../api";
import { markdownComponents } from "../chat/AskView";
import { isBusy, statusLine } from "../chat/chatModel";
import { describeError, isCommandError } from "../errors";
import { useL10n } from "../l10n";
import { ASK_PRESETS, applyMeetingEvent, historyOf, newExchange, transcriptCitationIds, type MeetingExchange } from "./meetingAskModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * The Ask bar on a meeting page (issue 246, the Mac's `MeetingAskBar`): questions answered from
 * this meeting's transcript only, the transcript so far while it records. Exchanges live as long
 * as the bar does and are never saved. Each cited line becomes a "Transcript N" chip.
 */
export function MeetingAskBar({
  meetingId,
  live,
  disabledReason,
  onCite,
}: {
  meetingId: number;
  /** During the call: answers come from the transcript so far, and say so. */
  live: boolean;
  disabledReason: string | null;
  onCite: (segmentId: number) => void;
}) {
  const { t } = useL10n();
  const [exchanges, setExchanges] = useState<MeetingExchange[]>([]);
  const [question, setQuestion] = useState("");
  const [asking, setAsking] = useState(false);
  // False once the bar is gone, so a late event is dropped; the run in flight is stopped.
  const mounted = useRef(true);
  const runId = useRef<string | null>(null);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (runId.current) void chatStop(runId.current);
    };
  }, []);

  const last = exchanges[exchanges.length - 1];
  const busy = asking || isBusy(last?.turn);
  const disabled = disabledReason != null;

  const submit = (text: string) => {
    const typed = text.trim();
    if (!typed || busy || disabled) return;
    const exchange = newExchange(typed);
    const history = historyOf(exchanges);
    const update = (f: (e: MeetingExchange) => MeetingExchange) =>
      setExchanges((all) => (all.length ? [...all.slice(0, -1), f(all[all.length - 1])] : all));
    setQuestion("");
    setAsking(true);
    setExchanges((all) => [...all, exchange]);
    meetingAsk(meetingId, exchange.prompt, history, (event) => {
      if (event.kind === "started") runId.current = event.runId ?? null;
      if (event.kind === "fell_back") runId.current = null;
      const done = event.kind === "completed" || event.kind === "cancelled" || event.kind === "failed";
      if (done) runId.current = null;
      if (!mounted.current) return;
      if (done) setAsking(false);
      if (event.kind === "cancelled") {
        // Stopped: no answer, and the question goes back in the field.
        setExchanges((all) => all.slice(0, -1));
        setQuestion(exchange.question);
        return;
      }
      update((e) => applyMeetingEvent(e, event));
    }).catch((e: unknown) => {
      if (!mounted.current) return;
      setAsking(false);
      update((ex) => ({ ...ex, turn: { ...ex.turn, status: "failed", error: message(e) } }));
    });
  };

  const retry = (index: number) => {
    if (busy || disabled) return;
    const failed = exchanges[index];
    // The failed exchange goes and the question is asked again. History skips it either way.
    setExchanges((all) => all.filter((_, i) => i !== index));
    submit(failed.question);
  };

  const stoppable = busy ? (last?.turn.runId ?? null) : null;

  return (
    <section className="field meeting-ask" aria-label={t("Ask about this meeting")}>
      {exchanges.length > 0 && (
        <div className="meeting-ask-answers">
          {live && <p className="muted small">{t("Based on the transcript so far")}</p>}
          <ol className="turns">
            {exchanges.map((exchange, i) => (
              <ExchangeView key={i} exchange={exchange} busy={busy} onRetry={() => retry(i)} onCite={onCite} />
            ))}
          </ol>
        </div>
      )}
      {!disabled && (
        <div className="chips">
          {ASK_PRESETS.map((p) => (
            <button key={p.id} className="mode starter" disabled={busy} onClick={() => submit(t(p.label))}>
              {t(p.label)}
            </button>
          ))}
        </div>
      )}
      <form
        className="composer"
        onSubmit={(e) => {
          e.preventDefault();
          submit(question);
        }}
      >
        <input
          className="input grow"
          value={question}
          placeholder={disabledReason ?? t("Ask about this meeting")}
          aria-label={t("Ask about this meeting")}
          disabled={disabled || busy}
          onChange={(e) => setQuestion(e.target.value)}
        />
        {stoppable ? (
          <button type="button" className="mode" onClick={() => void chatStop(stoppable)}>
            {t("Stop")}
          </button>
        ) : (
          <button type="submit" className="mode mode--active" disabled={disabled || busy || !question.trim()}>
            {t("Ask")}
          </button>
        )}
      </form>
    </section>
  );
}

function ExchangeView({
  exchange,
  busy,
  onRetry,
  onCite,
}: {
  exchange: MeetingExchange;
  busy: boolean;
  onRetry: () => void;
  onCite: (segmentId: number) => void;
}) {
  const { t } = useL10n();
  const { turn } = exchange;
  const status = statusLine(turn);
  const cited = transcriptCitationIds(turn.citations);
  return (
    <li className="turn">
      <p className="bubble bubble--question">{exchange.question}</p>
      <div
        className={turn.status === "failed" ? "bubble bubble--answer bubble--error" : "bubble bubble--answer"}
        aria-live={turn.status === "streaming" ? "polite" : undefined}
      >
        {turn.status === "done" ? (
          <div className="markdown">
            <ReactMarkdown components={markdownComponents}>{turn.text}</ReactMarkdown>
          </div>
        ) : turn.text ? (
          // Plain text until it is complete: half-streamed markdown reflows badly.
          <p className="streaming-text">{turn.text}</p>
        ) : null}
        {status && (
          <p className="muted small" role="status">
            {status}
          </p>
        )}
        {turn.error && <p role="alert">{turn.error}</p>}
        {turn.status === "failed" && (
          <div className="toolbar">
            <button className="mode" disabled={busy} onClick={onRetry}>
              {t("Retry")}
            </button>
            {turn.budgetSpent && (
              <button className="mode" onClick={() => void openAiSettings()}>
                {t("Open AI settings")}
              </button>
            )}
          </div>
        )}
        {exchange.narrowed && <p className="muted small">{t("Answered from the most relevant parts of a long transcript.")}</p>}
        {cited.length > 0 && (
          <div className="citations">
            {cited.map((id, n) => (
              <button key={id} className="chip citation" onClick={() => onCite(id)}>
                {t("Transcript {n}", { n: String(n + 1) })}
              </button>
            ))}
          </div>
        )}
      </div>
    </li>
  );
}
