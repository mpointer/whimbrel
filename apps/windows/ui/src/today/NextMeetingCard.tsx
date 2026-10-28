import { useCallback, useEffect, useState } from "react";
import { nextMeetingPrep } from "../api";
import { describeError, isCommandError } from "../errors";
import type { NextMeetingView, PrepBriefView } from "../generated/types";
import { dateTimeLabel } from "../library/rows";
import { citedSources, prepStatus } from "./todayModel";

/** One chip per source a claim cites. A meeting this PC has opens it; any other source is a plain label. */
function SourceChips({ brief, ids, onOpenMeeting }: { brief: PrepBriefView; ids: string[]; onOpenMeeting: (id: number) => void }) {
  const cited = citedSources(brief, ids);
  if (cited.length === 0) return null;
  return (
    <span className="source-chips">
      {cited.map((s) =>
        s.meetingId != null ? (
          <button key={s.id} type="button" className="chip source-chip" onClick={() => onOpenMeeting(s.meetingId as number)}>
            {s.title}
          </button>
        ) : (
          <span key={s.id} className="chip source-chip source-chip--label">
            {s.title}
          </span>
        ),
      )}
    </span>
  );
}

/** "Next meeting": the next calendar meeting and its Meeting Prep brief, made here or on another device. */
export function NextMeetingCard({ onOpenMeeting }: { onOpenMeeting: (meetingId: number) => void }) {
  const [next, setNext] = useState<NextMeetingView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    nextMeetingPrep()
      .then((n) => {
        setNext(n);
        setError(null);
      })
      .catch((e: unknown) => setError(isCommandError(e) ? describeError(e) : String(e)));
  }, []);

  useEffect(() => {
    load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, [load]);

  // No calendar connected, or nothing coming up: the card has nothing to say.
  if (!next && !error) return null;
  if (error) return <p role="alert">{error}</p>;
  if (!next) return null;
  const brief = next.brief;
  const sources = (ids: string[]) => (brief ? <SourceChips brief={brief} ids={ids} onOpenMeeting={onOpenMeeting} /> : null);

  return (
    <section className="card" aria-labelledby="next-meeting-heading">
      <h2 id="next-meeting-heading" className="section-label">
        Next meeting
      </h2>
      <p>
        <strong>{next.title}</strong> <span className="muted small">{dateTimeLabel(next.startUnixMs)}</span>
      </p>
      {brief ? (
        <div className="stack">
          <p>
            {brief.summary}
            {sources(brief.summarySources)}
          </p>
          {brief.attendees.length > 0 && (
            <ul className="rows">
              {brief.attendees.map((a) => (
                <li key={a.who} className="row-card static">
                  <span className="row-title">{a.who}</span>
                  <span className="row-snippet">{a.note}</span>
                  {sources(a.sources)}
                </li>
              ))}
            </ul>
          )}
          {brief.openItems.length > 0 && (
            <>
              <h3 className="section-label">Open items</h3>
              <ul>
                {brief.openItems.map((c) => (
                  <li key={c.text}>
                    {c.text}
                    {sources(c.sources)}
                  </li>
                ))}
              </ul>
            </>
          )}
          {brief.openQuestions.length > 0 && (
            <>
              <h3 className="section-label">Open questions</h3>
              <ul>
                {brief.openQuestions.map((c) => (
                  <li key={c.text}>
                    {c.text}
                    {sources(c.sources)}
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      ) : null}
      <p className="muted small">{prepStatus(brief)}</p>
    </section>
  );
}
