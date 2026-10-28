import { timeLabel } from "../library/rows";
import type { ScheduleState } from "./useSchedule";
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
          {schedule.events.map((e) => {
            const action = rowAction(e, schedule.recording);
            const prep = prepLine(e, next);
            const countdown = relativeStart(e.startUnixMs, e.endUnixMs, now);
            const invitees = attendeesLine(e.attendees);
            return (
              <li key={e.eventId} className="schedule-row">
                <time className="schedule-time">{timeLabel(e.startUnixMs)}</time>
                <span className="schedule-main">
                  <span className="schedule-title">{e.title}</span>
                  {invitees && <span className="muted small">{invitees}</span>}
                  {prep === "ready" && (
                    <span className="small">
                      Prep ready{" "}
                      <button type="button" className="link" onClick={onToggleBrief} aria-expanded={briefOpen}>
                        {briefOpen ? "Hide brief" : "Open brief"}
                      </button>
                    </span>
                  )}
                  {prep === "none" && (
                    <span className="muted small" title="Meeting Prep makes a brief shortly before the meeting once it is turned on in Automations.">
                      No prep yet
                    </span>
                  )}
                </span>
                {countdown && <span className="muted small schedule-countdown">{countdown}</span>}
                {action === "start" && (
                  <button type="button" className="mode" onClick={() => start(e.eventId)} disabled={starting !== null}>
                    {starting === e.eventId ? "Starting…" : "Start"}
                  </button>
                )}
                {action === "open" && (
                  <button type="button" className="mode" onClick={() => onOpenMeeting(e.recordedMeetingId as number)}>
                    Open
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {schedule.earlierToday.length > 0 && (
        <>
          <h3 className="schedule-sub">Earlier today</h3>
          <ul className="schedule-rows">
            {schedule.earlierToday.map((m) => (
              <li key={m.id} className="schedule-row">
                <time className="schedule-time">{timeLabel(m.tsUnixMs)}</time>
                <span className="schedule-main">
                  <span className="schedule-title">{m.title}</span>
                </span>
                <button type="button" className="mode" onClick={() => onOpenMeeting(m.id)}>
                  Open
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
