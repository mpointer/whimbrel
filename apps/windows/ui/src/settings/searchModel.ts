import type { DownloadView, ModelStatus } from "../generated/types";

export type SearchModelState = "ready" | "downloading" | "failed" | "missing";

/**
 * Where the search model's download stands. The store doesn't remember a missing model, so
 * once the files are on disk the next search or embed loads it: downloaded is ready, no restart.
 */
export function searchModelState(status: ModelStatus, job: DownloadView | null): SearchModelState {
  if (status.loaded) return "ready";
  if (job && !job.done) return "downloading";
  if (job?.error) return "failed";
  if (status.installed || (job?.done && !job.cancelled)) return "ready";
  return "missing";
}

export const percent = (fraction: number) => `${Math.round(Math.min(1, Math.max(0, fraction)) * 100)}%`;
