import { useCallback, useEffect, useState } from "react";
import { todayUpcoming } from "../api";
import type { TopicLookup, UpcomingDay } from "../generated/types";
import { timeLabel } from "../library/rows";
import { useL10n } from "../l10n";
import { TopicChip, useTopicLookup } from "../topics/TopicChip";
import { dayHeading, invitedLine, readExpanded, visibleDays, writeExpanded } from "./upcomingModel";

/** The calendar is polled every 5 minutes; Upcoming reads it as often, and on window focus. */
const REFRESH_MS = 5 * 60_000;

/**
 * Today's Upcoming (docs/superpowers/specs/2026-11-08-today-upcoming-design.md): the next 7 days'
 * meetings, grouped by day, with their topics. Nothing shows when the week is empty.
 */
export function UpcomingCard() {
  const { t, locale } = useL10n();
  const [days, setDays] = useState<UpcomingDay[]>([]);
  const [expanded, setExpanded] = useState(readExpanded);

  const load = useCallback(() => {
    todayUpcoming().then((d) => setDays(Array.isArray(d) ? d : []), () => {});
  }, []);
  useEffect(() => {
    load();
    const timer = setInterval(load, REFRESH_MS);
    window.addEventListener("focus", load);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", load);
    };
  }, [load]);

  const { shown, more } = visibleDays(days, expanded);
  const events = shown.flatMap((d) => d.events);
  const topics = useTopicLookup(events.map((e): TopicLookup => ({ kind: "event", id: e.eventId, title: e.title })));
  if (days.length === 0) return null;

  const toggle = () => {
    writeExpanded(!expanded);
    setExpanded(!expanded);
  };
  let n = 0;
  return (
    <section className="card schedule upcoming" aria-labelledby="upcoming-heading">
      <h2 id="upcoming-heading" className="section-label">
        {t("Upcoming")}
      </h2>
      {shown.map((d) => (
        <div key={d.date}>
          <h3 className="schedule-sub">{dayHeading(d, locale)}</h3>
          <ul className="schedule-rows">
            {d.events.map((e) => {
              const topic = topics[n++];
              const invited = invitedLine(e.attendees);
              return (
                <li key={e.eventId} className="schedule-row">
                  <time className="schedule-time">{timeLabel(e.startUnixMs, locale)}</time>
                  <span className="schedule-main">
                    <span className="schedule-title">{e.title}</span>
                    {topic && <TopicChip topic={topic} />}
                    {invited && <span className="muted small">{invited}</span>}
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
      {more && (
        <button type="button" className="link" onClick={toggle} aria-expanded={expanded}>
          {expanded ? t("Show less") : t("Show the rest of the week")}
        </button>
      )}
    </section>
  );
}
