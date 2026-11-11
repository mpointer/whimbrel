import { Fragment, type ReactNode } from "react";
import { useL10n } from "../l10n";
import { MeetingCard, type OpenTarget } from "../meetings/MeetingCard";
import { nowLabel, nowMarkerIndex, timeline, type HeldChips, type TimelineEntry } from "../meetings/meetingCardModel";
import type { TopicRefView } from "../generated/types";
import { prepLine, rowAction } from "./todayModel";
import type { ScheduleState } from "./useSchedule";

function dotClass(entry: TimelineEntry): string {
  if (entry.model.isRunning) return "timeline-dot timeline-dot--running";
  return entry.model.isPast ? "timeline-dot timeline-dot--past" : "timeline-dot timeline-dot--upcoming";
}

/**
 * Today's schedule as a timeline (Standard's layout; spec 2026-11-10 option C): a line down the left with a dot
 * and a card per meeting in start order, past ones included, and a "Now" marker before the first one that hasn't
 * ended. Start, Open and the next meeting's brief toggle stay under their card.
 */
export function TodayTimeline({
  state,
  title,
  briefOpen,
  brief,
  onToggleBrief,
  onOpenMeeting,
  onOpen,
  eventTopics,
  heldChips,
}: {
  state: ScheduleState;
  /** The skin's heading for the day ("Today" in Whimbrel). */
  title: string;
  briefOpen: boolean;
  /** The next meeting's brief card, shown under its card while open. */
  brief: ReactNode;
  onToggleBrief: () => void;
  onOpenMeeting: (meetingId: number) => void;
  /** A card's or chip's target: a meeting, a brief or a topic page. */
  onOpen: OpenTarget;
  /** Each event's topic, by position in `schedule.events` (useTodayChips). */
  eventTopics: (TopicRefView | null)[];
  /** Each held meeting's Prep key and topic (useTodayChips). */
  heldChips: ReadonlyMap<number, HeldChips>;
}) {
  const { schedule, next, now, starting, error, start } = state;
  const { locale, t } = useL10n();
  const events = schedule?.events ?? [];
  const held = schedule?.earlierToday ?? [];

  if (!schedule) return error ? <p role="alert">{error}</p> : null;
  // No calendar, or nothing today: the timeline has nothing to say.
  if (events.length === 0 && held.length === 0) return error ? <p role="alert">{error}</p> : null;

  const entries = timeline(events, held, next, { eventTopics, held: heldChips }, now, locale);
  const marker = nowMarkerIndex(
    entries.map((e) => e.endMs),
    now,
  );
  const nowRow = (
    <li className="timeline-now" key="now">
      <span className="timeline-dot timeline-dot--running" aria-hidden="true" />
      <span className="timeline-now__label">{nowLabel(now, locale)}</span>
    </li>
  );

  const extras = (entry: TimelineEntry): ReactNode => {
    const e = entry.event;
    if (!e) {
      const id = entry.model.target.kind === "meeting" ? entry.model.target.id : null;
      return id == null ? null : (
        <div className="timeline-extras">
          <button type="button" className="mode" onClick={() => onOpenMeeting(id)}>
            {t("Open")}
          </button>
        </div>
      );
    }
    const action = rowAction(e, schedule.recording);
    const prep = prepLine(e, next);
    if (!action && !prep) return null;
    return (
      <div className="timeline-extras">
        {prep === "ready" && (
          <button type="button" className="link" onClick={onToggleBrief} aria-expanded={briefOpen}>
            {briefOpen ? t("Hide brief") : t("Open brief")}
          </button>
        )}
        {prep === "none" && (
          <span className="muted small" title={t("Meeting Prep makes a brief shortly before the meeting once it is turned on in Automations.")}>
            {t("No prep yet")}
          </span>
        )}
        <span className="timeline-extras__spacer" />
        {action === "start" && (
          <button type="button" className="mode" onClick={() => start(e.eventId)} disabled={starting !== null}>
            {starting === e.eventId ? t("Starting…") : t("Start")}
          </button>
        )}
        {action === "open" && (
          <button type="button" className="mode" onClick={() => onOpenMeeting(e.recordedMeetingId as number)}>
            {t("Open")}
          </button>
        )}
      </div>
    );
  };

  return (
    <section className="timeline-section" aria-labelledby="schedule-heading">
      <h2 id="schedule-heading" className="section-label">
        {title}
      </h2>
      <ol className="timeline">
        {entries.map((entry, i) => (
          <Fragment key={entry.key}>
            {i === marker && nowRow}
            <li className="timeline-item">
              <span className={dotClass(entry)} aria-hidden="true" />
              <div className="timeline-stack">
                <MeetingCard model={entry.model} onOpen={onOpen} />
                {extras(entry)}
                {entry.isNext && briefOpen && brief}
              </div>
            </li>
          </Fragment>
        ))}
        {marker >= entries.length && nowRow}
      </ol>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
