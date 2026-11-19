// Where the live screen takes over (redesign flag on only): Home's place while a meeting records, and the Meetings tab's
// pane for the meeting that is live. Each is a small component of its own, so the sections it sits in keep their hooks.

import { useEffect, useState, type ReactNode } from "react";
import { meetingWorkspace } from "../../api";
import { useRecording } from "../../shell/redesign/useRecording";
import { LiveMeeting } from "./LiveMeeting";
import { liveScreenActive } from "./liveModel";
import { useRecordingContext } from "./recordingContext";

/** Home, or the live screen while a meeting starts, records, pauses, resumes or is being processed (the stage shows on the
 *  screen's bar until it finishes, then Home comes back). */
export function LiveOrHome({ home, onOpenSettings }: { home: ReactNode; onOpenSettings?: () => void }) {
  const shared = useRecordingContext();
  const own = useRecording(!shared);
  const rec = shared ?? own;
  const [wasLive, setWasLive] = useState(false);
  const live = liveScreenActive(rec.state);
  const processing = rec.state === "processing";
  useEffect(() => {
    if (live) setWasLive(true);
    else if (!processing) setWasLive(false);
  }, [live, processing]);
  if (live || (processing && wasLive)) {
    return <LiveMeeting meetingId={rec.view?.meetingId ?? rec.view?.processing?.meetingId ?? null} onOpenSettings={onOpenSettings} />;
  }
  return <>{home}</>;
}

/** How often the pane re-asks whether the selected meeting is live: a recording starting or ending changes its phase. */
const PHASE_POLL_MS = 5_000;

/** The Meetings tab's pane for the selected meeting: the live screen while that meeting is in progress (this PC's, or read
 *  only for another device's), else the old workspace passed in as `children`. */
export function MeetingPane({ meetingId, children, onOpenSettings }: { meetingId: number; children: ReactNode; onOpenSettings?: () => void }) {
  const [phase, setPhase] = useState<{ id: number; during: boolean; readOnly: boolean } | null>(null);
  useEffect(() => {
    let alive = true;
    const look = () =>
      meetingWorkspace(meetingId).then(
        (view) => alive && setPhase({ id: meetingId, during: view?.phase === "during", readOnly: view?.readOnly === true }),
        () => {},
      );
    look();
    const timer = setInterval(look, PHASE_POLL_MS);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [meetingId]);
  if (phase?.id === meetingId && phase.during) return <LiveMeeting meetingId={meetingId} readOnly={phase.readOnly} onOpenSettings={onOpenSettings} />;
  return <>{children}</>;
}
