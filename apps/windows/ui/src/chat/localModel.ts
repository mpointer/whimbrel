// The small local model behind Ask with nothing set up (W-1.3, the Mac's `LocalModelCard`). Its
// download is shared, so the Ask card and Settings → AI agree.

import { localModelDownloadStart, localModelStatus } from "../api";
import type { DownloadView, LocalModelStatus } from "../generated/types";
import { downloadStore } from "../settings/downloadStore";

export const localModelDownload = downloadStore<LocalModelStatus>(localModelStatus, localModelDownloadStart);

export type CardState = "hidden" | "offer" | "setup-only" | "downloading" | "failed";

/** What the empty Ask screen shows. `onRamp`: nothing else is set up (`showsOnRamp`). The download
 *  button needs a PC that can run the model; without it only the way to Settings shows. */
export function cardState(onRamp: boolean, status: LocalModelStatus | null, job: DownloadView | null): CardState {
  if (!onRamp || !status || status.installed) return "hidden";
  if (job && !job.done) return "downloading";
  if (!status.canOffer) return "setup-only";
  return job?.error ? "failed" : "offer";
}
