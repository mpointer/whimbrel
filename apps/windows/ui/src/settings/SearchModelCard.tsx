import { useEffect, useState } from "react";
import { modelDownloadCancel, modelDownloadPoll, modelDownloadStart, modelStatus, restartApp } from "../api";
import { describeError, isCommandError } from "../errors";
import type { DownloadView, ModelStatus } from "../generated/types";
import { percent, searchModelState } from "./searchModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** The MiniLM embedding model (plan Decision 12): downloaded on first run, not bundled. */
export function SearchModelCard() {
  const [status, setStatus] = useState<ModelStatus | null>(null);
  const [job, setJob] = useState<DownloadView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    modelStatus()
      .then(setStatus)
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const jobId = job && !job.done ? job.jobId : null;
  useEffect(() => {
    if (!jobId) return;
    // 200 ms, the Mac shim's cadence.
    const timer = setInterval(() => {
      modelDownloadPoll(jobId)
        .then((v) => {
          setJob(v);
          if (v.done) modelStatus().then(setStatus);
        })
        // The shell releases a job once it reports done, so a poll already in flight
        // when that happens fails; that is not an error once the job has finished.
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
    modelDownloadStart()
      .then((id) => setJob({ jobId: id, fraction: 0, status: "Starting…", done: false, cancelled: false }))
      .catch((e: unknown) => setError(message(e)));
  };

  if (!status) return <section className="card about">{error ? <p role="alert">{error}</p> : "Loading…"}</section>;
  const state = searchModelState(status, job);

  return (
    <section className="card about" aria-labelledby="model-heading">
      <h2 id="model-heading" className="section-label">
        Semantic search
      </h2>
      {state === "ready" && <p role="status">The search model is loaded. Library search can match by meaning.</p>}
      {(state === "missing" || state === "failed") && (
        <>
          <p>
            Searching by meaning needs a small model (all-MiniLM-L6-v2, about 90 MB), downloaded once from Hugging
            Face. Keyword search works without it.
          </p>
          {job?.error && <p role="alert">{job.error}</p>}
          <div className="toolbar">
            <button className="mode mode--active" onClick={start}>
              {state === "failed" ? "Try again" : "Download search model"}
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
      {state === "restart" && (
        <>
          <p role="status">The search model is downloaded. Restart Whimbrel to start using it.</p>
          <div className="toolbar">
            <button className="mode mode--active" onClick={() => restartApp().catch((e: unknown) => setError(message(e)))}>
              Restart Whimbrel
            </button>
          </div>
        </>
      )}
      <p className="muted small mono">{status.dir}</p>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
