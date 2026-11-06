import { useEffect, useState } from "react";
import { budgetAlertDismiss, budgetAlertRaise, budgetAlertStatus } from "../api";
import type { BudgetNoticeView } from "../generated/types";
import { t, useL10n } from "../l10n";
import { formatDollars, providers } from "../settings/aiModel";

/** "November" for "2026-11", in the UI language. */
export function monthName(month: string, locale: string): string {
  const [year, m] = month.split("-").map(Number);
  if (!year || !m) return month;
  return new Intl.DateTimeFormat(locale, { month: "long", timeZone: "UTC" }).format(new Date(Date.UTC(year, m - 1, 15)));
}

/** A notice's sentence, the Mac's copy. */
export function budgetNoticeText(n: BudgetNoticeView, locale: string): string {
  const provider = providers().find((p) => p.id === n.provider)?.label ?? n.provider;
  if (n.threshold >= 100) {
    return t("{provider} reached this month's budget. Whimbrel will use your other providers until {month} ends.", {
      provider,
      month: monthName(n.month, locale),
    });
  }
  return t("{provider} is at 80% of this month's budget ({spent} of {budget}).", {
    provider,
    spent: formatDollars(n.spentCents),
    budget: formatDollars(n.budgetCents),
  });
}

/**
 * The budget alert window (provider budgets): a provider reached 80% or 100% of this month's
 * budget. Raise budget opens Settings → AI; Dismiss clears every notice.
 */
export function BudgetAlertView() {
  const { locale } = useL10n();
  const [notices, setNotices] = useState<BudgetNoticeView[]>([]);

  useEffect(() => {
    const load = () => budgetAlertStatus().then(setNotices, () => {});
    load();
    const timer = setInterval(load, 1_000);
    return () => clearInterval(timer);
  }, []);

  if (notices.length === 0) return <div className="alert" />;
  return (
    <div className="alert">
      {/* The product name, the same in every language. */}
      <span className="popup-brand">{"Whimbrel"}</span>
      {notices.map((n) => (
        <p key={n.provider} role="status" className="alert-title">
          {budgetNoticeText(n, locale)}
        </p>
      ))}
      <div className="popup-actions">
        <button className="mode mode--active" onClick={() => void budgetAlertRaise()}>
          {t("Raise budget")}
        </button>
        <button className="mode" onClick={() => void budgetAlertDismiss()}>
          {t("Dismiss")}
        </button>
      </div>
    </div>
  );
}
