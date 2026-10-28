import { useEffect, useState } from "react";
import { meetingRecordStart, meetingRecordStop, meetingRecordingStatus } from "../api";
import { describeError, isCommandError } from "../errors";
import type { MeetingRecordingView } from "../generated/types";
import { elapsedLabel, recordAction } from "./recordingModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** "Record now" (M3.3 Task 3): a manual meeting of everything this PC plays, plus the mic. */
export function RecordingCard({ onChanged }: { onChanged?: () => void } = {}) {
  const [view, setView] = useState<MeetingRecordingView | null>(null);
  const [now, setNow] = useState(Date.now());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () =>
      meetingRecordingStatus()
        .then(setView)
        .catch((e: unknown) => setError(message(e)));
    load();
    // The tray popup can stop a recording too, so the card keeps checking.
    const status = setInterval(load, 3_000);
    const clock = setInterval(() => setNow(Date.now()), 1_000);
    return () => {
      clearInterval(status);
      clearInterval(clock);
    };
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

  const recording = view?.state === "recording" && view.startedUnixMs != null;
  return (
    <section className="card" aria-labelledby="record-heading">
      <h2 id="record-heading" className="section-label">
        Record a meeting
      </h2>
      {recording ? (
        <p role="status" className="recording-line">
          <span className="recording-dot" aria-hidden="true" /> Recording · {elapsedLabel(view.startedUnixMs!, now)}
        </p>
      ) : (
        <p className="muted">
          Records everything this PC plays and your microphone. Audio stays on this PC.
        </p>
      )}
      {view?.degraded && <p role="alert">{view.degraded}</p>}
      {view?.processing && (
        <p role="status" className="muted">
          {view.processing.label} Its transcript and summary appear in Library → Recent when it's done.
        </p>
      )}
      {view?.failure && !view.processing && (
        <p role="alert">
          The last meeting didn't finish processing: {view.failure.message}
        </p>
      )}
      <div className="toolbar">
        <button className="mode mode--active" onClick={press} disabled={busy || action === "wait"}>
          {action === "stop" ? "Stop recording" : "Record now"}
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
