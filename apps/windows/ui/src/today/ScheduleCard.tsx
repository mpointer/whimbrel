import { timeLabel } from "../library/rows";
import { useL10n } from "../l10n";
import type { ScheduleState } from "./useSchedule";
import { TopicChip, useTopicLookup } from "../topics/TopicChip";
import type { TopicLookup } from "../generated/types";
import { attendeesLine, prepLine, relativeStart, rowAction } from "./todayModel";

/**
 * Today's schedule: the calendar events still ahead today, each with its time, invitees, a countdown and
 * Start (or Open once recorded), the next one with its prep line, then the meetings held earlier today.
 * It draws what `useSchedule` read; TodayView owns the reading, so the skinned layouts share it.
 */
export function ScheduleCard({
  state,
  title,
  briefOpen,
  onToggleBrief,
  onOpenMeeting,
}: {
  state: ScheduleState;
  /** The skin's heading for the day ("Today" in Whimbrel). */
  title: string;
  briefOpen: boolean;
  onToggleBrief: () => void;
  onOpenMeeting: (meetingId: number) => void;
}) {
  const { schedule, next, now, starting, error, start } = state;
  const { locale, t } = useL10n();
  // A topic chip per row (topics piece 2): a recorded event by its meeting, else by the event.
  const lookups: TopicLookup[] = [
    ...(schedule?.events ?? []).map((e): TopicLookup =>
      e.recordedMeetingId != null ? { kind: "meeting", id: e.recordedMeetingId } : { kind: "event", id: e.eventId, title: e.title },
    ),
    ...(schedule?.earlierToday ?? []).map((m): TopicLookup => ({ kind: "meeting", id: m.id })),
  ];
  const topics = useTopicLookup(lookups);
  const eventCount = schedule?.events.length ?? 0;

  if (!schedule) return error ? <p role="alert">{error}</p> : null;
  // No calendar, or nothing today: the card has nothing to say.
  if (schedule.events.length === 0 && schedule.earlierToday.length === 0) return error ? <p role="alert">{error}</p> : null;

  return (
    <section className="card schedule" aria-labelledby="schedule-heading">
      <h2 id="schedule-heading" className="section-label">
        {title}
      </h2>
      {schedule.events.length > 0 && (
        <ul className="schedule-rows">
          {schedule.events.map((e, i) => {
            const action = rowAction(e, schedule.recording);
            const prep = prepLine(e, next);
            const countdown = relativeStart(e.startUnixMs, e.endUnixMs, now);
            const invitees = attendeesLine(e.attendees);
            return (
              <li key={e.eventId} className="schedule-row">
                <time className="schedule-time">{timeLabel(e.startUnixMs, locale)}</time>
                <span className="schedule-main">
                  <span className="schedule-title">{e.title}</span>
                  {topics[i] && <TopicChip topic={topics[i]} />}
                  {invitees && <span className="muted small">{invitees}</span>}
                  {prep === "ready" && (
                    <span className="small">
                      {t("Prep ready")}{" "}
                      <button type="button" className="link" onClick={onToggleBrief} aria-expanded={briefOpen}>
                        {briefOpen ? t("Hide brief") : t("Open brief")}
                      </button>
                    </span>
                  )}
                  {prep === "none" && (
                    <span className="muted small" title={t("Meeting Prep makes a brief shortly before the meeting once it is turned on in Automations.")}>
                      {t("No prep yet")}
                    </span>
                  )}
                </span>
                {countdown && <span className="muted small schedule-countdown">{countdown}</span>}
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
              </li>
            );
          })}
        </ul>
      )}
      {schedule.earlierToday.length > 0 && (
        <>
          <h3 className="schedule-sub">{t("Earlier today")}</h3>
          <ul className="schedule-rows">
            {schedule.earlierToday.map((m, i) => (
              <li key={m.id} className="schedule-row">
                <time className="schedule-time">{timeLabel(m.tsUnixMs, locale)}</time>
                <span className="schedule-main">
                  <span className="schedule-title">{m.title}</span>
                  {topics[eventCount + i] && <TopicChip topic={topics[eventCount + i]!} />}
                </span>
                <button type="button" className="mode" onClick={() => onOpenMeeting(m.id)}>
                  {t("Open")}
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
