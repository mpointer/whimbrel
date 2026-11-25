import { useEffect, useState } from "react";
import { dismissBetterOffer, setMeetingMultilingual, setTranscriptionQuality } from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import type { QualityChoice } from "../../../../generated/types";
import { t } from "../../../../l10n";
import { WButton, breakAtSeparators, WSettingsCard, WSettingsControlRow, WSettingsNote, WSettingsProgressRow, WSettingsSelectRow, WSettingsStatusRow, WSettingsToggleRow } from "../../../../system";
import { meetingModelsDownload } from "../../../meetingModelsDownload";
import { needsTurboDownload, offerState, qualityChoices } from "../../../transcriptionQuality";
import { turboDownload } from "../../../turboDownload";
import { downloadValueText, modelsView } from "./meetingsRedesignModel";
import "./meetops.css";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Meeting models: quality, the multilingual switch, the downloads on progress rows (cancel for the meeting models),
 *  and the one offer of better transcripts. Same downloads, stores and commands as MeetingModelsCard and BetterTranscriptsCard. */
export function MeetingModelsCardRedesign() {
  const { status, job, error: commandError } = meetingModelsDownload.use();
  const { status: quality, job: turboJob } = turboDownload.use();
  const [toggleError, setToggleError] = useState<string | null>(null);
  const error = toggleError ?? commandError;

  useEffect(() => {
    void meetingModelsDownload.refresh();
    void turboDownload.refresh();
  }, []);

  if (!status) return <WSettingsCard id="meetings.models" title={t("Meeting models")} headingLevel={3} loading error={error} />;

  const view = modelsView(status, job);
  const setMultilingual = (on: boolean) => {
    setToggleError(null);
    meetingModelsDownload.setStatus({ ...status, multilingual: on });
    setMeetingMultilingual(on)
      .then(meetingModelsDownload.setStatus)
      .catch((e: unknown) => setToggleError(message(e)));
  };
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
  // The offer's Download: Automatic, then the large model, then the meeting models if they are missing.
  const acceptOffer = async () => {
    await setTranscriptionQuality("automatic").then(turboDownload.setStatus);
    await dismissBetterOffer().then(turboDownload.setStatus);
    await turboDownload.start();
    await meetingModelsDownload.refresh();
    if (meetingModelsDownload.get().status?.installed === false) await meetingModelsDownload.start();
  };
  const notNow = () => void dismissBetterOffer().then(turboDownload.setStatus, () => {});

  const choices = qualityChoices();
  const offer = offerState(quality, turboJob);

  return (
    <WSettingsCard id="meetings.models" title={t("Meeting models")} headingLevel={3} error={error}>
      {offer === "offer" ? (
        <WSettingsControlRow label={t("Better transcripts are available.")} description={t("Download a larger model, about 574 MB. Your audio stays on this PC. Downloading it sets Quality to Automatic.")}>
          <WButton variant="primary" onClick={() => void acceptOffer()}>
            {t("Download")}
          </WButton>
          <WButton onClick={notNow}>{t("Not now")}</WButton>
        </WSettingsControlRow>
      ) : null}
      {quality ? (
        <WSettingsSelectRow
          label={t("Quality")}
          description={choices.find((c) => c.id === quality.quality)?.detail}
          value={quality.quality}
          options={choices.map((c) => ({ value: c.id, label: c.label }))}
          onChange={(v) => setQuality(v as QualityChoice)}
        />
      ) : null}
      {turboJob && !turboJob.done ? (
        <WSettingsProgressRow label={t("Downloading the better transcription model…")} progress={turboJob.fraction} announce={turboJob.status} />
      ) : turboJob?.error ? (
        <WSettingsControlRow label={t("Couldn't download the better model. Try Again")}>
          <WButton onClick={() => void turboDownload.start()}>{t("Try again")}</WButton>
        </WSettingsControlRow>
      ) : null}
      <WSettingsToggleRow
        label={t("I have meetings in other languages")}
        description={t(
          "Whimbrel detects each meeting's language and transcribes it as spoken, using a larger multilingual model. Off, every meeting is transcribed as English.",
        )}
        checked={status.multilingual}
        disabled={view.state === "downloading"}
        disabledReason={t("A download is running. Try again when it finishes.")}
        onToggle={setMultilingual}
      />
      {view.state === "ready" ? <WSettingsStatusRow live label={t("Transcription and speaker models are installed.")} status={{ tone: "good", children: t("Installed") }} /> : null}
      {view.needText ? (
        <WSettingsControlRow label={view.state === "failed" ? t("Download failed") : t("Download needed")} description={view.needText} error={job?.error ?? null}>
          <WButton variant="primary" onClick={() => void meetingModelsDownload.start()}>
            {view.buttonLabel}
          </WButton>
        </WSettingsControlRow>
      ) : null}
      {view.state === "downloading" && job ? (
        <WSettingsProgressRow label={t("Meeting models")} progress={job.fraction} valueText={downloadValueText(job)} onCancel={meetingModelsDownload.cancel} cancelLabel={t("Cancel")} announce={job.status} />
      ) : null}
      <WSettingsNote>{breakAtSeparators(status.dir)}</WSettingsNote>
    </WSettingsCard>
  );
}
