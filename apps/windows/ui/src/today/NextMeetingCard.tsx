import { useCallback, useEffect, useState } from "react";
import { nextMeetingPrep } from "../api";
import { describeError, isCommandError } from "../errors";
import type { NextMeetingView } from "../generated/types";
import { dateTimeLabel } from "../library/rows";
import { useL10n } from "../l10n";
import { PrepBriefBody } from "./PrepBrief";
import { prepStatus } from "./todayModel";

/** "Next meeting": the next calendar meeting and its Meeting Prep brief, made here or on another device. */
import { TopicChip, useTopicLookup } from "../topics/TopicChip";

export function NextMeetingCard({ onOpenMeeting }: { onOpenMeeting: (meetingId: number) => void }) {
  const { locale, t } = useL10n();
  const [next, setNext] = useState<NextMeetingView | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The brief's topic (topics piece 2), above the title as on the Mac.
  const [topic] = useTopicLookup(next ? [{ kind: "event", id: next.eventId, title: next.title }] : []);

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

  return (
    <section className="card" aria-labelledby="next-meeting-heading">
      <h2 id="next-meeting-heading" className="section-label">
        {t("Next meeting")}
      </h2>
      {topic && <TopicChip topic={topic} />}
      <p>
        <strong>{next.title}</strong> <span className="muted small">{dateTimeLabel(next.startUnixMs, new Date(), locale)}</span>
      </p>
      {brief ? <PrepBriefBody brief={brief} onOpenMeeting={onOpenMeeting} /> : null}
      <p className="muted small">{prepStatus(brief, locale)}</p>
    </section>
  );
}
