import { useEffect, useState } from "react";
import { modelDownloadCancel, modelDownloadPoll, modelDownloadStart, modelStatus } from "../api";
import { describeError, isCommandError } from "../errors";
import type { DownloadView, ModelStatus } from "../generated/types";
import { t } from "../l10n";
import { percent, searchModelState } from "./searchModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** The multilingual embedding model (plan Decision 12): downloaded on first run, not bundled. */
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
      .then((id) => setJob({ jobId: id, fraction: 0, status: t("Starting…"), done: false, cancelled: false }))
      .catch((e: unknown) => setError(message(e)));
  };

  if (!status) return <section className="card about">{error ? <p role="alert">{error}</p> : t("Loading…")}</section>;
  const state = searchModelState(status, job);

  return (
    <section className="card about" aria-labelledby="model-heading">
      <h2 id="model-heading" className="section-label">
        {t("Semantic search")}
      </h2>
      {state === "ready" && <p role="status">{t("The search model is ready. Library search can match by meaning, across languages.")}</p>}
      {(state === "missing" || state === "failed") && (
        <>
          <p>
            {t(
              "Searching by meaning needs a multilingual model ({model}, about 470 MB), downloaded once from Hugging Face. Keyword search works without it.",
              { model: "paraphrase-multilingual-MiniLM-L12-v2" },
            )}
          </p>
          {job?.error && <p role="alert">{job.error}</p>}
          <div className="toolbar">
            <button className="mode mode--active" onClick={start}>
              {state === "failed" ? t("Try again") : t("Download search model")}
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
              {t("Cancel")}
            </button>
          </div>
        </>
      )}
      <p className="muted small mono">{status.dir}</p>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
