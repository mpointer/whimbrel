// Where the live screen takes over (redesign flag on only): Home's place while a meeting records, and the Meetings tab's
// pane for the meeting that is live. Each is a small component of its own, so the sections it sits in keep their hooks.

import { useEffect, useState, type ReactNode } from "react";
import { meetingWorkspace } from "../../api";
import { t } from "../../l10n";
import { useRecording } from "../../shell/redesign/useRecording";
import { WButton } from "../../system";
import { FinishedNote } from "./FinishedNote";
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

type Phase = { id: number; kind: "during" | "after" | "other" | "error"; readOnly: boolean };

/** The pane for the selected meeting under the redesign: the live screen while that meeting is in progress (this PC's, or read
 *  only for another device's), the finished note once it is done, else the old workspace passed in as `children`. `onBack` is for
 *  a meeting opened on its own (Home, Library, Ask, Topics): the finished note draws it in its header, every other phase above
 *  the old pane. `onChanged` is told when the meeting changed or was forgotten. */
export function MeetingPane({
  meetingId,
  children,
  onOpenSettings,
  onBack,
  onChanged,
  onFullWidth,
  initialLine,
}: {
  meetingId: number;
  /** Open the finished note's transcript at this line (a search result or an Ask source). */
  initialLine?: number;
  children: ReactNode;
  onOpenSettings?: () => void;
  onBack?: () => void;
  onChanged?: () => void;
  /** Told whether the finished note is showing, which wants the whole width (the Meetings tab then hides its list). */
  onFullWidth?: (full: boolean) => void;
}) {
  const [phase, setPhase] = useState<Phase | null>(null);
  // While the finished note is open it reads the workspace on its own clock and reports the phase here, so this pane asks only
  // until the meeting is known to be finished (and again if the note reports it is not).
  const settled = phase?.id === meetingId && phase.kind === "after";
  useEffect(() => {
    if (settled) return;
    let alive = true;
    const look = () =>
      meetingWorkspace(meetingId).then(
        (view) => alive && setPhase({ id: meetingId, kind: view?.phase === "during" ? "during" : view?.phase === "after" ? "after" : "other", readOnly: view?.readOnly === true }),
        // The old pane shows what went wrong.
        () => alive && setPhase((p) => p ?? { id: meetingId, kind: "error", readOnly: false }),
      );
    look();
    const timer = setInterval(look, PHASE_POLL_MS);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [meetingId, settled]);
  const known = phase?.id === meetingId ? phase : null;
  const full = known ? known.kind === "after" : null;
  useEffect(() => {
    if (full != null) onFullWidth?.(full);
    // The callback is a state setter; only the answer matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [full]);
  useEffect(() => () => onFullWidth?.(false), []); // eslint-disable-line react-hooks/exhaustive-deps
  if (known?.kind === "after") {
    return (
      <FinishedNote
        meetingId={meetingId}
        initialLine={initialLine}
        onBack={onBack}
        onChanged={onChanged ?? (() => {})}
        onOpenSettings={onOpenSettings}
        onPhase={(p, readOnly) => setPhase({ id: meetingId, kind: p === "during" ? "during" : p === "after" ? "after" : "other", readOnly })}
      />
    );
  }
  const back = onBack ? (
    <WButton variant="quiet" onClick={onBack}>
      {t("← Back")}
    </WButton>
  ) : null;
  if (known?.kind === "during") {
    return (
      <>
        {back}
        <LiveMeeting meetingId={meetingId} readOnly={known.readOnly} onOpenSettings={onOpenSettings} />
      </>
    );
  }
  // Until the first answer, nothing of the old pane is drawn, so a finished meeting never flashes it.
  if (!known) return <p className="th-fin-quiet">{t("Loading…")}</p>;
  return (
    <>
      {back}
      {children}
    </>
  );
}
