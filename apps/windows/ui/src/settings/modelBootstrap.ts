// Release 1 item 1.1 (docs/superpowers/plans/2026-11-01-windows-setup-in-one-screen.md): every missing
// model downloads in the background at launch, with no prompt, as the Mac's `ModelBootstrap` does.
// The meeting models go first (smaller, and needed by the first meeting), then the search model;
// keyword search works meanwhile. A failure waits for "Try again" or the next launch.

import { useSyncExternalStore } from "react";
import type { DownloadView, MeetingModelsStatus, ModelStatus } from "../generated/types";
import type { DownloadStore } from "./downloadStore";
import { meetingModelsDownload } from "./meetingModelsDownload";
import { searchModelDownload } from "./searchModelDownload";

/** One download in the chain: its store, and whether its status says something is missing. */
export interface Step {
  store: Pick<DownloadStore<unknown>, "refresh" | "get" | "start">;
  missing: (status: never) => boolean;
}

export type Outcome = "ready" | "cancelled" | "failed";

/** Runs each step in turn, stopping at the first that fails or is cancelled. A step whose status
 *  can't be read is skipped and stops the chain quietly: there is nothing to say about it. */
export async function runSteps(steps: Step[]): Promise<Outcome> {
  for (const step of steps) {
    await step.store.refresh();
    const status = step.store.get().status;
    if (status === null) return "cancelled";
    if (!step.missing(status as never)) continue;
    const end = await step.store.start();
    if (end?.cancelled) return "cancelled";
    if (!end || end.error) return "failed";
  }
  return "ready";
}

export type Footer = { kind: "hidden" } | { kind: "downloading"; fraction: number } | { kind: "failed" };

/** The sidebar footer: progress while either download runs (from here or from Settings), else
 *  the launch download's failure, else nothing. A cancel shows nothing. */
export function footer(meetingJob: DownloadView | null, searchJob: DownloadView | null, failed: boolean): Footer {
  const running = [meetingJob, searchJob].find((job) => job && !job.done);
  if (running) return { kind: "downloading", fraction: running.fraction };
  return failed ? { kind: "failed" } : { kind: "hidden" };
}

const STEPS: Step[] = [
  { store: meetingModelsDownload, missing: (s: MeetingModelsStatus) => !s.installed },
  { store: searchModelDownload, missing: (s: ModelStatus) => !s.installed && !s.loaded },
];

let running = false;
let failed = false;
const listeners = new Set<() => void>();

function setFailed(value: boolean) {
  failed = value;
  listeners.forEach((listener) => listener());
}

/** Starts the launch download, unless it is already running. "Try again" calls it too. */
export function bootstrapModels(): void {
  if (running) return;
  running = true;
  setFailed(false);
  void runSteps(STEPS)
    .then((outcome) => setFailed(outcome === "failed"))
    .finally(() => (running = false));
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useModelFooter(): Footer {
  const meeting = meetingModelsDownload.use();
  const search = searchModelDownload.use();
  const isFailed = useSyncExternalStore(subscribe, () => failed);
  return footer(meeting.job, search.job, isFailed);
}
