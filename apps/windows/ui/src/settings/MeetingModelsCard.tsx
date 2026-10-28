import { useEffect, useState } from "react";
import { meetingModelsDownloadStart, meetingModelsStatus, modelDownloadCancel, modelDownloadPoll } from "../api";
import { describeError, isCommandError } from "../errors";
import type { DownloadView, MeetingModelsStatus } from "../generated/types";
import { meetingModelsState, missingList } from "./meetingModels";
import { percent } from "./searchModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Whisper and the two diarization models (M3.3 Decision 11), downloaded once. */
export function MeetingModelsCard() {
  const [status, setStatus] = useState<MeetingModelsStatus | null>(null);
  const [job, setJob] = useState<DownloadView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    meetingModelsStatus()
      .then(setStatus)
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const jobId = job && !job.done ? job.jobId : null;
  useEffect(() => {
    if (!jobId) return;
    const timer = setInterval(() => {
      modelDownloadPoll(jobId)
        .then((v) => {
          setJob(v);
          if (v.done) meetingModelsStatus().then(setStatus);
        })
        // A poll in flight when the shell releases the finished job fails harmlessly.
        .catch((e: unknown) =>
          setJob((current) => {
            if (current?.done) return current;
            setError(message(e));
            return null;
          }),
        );
    }, 200);
    return () => clearInterval(timer);
  }, [jobId]);

  const start = () => {
    setError(null);
    meetingModelsDownloadStart()
      .then((id) => setJob({ jobId: id, fraction: 0, status: "Starting…", done: false, cancelled: false }))
      .catch((e: unknown) => setError(message(e)));
  };

  if (!status) return <section className="card about">{error ? <p role="alert">{error}</p> : "Loading…"}</section>;
  const state = meetingModelsState(status, job);

  return (
    <section className="card about" aria-labelledby="meeting-models-heading">
      <h2 id="meeting-models-heading" className="section-label">
        Meeting models
      </h2>
      {state === "ready" && <p role="status">Transcription and speaker models are installed.</p>}
      {(state === "missing" || state === "failed") && (
        <>
          <p>
            Recording meetings needs {missingList(status.missing)} (about 110 MB in all), downloaded once. Meetings are
            transcribed on this PC; audio never leaves it.
          </p>
          {job?.error && <p role="alert">{job.error}</p>}
          <div className="toolbar">
            <button className="mode mode--active" onClick={start}>
              {state === "failed" ? "Try again" : "Download meeting models"}
            </button>
          </div>
        </>
      )}
      {state === "downloading" && job && (
        <>
          <p role="status">
            {job.status} {percent(job.fraction)}
          </p>
          <progress className="progress" value={job.fraction} max={1} />
          <div className="toolbar">
            <button className="mode" onClick={() => modelDownloadCancel(job.jobId)}>
              Cancel
            </button>
          </div>
        </>
      )}
      <p className="muted small mono">{status.dir}</p>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
