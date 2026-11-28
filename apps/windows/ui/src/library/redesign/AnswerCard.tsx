// The answer: its words with the numbered source marks where the model put them, Copy answer, Stop while it streams, the note that answers can be
// wrong, the model that answered and the notes that say how it was made. The answer is announced once, when it finishes, never while it streams.

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import { openAiSettings } from "../../api";
import { statusLine } from "../../chat/chatModel";
import { markdownComponents } from "../../chat/markdown";
import type { FindFilterInput } from "../../generated/types";
import { t, useL10n } from "../../l10n";
import { WButton, WIcon, WSourceMark } from "../../system";
import { leftOutLine } from "../../topics/clientControlsModel";
import type { SourcedTurn } from "./askSession";
import { answeredLine, drewOnLine, sourceMarkName, sourceOfHref } from "./findModel";
import { rehypeSourceMarks } from "./sourceMarks";

export interface AnswerCardProps {
  turn: SourcedTurn;
  filter: FindFilterInput;
  selected: number | null;
  onMark: (n: number) => void;
  onStop: () => void;
  /** Today's chips for an answer saved before numbered sources. */
  legacyChips: ReactNode;
  /** The turn is the newest: only it can be stopped. */
  last: boolean;
  /** Conversations and Today (screen 6): the question is drawn by the caller as "You asked", and there is no "Answer" header, drew line, Copy or
   *  can-be-wrong note here (the caller's own actions follow). Stop, the status lines, the error and the model that answered stay. */
  compact?: boolean;
}

export function AnswerCard({ turn, filter, selected, onMark, onStop, legacyChips, last, compact = false }: AnswerCardProps) {
  const { locale } = useL10n();
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => () => clearTimeout(timer.current), []);
  // A mark is drawn only for a number the core says the answer cites (live: the `cited` event; reopened: the saved sources, which are
  // exactly the cited ones). Any other bracketed number stays the text the model wrote.
  const valid = useMemo(() => new Set(turn.cited ?? []), [turn.cited]);
  // Marks are placed on the rendered answer, so code and link text are never touched. An answer saved before numbered sources has none.
  const plugins = useMemo(() => (turn.legacy ? [] : [[rehypeSourceMarks, valid]]) as never, [turn.legacy, valid]);
  const bySource = useMemo(() => new Map(turn.sources.map((s) => [s.n, s])), [turn.sources]);
  const components = useMemo(
    () => ({
      ...markdownComponents,
      a: ({ children, href }: { children?: ReactNode; href?: string }) => {
        const n = sourceOfHref(href);
        if (n === null) return markdownComponents.a({ children, href });
        return <WSourceMark number={n} label={sourceMarkName(n, bySource.get(n)?.card, locale, bySource.get(n)?.unreadable)} selected={selected === n} onSelect={onMark} />;
      },
    }),
    [bySource, locale, selected, onMark],
  );

  // Said once, when this answer finishes while it is on screen. A saved answer that is opened is already done: nothing is said for it.
  const [said, setSaid] = useState("");
  const before = useRef(turn.status);
  useEffect(() => {
    if (before.current !== "done" && turn.status === "done") setSaid(turn.text);
    before.current = turn.status;
  }, [turn.status, turn.text]);

  const busy = turn.status === "starting" || turn.status === "waiting" || turn.status === "streaming" || turn.status === "blocking";
  const status = statusLine(turn);
  const copy = () => {
    navigator.clipboard.writeText(turn.text).then(
      () => {
        setCopied(true);
        clearTimeout(timer.current);
        timer.current = setTimeout(() => setCopied(false), 2000);
      },
      () => {},
    );
  };
  const answered = answeredLine(turn);
  const left = turn.status === "done" && turn.leftOut != null ? leftOutLine(turn.leftOut) : null;
  return (
    <section className={`${turn.status === "failed" ? "th-find-answer th-find-answer-failed" : "th-find-answer"}${compact ? " th-cv-answer" : ""}`} aria-label={t("Answer")}>
      {compact ? (
        last && busy && turn.runId ? (
          <div className="th-cv-stop">
            <WButton icon="stop" onClick={onStop}>
              {t("Stop")}
            </WButton>
          </div>
        ) : null
      ) : (
      <header className="th-find-answer-head">
        <WIcon name="spark" size={16} />
        <h2>{t("Answer")}</h2>
        <span className="th-find-drew">{drewOnLine(turn.filter ?? filter, locale, turn.drewRange)}</span>
        {turn.status === "done" && turn.text ? (
          <WButton icon="copy" onClick={copy}>
            {copied ? t("Copied") : t("Copy answer")}
          </WButton>
        ) : null}
        {last && busy && turn.runId ? (
          <WButton icon="stop" onClick={onStop}>
            {t("Stop")}
          </WButton>
        ) : null}
      </header>
      )}
      {compact ? null : <p className="th-find-question">{turn.query}</p>}
      {turn.status === "done" ? (
        <div className="markdown th-find-text-answer">
          <ReactMarkdown components={components} rehypePlugins={plugins}>
            {turn.text}
          </ReactMarkdown>
        </div>
      ) : turn.text ? (
        // Plain text until it is complete: half-streamed markdown reflows badly, and a mark needs the whole answer. Same face, size and
        // line height as the finished answer (the container is the same), so nothing jumps when it ends.
        <div className="markdown th-find-text-answer">
          <p className="th-find-stream">{turn.text}</p>
        </div>
      ) : null}
      {status ? (
        <p className="th-find-status" role="status">
          {status}
        </p>
      ) : null}
      {turn.error ? (
        <p role="alert" className="th-find-error">
          {turn.error}
        </p>
      ) : null}
      {turn.budgetSpent ? (
        <WButton onClick={() => void openAiSettings()}>{t("Open AI settings")}</WButton>
      ) : null}
      {turn.legacy ? legacyChips : null}
      {turn.status === "done" ? (
        <>
          {compact ? null : (
            <p className="th-find-warn">
              <WIcon name="info" size={16} />
              <span>{t("Answers can be wrong. Open the sources to check.")}</span>
            </p>
          )}
          {turn.model ? <p className="th-find-small">{turn.model}</p> : null}
          {answered ? <p className="th-find-small">{answered}</p> : null}
          {left ? <p className="th-find-small">{left}</p> : null}
        </>
      ) : null}
      <div className="w-visually-hidden" role="status" aria-live="polite">
        {said}
      </div>
    </section>
  );
}
