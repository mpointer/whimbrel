import { useEffect, useState } from "react";
import { setMeetingMultilingual, setTranscriptionQuality } from "../api";
import { describeError, isCommandError } from "../errors";
import { t } from "../l10n";
import type { QualityChoice } from "../generated/types";
import { TurboDownloadLine } from "./BetterTranscriptsCard";
import { downloadSizeMb, meetingModelsState, missingList, MULTILINGUAL_MODEL } from "./meetingModels";
import { meetingModelsDownload } from "./meetingModelsDownload";
import { percent } from "./searchModel";
import { needsTurboDownload, qualityChoices } from "./transcriptionQuality";
import { turboDownload } from "./turboDownload";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Whisper and the two diarization models (M3.3 Decision 11), downloaded once. Its download is
 *  shared with the launch download (`modelBootstrap.ts`). */
export function MeetingModelsCard() {
  const { status, job, error: commandError } = meetingModelsDownload.use();
  const { status: quality } = turboDownload.use();
  const [toggleError, setToggleError] = useState<string | null>(null);
  const error = toggleError ?? commandError;

  useEffect(() => {
    void meetingModelsDownload.refresh();
    void turboDownload.refresh();
  }, []);

  if (!status) return <section className="card about">{error ? <p role="alert">{error}</p> : t("Loading…")}</section>;
  const state = meetingModelsState(status, job);
  // The Mac keeps this toggle beside its model setup; turning it on adds the multilingual model.
  const setMultilingual = (on: boolean) => {
    setToggleError(null);
    meetingModelsDownload.setStatus({ ...status, multilingual: on });
    setMeetingMultilingual(on)
      .then(meetingModelsDownload.setStatus)
      .catch((e: unknown) => setToggleError(message(e)));
  };
  // "Meetings in other languages need…" only when that's why base is missing; Automatic needs it too (W-1.2).
  const multilingualMissing = status.multilingual && status.missing.includes(MULTILINGUAL_MODEL);
  // Better downloads large-v3-turbo; Automatic and other languages need base, which the list below offers.
  const setQuality = (choice: QualityChoice) => {
    setToggleError(null);
    setTranscriptionQuality(choice)
      .then((next) => {
        turboDownload.setStatus(next);
        if (needsTurboDownload(choice, next)) void turboDownload.start();
        return meetingModelsDownload.refresh();
      })
      .catch((e: unknown) => setToggleError(message(e)));
  };
  const choices = qualityChoices();

  return (
    <section className="card about" aria-labelledby="meeting-models-heading">
      <h2 id="meeting-models-heading" className="section-label">
        {t("Meeting models")}
      </h2>
      {quality && (
        <>
          <label className="field">
            <span>{t("Quality")}</span>
            <select className="input" value={quality.quality} onChange={(e) => setQuality(e.target.value as QualityChoice)}>
              {choices.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.label}
                </option>
              ))}
            </select>
          </label>
          <p className="muted small">{choices.find((c) => c.id === quality.quality)?.detail}</p>
          <TurboDownloadLine />
        </>
      )}
      <label className="toggle">
        <input
          type="checkbox"
          checked={status.multilingual}
          disabled={state === "downloading"}
          onChange={(e) => setMultilingual(e.target.checked)}
        />
        {t("I have meetings in other languages")}
      </label>
      <p className="muted small">
        {t(
          "Whimbrel detects each meeting's language and transcribes it as spoken, using a larger multilingual model. Off, every meeting is transcribed as English.",
        )}
      </p>
      {state === "ready" && <p role="status">{t("Transcription and speaker models are installed.")}</p>}
      {(state === "missing" || state === "failed") && (
        <>
          <p>
            {multilingualMissing
              ? t(
                  "Meetings in other languages need {models} (about {size} MB), downloaded once. Until then, meetings are transcribed as English. Audio never leaves this PC.",
                  { models: missingList(status.missing), size: downloadSizeMb(status.missing) },
                )
              : t(
                  "Recording meetings needs {models} (about {size} MB in all), downloaded once. Meetings are transcribed on this PC; audio never leaves it.",
                  { models: missingList(status.missing), size: downloadSizeMb(status.missing) },
                )}
          </p>
          {job?.error && <p role="alert">{job.error}</p>}
          <div className="toolbar">
            <button className="mode mode--active" onClick={() => void meetingModelsDownload.start()}>
              {state === "failed" ? t("Try again") : t("Download meeting models")}
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
            <button className="mode" onClick={meetingModelsDownload.cancel}>
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
