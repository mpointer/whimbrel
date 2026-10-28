import type { DownloadView, ModelStatus } from "../generated/types";

export type SearchModelState = "ready" | "downloading" | "failed" | "restart" | "missing";

/**
 * Where the MiniLM download stands. The store caches a failed model load for the life
 * of the process, so a finished download still needs a restart before it's used.
 */
export function searchModelState(status: ModelStatus, job: DownloadView | null): SearchModelState {
  if (status.loaded) return "ready";
  if (job && !job.done) return "downloading";
  if (job?.error) return "failed";
  if (status.installed || (job?.done && !job.cancelled)) return "restart";
  return "missing";
}

export const percent = (fraction: number) => `${Math.round(Math.min(1, Math.max(0, fraction)) * 100)}%`;
