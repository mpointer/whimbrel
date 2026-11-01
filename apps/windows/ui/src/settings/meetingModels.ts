import type { DownloadView, MeetingModelsStatus } from "../generated/types";

export type MeetingModelsState = "ready" | "downloading" | "failed" | "missing";

/** Where the meeting models stand. Unlike the search model, they load per meeting, so no restart. */
export function meetingModelsState(status: MeetingModelsStatus, job: DownloadView | null): MeetingModelsState {
  if (job && !job.done) return "downloading";
  if (job?.error) return "failed";
  if (status.installed) return "ready";
  return "missing";
}

/** core's name for the multilingual whisper model (`api::models::manifest`). */
export const MULTILINGUAL_MODEL = "Multilingual transcription model";

/** Roughly what the missing files weigh: about 150 MB for the multilingual model (ggml-base),
 *  about 110 MB for the other three together. */
export function downloadSizeMb(missing: string[]): number {
  const multilingual = missing.includes(MULTILINGUAL_MODEL) ? 150 : 0;
  return multilingual + (missing.some((m) => m !== MULTILINGUAL_MODEL) ? 110 : 0);
}

/** "the transcription model and the speaker segmentation model", for the missing list. */
export function missingList(missing: string[]): string {
  const names = missing.map((m) => `the ${m.charAt(0).toLowerCase()}${m.slice(1)}`);
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}
