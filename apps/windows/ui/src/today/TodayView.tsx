import { useCallback, useEffect, useRef, useState } from "react";
import { meetingList, meetingMostRelevant, todayOverview, todayRecap } from "../api";
import { describeError, isCommandError } from "../errors";
import type { MeetingRow, TodayOverview, TodayRecap } from "../generated/types";
import { Detail, type Selection } from "../library/Detail";
import { PrepBriefPanel } from "./PrepBrief";
import { useUpcoming } from "./UpcomingCard";
import { earlierMeetings } from "./layoutModel";
import { useSchedule } from "./useSchedule";
import { RedesignedHome, type HomeSection } from "./redesign/HomeView";
import { LiveOrHome } from "../meetings/redesign/LiveGate";
import { TodayAnswer } from "../conversations/TodayAnswer";
import { recapFor, shouldGenerate } from "./todayModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** The Today tab (plan Task 9). Refreshes when shown and on window focus; no timer. */
export function TodayView({
  onOpenSettings,
  onOpenSection,
}: {
  onOpenSettings: () => void;
  /** The Home's links to other sections. */
  onOpenSection?: (target: HomeSection) => void;
}) {
  const [overview, setOverview] = useState<TodayOverview | null>(null);
  const [recap, setRecap] = useState<TodayRecap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [recapError, setRecapError] = useState<string | null>(null);
  const [writing, setWriting] = useState(false);
  const [selected, setSelected] = useState<Selection | null>(null);
  // Meetings from earlier days, for the Home's Earlier section.
  const [earlier, setEarlier] = useState<MeetingRow[]>([]);
  // Any other brief, opened from a card's Prep chip (or an upcoming card), by its key.
  const [prepKey, setPrepKey] = useState<string | null>(null);
  /// Which meeting Today shows: the shell picks it (recording, else today's next, else latest).
  const [relevantMeeting, setRelevantMeeting] = useState<number | null>(null);
  // A recap holds the store for a model call, so never two at once, and at most one
  // automatic attempt per day (see shouldGenerate).
  const inFlight = useRef(false);
  const attemptedDay = useRef<string | null>(null);

  const loadOverview = useCallback(() => {
    meetingMostRelevant()
      .then(setRelevantMeeting)
      .catch(() => {});
    meetingList("")
      .then((rows) => setEarlier(earlierMeetings(rows, Date.now())))
      .catch(() => setEarlier([]));
    todayOverview()
      .then((o) => {
        setOverview(o);
        setError(null);
        setRecap((current) => recapFor(o, current));
      })
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const generate = useCallback((refresh: boolean) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setWriting(true);
    setRecapError(null);
    todayRecap(refresh)
      .then(setRecap)
      .catch((e: unknown) => setRecapError(message(e)))
      .finally(() => {
        inFlight.current = false;
        setWriting(false);
      });
  }, []);

  const sched = useSchedule(loadOverview);
  const upcoming = useUpcoming();

  useEffect(() => {
    loadOverview();
    // Focus re-reads the overview only; it never regenerates the recap on its own.
    window.addEventListener("focus", loadOverview);
    return () => window.removeEventListener("focus", loadOverview);
  }, [loadOverview]);

  useEffect(() => {
    if (overview && shouldGenerate(overview, attemptedDay.current, recap) && !inFlight.current) {
      attemptedDay.current = overview.day;
      generate(false);
    }
  }, [overview, recap, generate]);

  if (selected) return <Detail selection={selected} onBack={() => setSelected(null)} />;
  return (
    <>
      {/* Ask on Today (screen 6): the answer shows under the shell's bar, above Home. */}
      <TodayAnswer onOpen={(open) => setSelected({ kind: open.kind, id: open.id, line: open.line })} />
      <LiveOrHome
        onOpenSettings={onOpenSettings}
        home={
          <RedesignedHome
            overview={overview}
            error={error}
            onRetry={loadOverview}
            earlier={earlier}
            sched={sched}
            upcoming={upcoming}
            relevantMeeting={relevantMeeting}
            recap={recap}
            writing={writing}
            recapError={recapError}
            onRefreshRecap={() => generate(true)}
            onOpenMeeting={(id) => setSelected({ kind: "meeting", id })}
            onOpenPrep={setPrepKey}
            prep={prepKey ? <PrepBriefPanel prepKey={prepKey} onClose={() => setPrepKey(null)} onOpenMeeting={(id) => setSelected({ kind: "meeting", id })} /> : null}
            onOpenSection={(target) => onOpenSection?.(target)}
          />
        }
      />
    </>
  );
}
