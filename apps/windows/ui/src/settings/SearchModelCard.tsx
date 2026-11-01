import { useEffect } from "react";
import { t } from "../l10n";
import { percent, searchModelState } from "./searchModel";
import { searchModelDownload } from "./searchModelDownload";

/** The multilingual embedding model (plan Decision 12): downloaded on first run, not bundled. Its
 *  download is shared with the launch download (`modelBootstrap.ts`). */
export function SearchModelCard() {
  const { status, job, error } = searchModelDownload.use();

  // Whether it has loaded since the last read (it loads on the first search after the download).
  useEffect(() => {
    void searchModelDownload.refresh();
  }, []);

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
            <button className="mode mode--active" onClick={() => void searchModelDownload.start()}>
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
            <button className="mode" onClick={searchModelDownload.cancel}>
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
