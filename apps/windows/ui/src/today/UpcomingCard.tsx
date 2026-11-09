import { useCallback, useEffect, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { todayUpcoming } from "../api";
import type { TopicLookup, UpcomingView } from "../generated/types";
import { timeLabel } from "../library/rows";
import { useL10n } from "../l10n";
import { TopicChip, useTopicLookup } from "../topics/TopicChip";
import { dayHeading, invitedLine, readExpanded, visibleDays, writeExpanded } from "./upcomingModel";

/** A backstop; the calendar's own rounds (`calendar-synced`) are what bring new meetings. */
const REFRESH_MS = 5 * 60_000;

/**
 * Today's Upcoming (docs/superpowers/specs/2026-11-08-today-upcoming-design.md), read once for every layout:
 * on mount, after each calendar round (at launch the meetings arrive only with the first one), on window
 * focus, and every 5 minutes.
 */
export function useUpcoming(): UpcomingView | null {
  const [view, setView] = useState<UpcomingView | null>(null);
  const load = useCallback(() => {
    todayUpcoming().then(
      (v) => setView(v && Array.isArray(v.days) ? v : { days: [], syncError: null }),
      () => {},
    );
  }, []);
  useEffect(() => {
    load();
    const timer = setInterval(load, REFRESH_MS);
    window.addEventListener("focus", load);
    const unlisten = listen("calendar-synced", load);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", load);
      void unlisten.then((stop) => stop());
    };
  }, [load]);
  return view;
}

/** Whether Upcoming has anything to say: meetings, or why there may be none. */
export const upcomingShows = (view: UpcomingView | null | undefined): boolean => !!view && (view.days.length > 0 || !!view.syncError);

/** The days and rows, for a layout to put in its own frame. */
export function UpcomingBody({ view }: { view: UpcomingView }) {
  const { t, locale } = useL10n();
  const [expanded, setExpanded] = useState(readExpanded);
  const { shown, more } = visibleDays(view.days, expanded);
  const events = shown.flatMap((d) => d.events);
  const topics = useTopicLookup(events.map((e): TopicLookup => ({ kind: "event", id: e.eventId, title: e.title })));
  const toggle = () => {
    writeExpanded(!expanded);
    setExpanded(!expanded);
  };
  let n = 0;
  return (
    <div className="upcoming-body">
      {view.syncError && (
        <div className="field-row" role="status">
          <span className="small">{t("Your calendar couldn't sync, so meetings may be missing: {error}", { error: view.syncError })}</span>
          <button type="button" className="mode" onClick={() => void emit("open-settings", "connectors")}>
            {t("Open Connectors")}
          </button>
        </div>
      )}
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
    </div>
  );
}

/** Standard's frame: a card below the schedule. Nothing when there's nothing to say. */
export function UpcomingCard({ view }: { view: UpcomingView | null }) {
  const { t } = useL10n();
  if (!view || !upcomingShows(view)) return null;
  return (
    <section className="card schedule upcoming" aria-labelledby="upcoming-heading">
      <h2 id="upcoming-heading" className="section-label">
        {t("Upcoming")}
      </h2>
      <UpcomingBody view={view} />
    </section>
  );
}
