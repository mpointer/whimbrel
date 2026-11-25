// The area under the Ask bar on Today (redesign flag only; screen 6, part 1): "You asked", the answer as it streams, its numbered sources, the
// answered-locally line, then Continue as a conversation, Copy and Clear. Draws nothing while empty. The answer is announced once, when it
// finishes (the answer card's own live region), and focus moves to the conversation's page when Continue opens it.

import { useEffect, useRef, useState } from "react";
import { t } from "../l10n";
import { WButton, WNotice } from "../system";
import type { FindOpen } from "../library/redesign/findModel";
import { ExchangeView } from "./ExchangeView";
import { focusAskField } from "./nav";
import { busy, canContinue, clearToday, continueToday, retryToday, stopToday, useToday } from "./todayStore";
import "./conversations.css";

/** The answer area, with a status line that stays in the page whether or not there is an answer, so a screen reader hears "Answering…" once when a
 *  question starts (a live region added together with its text is often not read). */
export function TodayAnswer({ onOpen }: { onOpen: (open: FindOpen) => void }) {
  const today = useToday();
  return (
    <>
      <Area onOpen={onOpen} />
      <p className="w-visually-hidden" role="status">
        {busy(today) ? t("Answering…") : ""}
      </p>
    </>
  );
}

function Area({ onOpen }: { onOpen: (open: FindOpen) => void }) {
  const today = useToday();
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => () => clearTimeout(timer.current), []);
  const { question, turn } = today;
  // Clear leaves nothing under the bar, so focus goes to the Ask field, where the next question is typed.
  const clear = () => {
    clearToday();
    focusAskField();
  };
  if (!question || !turn) return today.note ? <div className="th-today-answer"><WNotice tone="warning">{today.note}</WNotice></div> : null;
  const failed = turn.status === "failed";
  const done = turn.status === "done";
  const copy = () =>
    navigator.clipboard.writeText(turn.text).then(
      () => {
        setCopied(true);
        clearTimeout(timer.current);
        timer.current = setTimeout(() => setCopied(false), 2000);
      },
      () => {},
    );
  return (
    <section className="th-today-answer" aria-label={t("Answer to your question")}>
      {failed ? (
        <div className="th-cv-exchange">
          <p className="th-cv-asked">{t("You asked")}</p>
          <p className="th-cv-question">{turn.query}</p>
          <WNotice tone="error">{turn.error ?? t("The question could not be answered.")}</WNotice>
          <div className="th-cv-actions">
            <WButton variant="primary" onClick={retryToday}>
              {t("Try again")}
            </WButton>
            <WButton onClick={clear}>{t("Clear")}</WButton>
          </div>
        </div>
      ) : (
        <>
          <ExchangeView turn={turn} idPrefix="th-today-source" last onStop={stopToday} onOpen={onOpen} />
          {busy(today) ? <div className="th-cv-quiet" aria-hidden="true" /> : null}
          <div className="th-cv-actions">
            <WButton variant="primary" icon="chat" disabled={!canContinue(today)} onClick={() => void continueToday()}>
              {t("Continue as a conversation")}
            </WButton>
            {done ? (
              <WButton icon="copy" onClick={copy}>
                {copied ? t("Copied") : t("Copy")}
              </WButton>
            ) : null}
            <WButton icon="close" onClick={clear}>
              {t("Clear")}
            </WButton>
          </div>
        </>
      )}
      {today.note ? <WNotice tone="warning">{today.note}</WNotice> : null}
    </section>
  );
}
