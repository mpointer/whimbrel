// The redesigned Plan & billing card, drawn from what usePlan read. Same states and conditions as the old PlanBody:
// signed out, checking, could not check, finish signing in, and the plan (with Buy or Manage, and the Plus allowance).
import { WButton, WSettingsCard, WSettingsControlRow, WSettingsNote } from "../../../../system";
import type { SyncView, WhimbrelAiView } from "../../../../generated/types";
import { t, useL10n } from "../../../../l10n";
import { planAllowanceLine, planSectionState } from "../../../planModel";
import { captionText, purchaseOffers, type Act } from "../../../PlanControls";
import { offersManage, offersUpgrade, planLine, sellsPlus } from "../../../syncModel";
import { syncManageSubscription, syncUpgrade } from "../../../../api";
import type { SettingsSectionId } from "../../../settingsSections";
import { useJump } from "../../jump";

export interface PlanBodyProps {
  /** Null until the first read of sync lands. */
  view: SyncView | null;
  ai: WhimbrelAiView | null;
  busy: boolean;
  error: string | null;
  act: Act;
  onRetry: () => void;
  onSection: (id: SettingsSectionId) => void;
}

export function PlanBody({ view, ai, busy, error, act, onRetry, onSection }: PlanBodyProps) {
  const { locale } = useL10n();
  const jumpManage = useJump("plan.manage");
  const jumpProvider = useJump("plan.provider");
  const state = view ? planSectionState(view) : "loading";
  const working = busy ? { disabled: true, disabledReason: t("Working…") } : {};

  const rows = () => {
    if (!view) return <WSettingsControlRow label={t("Checking your plan…")}>{null}</WSettingsControlRow>;
    switch (state) {
      case "signedOut":
        return (
          <WSettingsControlRow label={t("Sign in to sync to see your plan.")}>
            <WButton variant="primary" onClick={() => onSection("devices")}>
              {t("Sign in to sync")}
            </WButton>
          </WSettingsControlRow>
        );
      case "loading":
        return <WSettingsControlRow label={t("Checking your plan…")}>{null}</WSettingsControlRow>;
      case "readFailed":
        return (
          <WSettingsControlRow label={t("Couldn't check your plan. Try again in a moment.")}>
            <WButton onClick={onRetry}>{t("Try again")}</WButton>
          </WSettingsControlRow>
        );
      case "finishSigningIn":
        return <WSettingsControlRow label={t("Finish signing in to manage your plan.")}>{null}</WSettingsControlRow>;
      case "plan": {
        const allowance = planAllowanceLine(view, ai, locale);
        const line = view.plan ? planLine(view.plan, locale) : "";
        const plus = sellsPlus(view);
        return (
          <>
            {offersUpgrade(view) ? (
              <WSettingsControlRow key="upgrade" label={line} error={error} {...working}>
                {purchaseOffers(plus).map((o) => (
                  <WButton key={o.interval} variant={o.primary ? "primary" : "secondary"} disabled={busy} onClick={() => act(() => syncUpgrade(o.interval))}>
                    {o.label}
                  </WButton>
                ))}
              </WSettingsControlRow>
            ) : offersManage(view) ? (
              <WSettingsControlRow key="manage" id="plan.manage" highlighted={jumpManage} label={line} error={error} {...working}>
                <WButton disabled={busy} onClick={() => act(syncManageSubscription)}>
                  {t("Manage subscription")}
                </WButton>
              </WSettingsControlRow>
            ) : (
              <WSettingsControlRow key="plain" label={line} error={error}>
                {null}
              </WSettingsControlRow>
            )}
            {offersUpgrade(view) ? <WSettingsNote>{captionText(plus)}</WSettingsNote> : null}
            {allowance ? <WSettingsNote>{allowance}</WSettingsNote> : null}
          </>
        );
      }
    }
  };

  return (
    <WSettingsCard id="plan.summary" title={t("Plan & billing")} headingLevel={3}>
      {rows()}
      <WSettingsControlRow id="plan.provider" highlighted={jumpProvider} label={t("Use Whimbrel AI as your provider")}>
        <WButton onClick={() => onSection("ai")}>{t("AI settings")}</WButton>
      </WSettingsControlRow>
    </WSettingsCard>
  );
}
