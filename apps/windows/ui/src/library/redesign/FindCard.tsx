// One result or one source of an answer, as a card: the kind in words and by icon, the app, the title, the date and time, the device, the stored
// text (a few lines, more on request) and Open where there is something to open. A source that is gone from this PC says so and shows no text.

import { memo, useLayoutEffect, useRef, useState } from "react";
import type { FoundCard } from "../../generated/types";
import { t } from "../../l10n";
import { WButton, WIcon } from "../../system";
import { appDot } from "../../today/redesign/homeModel";
import { DeviceChip } from "../DeviceChip";
import { cardApp, cardTitle, cardWhen, kindIcon, kindLabel, openHint, openName, openTarget, sourceMarkName, type FindOpen } from "./findModel";

/** Whether the text is long enough to be worth a Show more, before the card has been measured. */
const longText = (text: string): boolean => text.length > 220 || text.split("\n").length > 3;

export interface FindCardProps {
  card: FoundCard;
  /** The source's number when this card is evidence for an answer. */
  number?: number;
  selected?: boolean;
  now: Date;
  locale: string;
  onOpen: (open: FindOpen) => void;
  /** The row could not be read just now: it may well exist, so the card does not say it is gone. */
  unreadable?: boolean;
  /** The element id's start for a numbered card (the page has one list of sources per answer). */
  idPrefix?: string;
}

export const FindCardView = memo(function FindCardView({ card, number, selected, unreadable, now, locale, onOpen, idPrefix = "th-find-source" }: FindCardProps) {
  const [expanded, setExpanded] = useState(false);
  const clamp = useRef<HTMLDivElement>(null);
  const text = card.text?.trim() ?? "";
  // Show more is offered when the clamped text really is cut, which depends on the card's width, not on how many characters it has.
  const [cut, setCut] = useState(() => longText(text));
  useLayoutEffect(() => {
    const el = clamp.current;
    if (!el || expanded) return;
    const measure = () => setCut(el.scrollHeight > el.clientHeight + 1);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, [text, expanded]);
  const open = openTarget(card);
  const title = cardTitle(card);
  const dot = card.kind === "capture" && card.app ? appDot(card.app) : null;
  const gone = !card.found;
  return (
    <article
      className={`w-source w-source-card th-find-card${number != null ? " th-find-card-numbered" : ""}${selected ? " w-source-selected" : ""}${gone ? " th-find-card-gone" : ""}`}
      aria-label={number != null ? sourceMarkName(number, card, locale, unreadable) : undefined}
      aria-current={selected ? "true" : undefined}
      data-source={number}
      id={number != null ? `${idPrefix}-${number}` : undefined}
      tabIndex={number != null ? -1 : undefined}
    >
      <div className="w-source-head">
        {number != null ? (
          <span className="w-source-num" aria-hidden="true">
            {number}
          </span>
        ) : null}
        {dot ? <span className={dot === "blue" ? "w-dot" : `w-dot w-dot-${dot}`} aria-hidden="true" /> : null}
        <WIcon name={kindIcon(card)} size={16} />
        <span className="w-source-name">{cardApp(card)}</span>
        <span className="w-source-kind th-find-kind">{kindLabel(card.kind)}</span>
      </div>
      {gone ? (
        <p className="th-find-gone">{unreadable ? t("Could not read this just now.") : t("No longer on this device")}</p>
      ) : (
        <>
          <h4 className="w-source-title th-find-title">{title}</h4>
          <p className="th-find-when">
            {cardWhen(card, now, locale)}
            {card.device ? <DeviceChip device={card.device} /> : null}
          </p>
          {card.kind === "thread" && card.sender ? <p className="th-find-when">{t("Last from {sender}", { sender: card.sender })}</p> : null}
          {text ? (
            <>
              <div className={expanded ? "th-find-text th-find-text-open" : "th-find-text"}>
                {/* The padding is on the box and the clamp is on this inner element, so a clamped text ends on a whole line. */}
                <div className="th-find-clamp" ref={clamp}>
                  {card.kind === "transcript" && card.speaker ? <b className="th-find-speaker">{card.speaker}{card.speakerIsGuess ? "? " : " "}</b> : null}
                  {text}
                </div>
              </div>
              {expanded || cut ? (
                <button type="button" className="th-find-more" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>
                  {expanded ? t("Show less") : t("Show more")}
                </button>
              ) : null}
            </>
          ) : null}
          {open ? (
            <div className="th-find-actions">
              <WButton icon="open" aria-label={openName(card, locale)} data-open-key={`${card.kind}-${card.id}`} onClick={() => onOpen(open)}>
                {t("Open")}
              </WButton>
              <span className="th-find-hint">{openHint(card)}</span>
            </div>
          ) : null}
        </>
      )}
    </article>
  );
});
