// The time line: results grouped by day, each day a stop on the Thread line. After an answer its sources come first as numbered evidence
// cards in time order, then the other results. "Best matches first" is the same cards in one list with no days.

import { memo, useId, useMemo, type ReactNode } from "react";
import type { FoundCard, NumberedSourceView } from "../../generated/types";
import { t, useL10n } from "../../l10n";
import { WSectionLabel, WSpine } from "../../system";
import { FindCardView } from "./FindCard";
import { dayGroups, type FindOpen } from "./findModel";

export function DayLine({ label, cards, order, render }: { label: string; cards: readonly FoundCard[]; order: "newest" | "oldest"; render: (card: FoundCard) => ReactNode }) {
  const { locale } = useL10n();
  const groups = useMemo(() => dayGroups(cards, new Date(), locale, order), [cards, locale, order]);
  const id = useId();
  // Each day is named by its day alone (the heading); its cards are the list inside it, each with its own name.
  return (
    <WSpine label={label}>
      {groups.map((g, i) => (
        <li key={g.key} className={i === groups.length - 1 ? "w-spine-stop w-spine-last" : "w-spine-stop"} data-node="past" aria-labelledby={`${id}-${i}`}>
          <span className="w-spine-time" aria-hidden="true">
            {g.label}
          </span>
          <span className="w-spine-rail w-spine-connector-solid" aria-hidden="true">
            <span className="w-spine-node w-spine-node-past" />
          </span>
          <div className="w-spine-body">
            <h3 className="w-visually-hidden" id={`${id}-${i}`}>
              {g.label}
            </h3>
            <ul className="th-find-cards">{g.cards.map((c) => render(c))}</ul>
          </div>
        </li>
      ))}
    </WSpine>
  );
}

export interface ResultsProps {
  /** The answer's cited sources, with their numbers; empty before an answer. */
  evidence: readonly NumberedSourceView[];
  selected: number | null;
  results: readonly FoundCard[];
  mentions: readonly FoundCard[];
  person: string | null;
  grouped: boolean;
  onOpen: (open: FindOpen) => void;
}

/** The evidence of an answer and the results under it. Memoised on what it draws, so a streaming answer does not redraw the list. */
export const FindResultsView = memo(function FindResultsView({ evidence, selected, results, mentions, person, grouped, onOpen }: ResultsProps) {
  const { locale } = useL10n();
  const now = useMemo(() => new Date(), []);
  const byKey = useMemo(() => new Map(evidence.map((e) => [`${e.card.kind}-${e.card.id}`, e.n])), [evidence]);
  const unreadable = useMemo(() => new Set(evidence.filter((e) => e.unreadable).map((e) => e.n)), [evidence]);
  const card = (c: FoundCard, n?: number) => (
    <li key={`${c.kind}-${c.id}`} className="th-find-item">
      <FindCardView card={c} number={n} selected={n != null && n === selected} unreadable={n != null && unreadable.has(n)} now={now} locale={locale} onOpen={onOpen} />
    </li>
  );
  const evidenceCards = useMemo(() => evidence.map((e) => e.card), [evidence]);
  const numberOf = (c: FoundCard) => byKey.get(`${c.kind}-${c.id}`);
  return (
    <div className="th-find-results">
      {evidence.length > 0 ? (
        <section aria-labelledby="th-find-sources-h">
          <WSectionLabel id="th-find-sources-h">{t("Sources for this answer")}</WSectionLabel>
          <DayLine label={t("Sources for this answer")} cards={evidenceCards} order="oldest" render={(c) => card(c, numberOf(c))} />
        </section>
      ) : null}
      {results.length > 0 ? (
        <section aria-labelledby="th-find-results-h">
          <WSectionLabel id="th-find-results-h">{evidence.length > 0 ? t("More from your work memory") : t("Results")}</WSectionLabel>
          {grouped ? (
            <DayLine label={t("Results by day")} cards={results} order="newest" render={(c) => card(c)} />
          ) : (
            <ul className="th-find-cards th-find-flat" aria-label={t("Best matches first")}>
              {results.map((c) => card(c))}
            </ul>
          )}
        </section>
      ) : null}
      {mentions.length > 0 && person ? (
        <section aria-labelledby="th-find-mentions-h">
          <WSectionLabel id="th-find-mentions-h">{t("Mentions of {name}", { name: person })}</WSectionLabel>
          <p className="th-find-note">{t("Captures whose text holds the name. They are not from {name} and they are listed apart.", { name: person })}</p>
          {grouped ? (
            <DayLine label={t("Mentions by day")} cards={mentions} order="newest" render={(c) => card(c)} />
          ) : (
            <ul className="th-find-cards th-find-flat" aria-label={t("Mentions of {name}", { name: person })}>
              {mentions.map((c) => card(c))}
            </ul>
          )}
        </section>
      ) : null}
    </div>
  );
});
