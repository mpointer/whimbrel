import type { ReactNode } from "react";
import { t } from "../l10n";
import { chipLabel, chipTarget, durationText, peopleText, spokenLabel, subtitle, type MeetingCardModel, type MeetingCardTarget, type MeetingChip } from "./meetingCardModel";

/** Opens a card's or chip's target; `none` never reaches it. */
export type OpenTarget = (target: MeetingCardTarget) => void;

function chipKey(chip: MeetingChip): string {
  return chip.kind === "prep" ? `prep-${chip.key}` : chip.kind === "topic" ? `topic-${chip.id}` : chip.kind;
}

/** A meeting's chips in a wrapping row, each its own button, so a click on one doesn't also open the card. */
export function MeetingChips({ chips, card, onOpen }: { chips: MeetingChip[]; card: MeetingCardTarget; onOpen: OpenTarget }) {
  if (chips.length === 0) return null;
  return (
    <span className="meeting-chips">
      {chips.map((chip) => {
        const target = chipTarget(chip, card);
        const label = chipLabel(chip);
        return (
          <button
            key={chipKey(chip)}
            type="button"
            className={chip.kind === "topic" ? "chip citation meeting-chip topic-chip" : "chip citation meeting-chip"}
            title={chip.kind === "prep" ? t("Open brief") : chip.kind === "topic" ? label : undefined}
            disabled={target.kind === "none"}
            onClick={(e) => {
              e.stopPropagation();
              onOpen(target);
            }}
          >
            {label}
          </button>
        );
      })}
    </span>
  );
}

/** The trailing chevron on a card that goes somewhere. */
function Chevron({ className }: { className: string }) {
  return (
    <svg className={className} width="12" height="12" viewBox="0 0 12 12" aria-hidden="true" focusable="false">
      <path d="M4.5 2.5 8 6l-3.5 3.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/**
 * The shared hit target: one button stretched under the whole card (time column included), with the text drawn
 * over it and the chips as their own buttons above it. An inert card has no button, hover or chevron.
 */
function Shell({
  model,
  className,
  active,
  onOpen,
  children,
}: {
  model: MeetingCardModel;
  className: string;
  active?: boolean;
  onOpen: OpenTarget;
  children: ReactNode;
}) {
  const clickable = model.target.kind !== "none";
  const classes = [className, clickable ? "meeting-item--clickable" : "", active ? "meeting-item--active" : ""].filter(Boolean).join(" ");
  return (
    // A card with nowhere to go has no button, so the card itself carries the spoken label (its text is aria-hidden).
    <div className={`meeting-item ${classes}`} {...(clickable ? {} : { role: "group", "aria-label": spokenLabel(model) })}>
      {clickable && (
        <button type="button" className="meeting-item__hit" aria-label={spokenLabel(model)} aria-current={active || undefined} onClick={() => onOpen(model.target)} />
      )}
      {children}
      {clickable ? <Chevron className="meeting-item__chevron" /> : <span className="meeting-item__chevron" aria-hidden="true" />}
    </div>
  );
}

/** A Today timeline card: title, "start to end · N people", the status line and the chips. */
export function MeetingCard({ model, onOpen }: { model: MeetingCardModel; onOpen: OpenTarget }) {
  return (
    <Shell model={model} className="meeting-card" onOpen={onOpen}>
      <span className="meeting-item__main">
        <span className="meeting-item__text" aria-hidden="true">
          <span className="meeting-card__title">{model.title}</span>
          <span className="meeting-item__sub">{subtitle(model)}</span>
          {model.statusLine && <span className="meeting-item__sub">{model.statusLine}</span>}
        </span>
        <MeetingChips chips={model.chips} card={model.target} onOpen={onOpen} />
      </span>
    </Shell>
  );
}

/**
 * A Meetings row: the start with the duration under it, then the title, the people line and the chips, then the
 * chevron. `meta` is what else Windows shows on the people line (how it started, the device).
 */
export function MeetingListRow({ model, active, meta, onOpen }: { model: MeetingCardModel; active: boolean; meta?: ReactNode; onOpen: OpenTarget }) {
  const people = model.peopleCount != null ? peopleText(model.peopleCount) : null;
  return (
    <Shell model={model} className="meeting-row" active={active} onOpen={onOpen}>
      <span className="meeting-row__time" aria-hidden="true">
        <span className="meeting-row__start">{model.startText}</span>
        {model.durationMinutes != null && <span className="meeting-item__sub">{durationText(model.durationMinutes)}</span>}
      </span>
      <span className="meeting-item__main">
        <span className="meeting-item__text">
          <span className="meeting-row__title" aria-hidden="true">
            {model.title}
          </span>
          <span className="meeting-item__sub meeting-row__people">
            {people && <span aria-hidden="true">{people}</span>}
            {meta}
          </span>
          {model.statusLine && (
            <span className="meeting-item__sub" aria-hidden="true">
              {model.statusLine}
            </span>
          )}
        </span>
        <MeetingChips chips={model.chips} card={model.target} onOpen={onOpen} />
      </span>
    </Shell>
  );
}
