import type { SyncView } from "../generated/types";
import { t } from "../l10n";

/** Runs a billing command and shows its result; the host keeps the busy flag and the error. */
export type Act = (f: () => Promise<SyncView>) => void;

/** The purchase choices for the Plan & billing section and the device-limit row: the words and billing interval of each. */
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

/** The words shown under the purchase buttons. */
export function captionText(plus: boolean): string {
  return plus
    ? t("Plus includes hosted sync on all your devices and a monthly Whimbrel AI allowance. Tax is added at checkout where it applies.")
    : t("Plus tax where applicable. Opens Stripe in your browser. Cloud adds unlimited devices on our sync server.");
}
