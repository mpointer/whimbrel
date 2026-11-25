// What the redesigned Plan & billing section reads and when, the same as the old PlanCard: sync status every 5 s while
// open, the plan and the Whimbrel AI balance when it opens, when the window comes back (from Stripe, say) and when
// sync becomes ready. Same api.ts commands, same conditions.
import { useEffect, useRef, useState } from "react";
import { syncRefreshPlan, syncStatus, whimbrelAiRefresh, whimbrelAiStatus } from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import type { SyncView, WhimbrelAiView } from "../../../../generated/types";
import { planReachable } from "../../../planModel";
import type { Act } from "../../../PlanControls";

export const PLAN_REFRESH_MS = 5000;

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

export function usePlan() {
  const [view, setView] = useState<SyncView | null>(null);
  const [ai, setAi] = useState<WhimbrelAiView | null>(null);
  const [busy, setBusy] = useState(false);
  const [billingError, setBillingError] = useState<string | null>(null);
  const reachable = useRef(false);

  useEffect(() => {
    let live = true;
    const readPlan = () => {
      void syncRefreshPlan().then((v) => live && setView(v), () => {});
      void whimbrelAiRefresh().then((v) => live && setAi(v), () => {});
    };
    const refresh = () =>
      void syncStatus().then(
        (v) => {
          if (!live) return;
          setView(v);
          // Sync just became ready (or reached its device limit): read the plan for it.
          const now = planReachable(v);
          if (now && !reachable.current) readPlan();
          reachable.current = now;
        },
        () => {},
      );
    refresh();
    void whimbrelAiStatus().then((v) => live && setAi(v), () => {});
    readPlan();
    const timer = setInterval(refresh, PLAN_REFRESH_MS);
    window.addEventListener("focus", readPlan);
    return () => {
      live = false;
      clearInterval(timer);
      window.removeEventListener("focus", readPlan);
    };
  }, []);

  /** Runs a billing command and shows its result; the busy flag and the error are kept here. */
  const act: Act = (f) => {
    setBusy(true);
    setBillingError(null);
    f()
      .then(setView)
      .catch((e: unknown) => setBillingError(message(e)))
      .finally(() => setBusy(false));
  };

  const retry = () => void syncRefreshPlan().then(setView, () => {});

  return { view, ai, busy, billingError, act, retry };
}
