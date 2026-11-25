// One exchange on Today and on a conversation page, in the same parts: "You asked" and the question, the answer with its numbered marks, its
// sources as cards numbered to match (lowest first), the model that answered in small text and the answered-locally line. The answer card,
// the source cards and the marks are the Find page's own (`AnswerCard`, `FindCardView`, `sourceMarks`), so there is one Ask on this PC.

import { useMemo, useState } from "react";
import { AnswerCard } from "../library/redesign/AnswerCard";
import { FindCardView } from "../library/redesign/FindCard";
import { LegacyChips } from "../library/redesign/FindPage";
import { centeredScrollTop, type FindOpen } from "../library/redesign/findModel";
import type { SourcedTurn } from "../library/redesign/askSession";
import { t, useL10n } from "../l10n";
import { NO_FILTER_INPUT } from "./runTurn";

/** The cards under an answer: the sources it cites, by number. A saved answer's sources are exactly the cited ones. */
export function cardsOf(turn: Pick<SourcedTurn, "sources" | "cited" | "legacy">) {
  if (turn.legacy || !turn.cited) return [];
  const cited = new Set(turn.cited);
  return turn.sources.filter((s) => cited.has(s.n)).sort((a, b) => a.n - b.n);
}

/** The nearest ancestor that scrolls up and down, or null. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const y = getComputedStyle(p).overflowY;
    if ((y === "auto" || y === "scroll") && p.scrollHeight > p.clientHeight) return p;
  }
  return null;
}

/** Brings a source's card into view (the page's own scroll only, never the window's) and puts focus on it. */
export function focusSource(id: string): void {
  requestAnimationFrame(() => {
    const card = document.getElementById(id);
    if (!card) return;
    const calm = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    const box = scrollParent(card);
    if (box) {
      const b = box.getBoundingClientRect();
      const c = card.getBoundingClientRect();
      box.scrollTo({ top: centeredScrollTop({ scrollTop: box.scrollTop, scrollHeight: box.scrollHeight, top: b.top, height: box.clientHeight }, { top: c.top, height: c.height }), behavior: calm ? "auto" : "smooth" });
    }
    card.focus({ preventScroll: true });
  });
}

export interface ExchangeViewProps {
  turn: SourcedTurn;
  /** Distinguishes the cards of one exchange from another's on the same page. */
  idPrefix: string;
  /** The newest exchange: only it can be stopped. */
  last: boolean;
  onStop: () => void;
  onOpen: (open: FindOpen) => void;
}

export function ExchangeView({ turn, idPrefix, last, onStop, onOpen }: ExchangeViewProps) {
  const { locale } = useL10n();
  const [selected, setSelected] = useState<number | null>(null);
  const cards = useMemo(() => cardsOf(turn), [turn.sources, turn.cited, turn.legacy]); // eslint-disable-line react-hooks/exhaustive-deps
  const now = useMemo(() => new Date(), []);
  const onMark = (n: number) => {
    setSelected(n);
    focusSource(`${idPrefix}-${n}`);
  };
  return (
    <article className="th-cv-exchange">
      <p className="th-cv-asked">{t("You asked")}</p>
      <p className="th-cv-question">{turn.query}</p>
      <AnswerCard compact turn={turn} filter={NO_FILTER_INPUT} selected={selected} onMark={onMark} onStop={onStop} last={last} legacyChips={<LegacyChips turn={turn} onOpen={onOpen} />} />
      {cards.length > 0 ? (
        <ul className="th-find-cards th-cv-cards" aria-label={t("Sources for this answer")}>
          {cards.map((e) => (
            <li key={e.n} className="th-find-item">
              <FindCardView card={e.card} number={e.n} unreadable={e.unreadable} idPrefix={idPrefix} selected={selected === e.n} now={now} locale={locale} onOpen={onOpen} />
            </li>
          ))}
        </ul>
      ) : null}
    </article>
  );
}
