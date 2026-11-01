import type { DownloadView, QualityChoice, TranscriptionQualityStatus } from "../generated/types";
import { t } from "../l10n";

/** What Library shows about better transcripts: the one offer, the download's progress or failure, or nothing. */
export type OfferState = "hidden" | "offer" | "downloading" | "failed";

/** The offer only while Rust says to make it; a download running or failed shows in its place, wherever it started. */
export function offerState(status: TranscriptionQualityStatus | null, job: DownloadView | null): OfferState {
  if (job && !job.done) return "downloading";
  if (job?.error) return "failed";
  return status?.offer ? "offer" : "hidden";
}

/** The picker's choices, in the Mac's order; a function, so the labels follow the language. */
export function qualityChoices(): { id: QualityChoice; label: string; detail: string }[] {
  return [
    { id: "automatic", label: t("Automatic"), detail: t("The best engine this computer runs well.") },
    { id: "better", label: t("Better"), detail: t("The most accurate model, a download of about 574 MB. Slower on older computers.") },
    { id: "fast", label: t("Fast"), detail: t("The small model. Quickest, least accurate.") },
  ];
}

/** Better needs large-v3-turbo on disk; choosing it starts the download. */
export const needsTurboDownload = (quality: QualityChoice, status: TranscriptionQualityStatus) => quality === "better" && !status.turboInstalled;
