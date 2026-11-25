import { useEffect, useRef, useState } from "react";
import { syncManageSubscription, syncRefreshPlan, syncStatus, whimbrelAiRefresh, whimbrelAiStatus } from "../api";
import { describeError, isCommandError } from "../errors";
import type { SyncView, WhimbrelAiView } from "../generated/types";
import { useL10n } from "../l10n";
import { BillingError, PurchaseButtons, type Act } from "./PlanControls";
import { planAllowanceLine, planReachable, planSectionState } from "./planModel";
import type { SettingsSectionId } from "./settingsSections";
import { offersManage, offersUpgrade, planLine } from "./syncModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** How often the section looks at sync again while it is open (sign-in finishes in the background). */
const REFRESH_MS = 5000;

/**
 * Settings → Plan & billing (the Mac's PlanSettingsPane): the plan, Buy or Manage, the Plus
 * allowance, and a way to the AI provider settings. Reads sync and the plan when it opens, when
 * the window comes back (from Stripe, say), and when sync becomes ready.
 */
export function PlanCard({ onSection }: { onSection: (id: SettingsSectionId) => void }) {
  const { t } = useL10n();
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
      void syncStatus().then((v) => {
        if (!live) return;
        setView(v);
        // Sync just became ready (or reached its device limit): read the plan for it.
        const now = planReachable(v);
        if (now && !reachable.current) readPlan();
        reachable.current = now;
      }, () => {});
    refresh();
    void whimbrelAiStatus().then((v) => live && setAi(v), () => {});
    readPlan();
    const timer = setInterval(refresh, REFRESH_MS);
    window.addEventListener("focus", readPlan);
    return () => {
      live = false;
      clearInterval(timer);
      window.removeEventListener("focus", readPlan);
    };
  }, []);

  const act: Act = (f) => {
    setBusy(true);
    setBillingError(null);
    f()
      .then(setView)
      .catch((e: unknown) => setBillingError(message(e)))
      .finally(() => setBusy(false));
  };

  return (
    <section className="card about" aria-labelledby="plan-heading" data-settings-anchor="plan.summary">
      <h2 id="plan-heading" className="section-label">
        {t("Plan & billing")}
      </h2>
      {view ? (
        <PlanBody
          view={view}
          ai={ai}
          busy={busy}
          act={act}
          error={billingError}
          onSignIn={() => onSection("devices")}
          onRetry={() => void syncRefreshPlan().then(setView, () => {})}
        />
      ) : (
        <p className="muted">{t("Checking your plan…")}</p>
      )}
      <div className="field-row" data-settings-anchor="plan.provider">
        <span className="grow">{t("Use Whimbrel AI as your provider")}</span>
        <button type="button" className="mode" onClick={() => onSection("ai")}>
          {t("AI settings")}
        </button>
      </div>
    </section>
  );
}

/** The part of the section that depends on sync and the plan read; exported for tests. */
export function PlanBody(props: {
  view: SyncView;
  ai: WhimbrelAiView | null;
  busy: boolean;
  act: Act;
  error: string | null;
  onSignIn: () => void;
  onRetry: () => void;
}) {
  const { view, ai, busy, act, error, onSignIn, onRetry } = props;
  const { t, locale } = useL10n();
  switch (planSectionState(view)) {
    case "signedOut":
      return (
        <div className="stack-sm">
          <p className="muted">{t("Sign in to sync to see your plan.")}</p>
          <div className="toolbar">
            <button type="button" className="mode mode--active" onClick={onSignIn}>
              {t("Sign in to sync")}
            </button>
          </div>
        </div>
      );
    case "loading":
      return <p className="muted">{t("Checking your plan…")}</p>;
    case "readFailed":
      return (
        <div className="stack-sm">
          <p className="muted">{t("Couldn't check your plan. Try again in a moment.")}</p>
          <div className="toolbar">
            <button type="button" className="mode" onClick={onRetry}>
              {t("Try again")}
            </button>
          </div>
        </div>
      );
    case "finishSigningIn":
      return <p className="muted">{t("Finish signing in to manage your plan.")}</p>;
    case "plan": {
      const allowance = planAllowanceLine(view, ai, locale);
      return (
        <div className="stack-sm">
          <p>{view.plan && planLine(view.plan, locale)}</p>
          {offersUpgrade(view) && <PurchaseButtons view={view} busy={busy} act={act} error={error} />}
          {offersManage(view) && (
            <>
              <div className="toolbar" data-settings-anchor="plan.manage">
                <button type="button" className="mode" disabled={busy} onClick={() => act(syncManageSubscription)}>
                  {t("Manage subscription")}
                </button>
              </div>
              <BillingError error={error} />
            </>
          )}
          {allowance && <p className="muted small">{allowance}</p>}
        </div>
      );
    }
  }
}
