// The wording and state mapping for the redesigned Meetings section. The old model files (meetingModels.ts,
// transcriptionQuality.ts) stay the source of the state; these functions only build the redesigned lines.
import type { DownloadView, MeetingModelsStatus } from "../../../../generated/types";
import { t } from "../../../../l10n";
import { downloadSizeMb, meetingModelsState, missingList, MULTILINGUAL_MODEL, type MeetingModelsState } from "../../../meetingModels";

export interface ModelsView {
  state: MeetingModelsState;
  /** The sentence above the Download button while the models are missing or a download failed. */
  needText: string | null;
  /** The button on that row. */
  buttonLabel: string | null;
}

/** Where the models stand and what to say about it; the same conditions as the old Meeting models card. */
export function modelsView(status: MeetingModelsStatus, job: DownloadView | null): ModelsView {
  const state = meetingModelsState(status, job);
  if (state !== "missing" && state !== "failed") return { state, needText: null, buttonLabel: null };
  const multilingualMissing = status.multilingual && status.missing.includes(MULTILINGUAL_MODEL);
  const args = { models: missingList(status.missing), size: downloadSizeMb(status.missing) };
  const needText = multilingualMissing
    ? t(
        "Meetings in other languages need {models} (about {size} MB), downloaded once. Until then, meetings are transcribed as English. Audio stays on this PC.",
        args,
      )
    : t("Recording meetings needs {models} (about {size} MB in all), downloaded once. Meetings are transcribed on this PC. Audio stays on this PC.", args);
  return { state, needText, buttonLabel: state === "failed" ? t("Try again") : t("Download meeting models") };
}

/** "Downloading… 40%" for the progress row's value. */
export function downloadValueText(job: Pick<DownloadView, "status" | "fraction">): string {
  return `${job.status} ${Math.round(Math.min(1, Math.max(0, job.fraction)) * 100)}%`;
}
