import { useEffect, useState } from "react";
import { meetingRecordStart, meetingRecordStop, meetingRecordingStatus } from "../api";
import { describeError, isCommandError } from "../errors";
import type { MeetingRecordingView } from "../generated/types";
import { useL10n } from "../l10n";
import { recordAction, recordButtonDisabled, recordingCardShown } from "./recordingModel";
import { useNoStore } from "../launch/noStore";
import { RecordingOffCaption } from "../launch/NoStoreBanner";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** "Record now" (M3.3 Task 3): a manual meeting of everything this PC plays, plus the mic. The shell's recording bar shows the
 *  recording, so this card is drawn only while nothing records (`recordingCardShown`); the recording notice is drawn by the
 *  open meeting and the Meetings screen, never here. */
export function RecordingCard({ onChanged }: { onChanged?: () => void } = {}) {
  const [view, setView] = useState<MeetingRecordingView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { t } = useL10n();
  // Without the memory database recording is off (no-store mode); the shell refuses it too.
  const noStore = useNoStore();

  useEffect(() => {
    const load = () =>
      meetingRecordingStatus()
        .then(setView)
        .catch((e: unknown) => setError(message(e)));
    load();
    // The tray popup can stop a recording too, so the card keeps checking.
    const status = setInterval(load, 3_000);
    return () => clearInterval(status);
  }, []);

  const action = recordAction(view);
  const press = () => {
    setBusy(true);
    setError(null);
    (action === "stop" ? meetingRecordStop() : meetingRecordStart())
      .then((next) => {
        setView(next);
        onChanged?.();
      })
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  if (!recordingCardShown(view)) {
    return error ? <p role="alert">{error}</p> : null;
  }
  return (
    <section className="card rt" aria-labelledby="record-heading">
      <h2 id="record-heading" className="section-label">
        {t("Record a meeting")}
      </h2>
      <p className="muted">
        {t("Records everything this PC plays and your microphone. Audio stays on this PC.")}
      </p>
      {/* Windows has no first-run screen, so the consent note goes where a recording starts by hand (W-1.7). */}
      <p className="muted small">
        {t("Some places require everyone's consent before a meeting is recorded. Check the rules where you and the other people are.")}
      </p>
      {view?.degraded && <p role="alert">{view.degraded}</p>}
      {view?.processing && (
        <p role="status" className="muted">
          {/* The label is core's status line; W5 translates it. */}
          {t("{label} Its transcript and summary appear in Library → Recent when it's done.", { label: view.processing.label })}
        </p>
      )}
      {view?.failure && !view.processing && (
        <p role="alert">
          {t("The last meeting didn't finish processing: {message}", { message: view.failure.message })}
        </p>
      )}
      <div className="toolbar">
        <button className="mode mode--active" onClick={press} disabled={recordButtonDisabled(action, busy, noStore)}>
          {action === "stop" ? t("Stop recording") : t("Record now")}
        </button>
        {noStore && <RecordingOffCaption />}
      </div>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
