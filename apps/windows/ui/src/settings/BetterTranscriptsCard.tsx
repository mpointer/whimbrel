import { dismissBetterOffer, setTranscriptionQuality } from "../api";
import { t } from "../l10n";
import { meetingModelsDownload } from "./meetingModelsDownload";
import { percent } from "./searchModel";
import { offerState } from "./transcriptionQuality";
import { turboDownload } from "./turboDownload";

/** The one offer of better transcripts (the Mac's `BetterTranscriptsCard`, W-1.2), in Library.
 *  Dismissing it, or accepting, ends it for good. */
export function BetterTranscriptsCard() {
  const { status, job } = turboDownload.use();
  const state = offerState(status, job);
  if (state === "hidden") return null;

  // Automatic, as on the Mac. It needs base too, as the floor model and for the live transcript
  // below core's live floor, so the meeting models download next if it's missing.
  const accept = async () => {
    await setTranscriptionQuality("automatic").then(turboDownload.setStatus);
    await dismissBetterOffer().then(turboDownload.setStatus);
    await turboDownload.start();
    await meetingModelsDownload.refresh();
    if (meetingModelsDownload.get().status?.installed === false) await meetingModelsDownload.start();
  };
  const notNow = () => void dismissBetterOffer().then(turboDownload.setStatus, () => {});

  return (
    <section className="card notice" aria-label={t("Better transcripts are available.")}>
      {state === "offer" ? (
        <>
          <h2 className="section-label">
            {t("Better transcripts are available.")}
          </h2>
          <p>{t("Download a larger model, about 574 MB. Your audio stays on this PC.")}</p>
          <div className="toolbar">
            <button className="mode mode--active" onClick={() => void accept()}>
              {t("Download")}
            </button>
            <button className="mode" onClick={notNow}>
              {t("Not now")}
            </button>
          </div>
        </>
      ) : (
        <TurboDownloadLine />
      )}
    </section>
  );
}

/** large-v3-turbo's download, wherever it was started. Nothing when none runs or failed. */
export function TurboDownloadLine() {
  const { job } = turboDownload.use();
  if (job && !job.done) {
    return (
      <div role="status">
        <p>
          {t("Downloading the better transcription model…")} {percent(job.fraction)}
        </p>
        <progress className="progress" value={job.fraction} max={1} />
      </div>
    );
  }
  if (job?.error) {
    return (
      <button className="link" onClick={() => void turboDownload.start()}>
        {t("Couldn't download the better model. Try Again")}
      </button>
    );
  }
  return null;
}
