// One model download's shared state, so a download started from Settings or by the launch download
// (`modelBootstrap.ts`) shows in both. One poller per store for the whole window; Rust also joins a
// second start to the running job.

import { useSyncExternalStore } from "react";
import { modelDownloadCancel, modelDownloadPoll } from "../api";
import { describeError, isCommandError } from "../errors";
import type { DownloadView } from "../generated/types";
import { t } from "../l10n";

export interface DownloadState<S> {
  /** Null until the first status read. */
  status: S | null;
  job: DownloadView | null;
  /** A command that failed (not a download failure, which is `job.error`). */
  error: string | null;
}

export interface DownloadStore<S> {
  /** The shared state; reading it loads the status the first time. */
  use(): DownloadState<S>;
  get(): DownloadState<S>;
  /** Reads the status again (after a download, or when a card mounts). */
  refresh(): Promise<void>;
  setStatus(status: S): void;
  /** Starts the download, or joins the running one. Resolves with the job once it reports done,
   *  or null if a command failed. */
  start(): Promise<DownloadView | null>;
  cancel(): void;
}

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

export function downloadStore<S>(readStatus: () => Promise<S>, startJob: () => Promise<string>): DownloadStore<S> {
  let snapshot: DownloadState<S> = { status: null, job: null, error: null };
  const listeners = new Set<() => void>();
  let timer: ReturnType<typeof setInterval> | null = null;
  let loading = false;
  let waiters: ((end: DownloadView | null) => void)[] = [];

  const set = (change: Partial<DownloadState<S>>) => {
    snapshot = { ...snapshot, ...change };
    listeners.forEach((listener) => listener());
  };
  const finish = (end: DownloadView | null) => {
    const done = waiters;
    waiters = [];
    done.forEach((resolve) => resolve(end));
  };
  const stopPolling = () => {
    if (timer) clearInterval(timer);
    timer = null;
  };

  const refresh = () =>
    readStatus().then(
      (status) => set({ status }),
      (e: unknown) => set({ error: message(e) }),
    );

  const poll = (jobId: string) => {
    stopPolling();
    // 200 ms, the Mac shim's cadence.
    timer = setInterval(() => {
      modelDownloadPoll(jobId)
        .then((view) => {
          set({ job: view });
          if (view.done) {
            stopPolling();
            void refresh().then(() => finish(view));
          }
        })
        // The shell releases a job once it reports done, so a poll already in flight when that
        // happens fails; that is not an error once the job has finished.
        .catch((e: unknown) => {
          if (snapshot.job?.done) return;
          stopPolling();
          set({ job: null, error: message(e) });
          finish(null);
        });
    }, 200);
  };

  const start = () =>
    new Promise<DownloadView | null>((resolve) => {
      waiters.push(resolve);
      set({ error: null });
      startJob().then(
        (jobId) => {
          if (snapshot.job?.jobId !== jobId || snapshot.job.done) {
            set({ job: { jobId, fraction: 0, status: t("Starting…"), done: false, cancelled: false } });
          }
          poll(jobId);
        },
        (e: unknown) => {
          set({ error: message(e) });
          finish(null);
        },
      );
    });

  const subscribe = (listener: () => void) => {
    listeners.add(listener);
    if (!snapshot.status && !loading) {
      loading = true;
      void refresh().finally(() => (loading = false));
    }
    return () => listeners.delete(listener);
  };

  return {
    use: () => useSyncExternalStore(subscribe, () => snapshot),
    get: () => snapshot,
    refresh,
    setStatus: (status) => set({ status }),
    start,
    cancel: () => {
      const job = snapshot.job;
      if (job && !job.done) void modelDownloadCancel(job.jobId);
    },
  };
}
