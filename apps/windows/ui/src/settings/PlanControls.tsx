import { syncUpgrade } from "../api";
import type { SyncView } from "../generated/types";
import { t, useL10n } from "../l10n";
import { offersUpgrade, sellsPlus } from "./syncModel";

/** Runs a billing command and shows its result; the host keeps the busy flag and the error. */
export type Act = (f: () => Promise<SyncView>) => void;

/** What Upgrade or Manage subscription last reported, right under the buttons. */
export function BillingError({ error }: { error: string | null }) {
  return error ? (
    <p role="alert" className="small">
      {error}
    </p>
  ) : null;
}

/**
 * The two Upgrade buttons (Cloud v1), shown only on the free plan with billing open. When the server
 * sells Plus they are the Plus ones instead (the shell asks for Plus then); never both.
 */
export function UpgradeButtons({ busy, act, plus }: { busy: boolean; act: Act; plus: boolean }) {
  const { t } = useL10n();
  if (plus) {
    return (
      <div className="toolbar">
        <button className="mode mode--active" disabled={busy} onClick={() => act(() => syncUpgrade("month"))}>
          {t("Get Plus: $15 a month")}
        </button>
        <button className="mode" disabled={busy} onClick={() => act(() => syncUpgrade("year"))}>
          {t("Get Plus: $144 a year")}
        </button>
      </div>
    );
  }
  return (
    <div className="toolbar">
      <button className="mode mode--active" disabled={busy} onClick={() => act(() => syncUpgrade("month"))}>
        {t("Upgrade to Cloud: $5 a month")}
      </button>
      <button className="mode" disabled={busy} onClick={() => act(() => syncUpgrade("year"))}>
        {t("Upgrade to Cloud: $50 a year")}
      </button>
    </div>
  );
}

export function Caption({ plus }: { plus: boolean }) {
  const { t } = useL10n();
  if (plus) {
    return (
      <p className="muted small">
        {t("Plus includes hosted sync on all your devices and a monthly Whimbrel AI allowance. Tax is added at checkout where it applies.")}
      </p>
    );
  }
  return <p className="muted small">{t("Plus tax where applicable. Opens Stripe in your browser. Cloud adds unlimited devices on our sync server.")}</p>;
}

/** The purchase buttons, their error and the caption, in that order (the Mac's PlanPurchaseButtons). */
export function PurchaseButtons({ view, busy, act, error }: { view: SyncView; busy: boolean; act: Act; error: string | null }) {
  const plus = sellsPlus(view);
  return (
    <>
      <UpgradeButtons busy={busy} act={act} plus={plus} />
      <BillingError error={error} />
      <Caption plus={plus} />
    </>
  );
}

/** At the free plan's device limit: the way past it. Stays in Sync & Devices, beside the device list. */
export function LimitUpgrade({ view, busy, act, error }: { view: SyncView; busy: boolean; act: Act; error: string | null }) {
  const { t } = useL10n();
  if (view.planUnknown) return <p className="muted small">{t("Couldn't check your plan. Try again in a moment.")}</p>;
  if (!offersUpgrade(view)) return null;
  return (
    <div className="stack-sm">
      <p>{sellsPlus(view) ? t("Upgrade to Plus for unlimited devices.") : t("Upgrade to Cloud for unlimited devices.")}</p>
      <PurchaseButtons view={view} busy={busy} act={act} error={error} />
    </div>
  );
}

/**
 * The same purchase choices as `UpgradeButtons`, as data, for the redesigned Plan & billing section. The words and
 * the billing interval of each are the ones the buttons above use; nothing here is a second price list.
 */
export function purchaseOffers(plus: boolean): { interval: "month" | "year"; label: string; primary: boolean }[] {
  if (plus) {
    return [
      { interval: "month", label: t("Get Plus: $15 a month"), primary: true },
      { interval: "year", label: t("Get Plus: $144 a year"), primary: false },
    ];
  }
  return [
    { interval: "month", label: t("Upgrade to Cloud: $5 a month"), primary: true },
    { interval: "year", label: t("Upgrade to Cloud: $50 a year"), primary: false },
  ];
}

/** The words `Caption` shows under the purchase buttons, for the redesigned section. */
export function captionText(plus: boolean): string {
  return plus
    ? t("Plus includes hosted sync on all your devices and a monthly Whimbrel AI allowance. Tax is added at checkout where it applies.")
    : t("Plus tax where applicable. Opens Stripe in your browser. Cloud adds unlimited devices on our sync server.");
}
