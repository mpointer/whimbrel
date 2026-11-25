import { useEffect, useState } from "react";
import { syncRefreshPlan, syncStatus } from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import type { SyncView } from "../../../../generated/types";

export function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

export interface SyncStatus {
  view: SyncView | null;
  error: string | null;
  busy: boolean;
  /** Upgrade reports under its own buttons: at the top of the card an error was off screen. */
  billingError: string | null;
  act: (f: () => Promise<SyncView>) => void;
  billing: (f: () => Promise<SyncView>) => void;
}

/** The sync state the old Sync card keeps: read every 2 s (sign-in and approval happen in the background), the plan read when the section opens and when the window comes back. */
export function useSyncStatus(): SyncStatus {
  const [view, setView] = useState<SyncView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [billingError, setBillingError] = useState<string | null>(null);

  useEffect(() => {
    const refresh = () => syncStatus().then(setView).catch((e: unknown) => setError(message(e)));
    refresh();
    const timer = setInterval(refresh, 2000);
    const plan = () => void syncRefreshPlan().then(setView, () => {});
    plan();
    window.addEventListener("focus", plan);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", plan);
    };
  }, []);

  const act = (f: () => Promise<SyncView>) => {
    setBusy(true);
    setError(null);
    f()
      .then(setView)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };
  const billing = (f: () => Promise<SyncView>) => {
    setBusy(true);
    setBillingError(null);
    f()
      .then(setView)
      .catch((e: unknown) => setBillingError(message(e)))
      .finally(() => setBusy(false));
  };
  return { view, error, busy, billingError, act, billing };
}
