import type { CalendarAccounts, GmailAccounts } from "../generated/types";
import { t } from "../l10n";

/** One Google sign-in that expired: which connector, and which address. */
export interface ExpiredSignIn {
  connector: "calendar" | "gmail";
  email: string;
}

/** Every expired sign-in across Calendar and Gmail, calendar first, each address once per connector. */
export function expiredSignIns(calendar: CalendarAccounts | null, gmail: GmailAccounts | null): ExpiredSignIn[] {
  const fromCalendar = (calendar?.signInExpired ?? []).map((email): ExpiredSignIn => ({ connector: "calendar", email }));
  const fromGmail = (gmail?.accounts ?? []).filter((a) => a.signInExpired).map((a): ExpiredSignIn => ({ connector: "gmail", email: a.email }));
  return [...fromCalendar, ...fromGmail];
}

/** The banner's line for one expired sign-in. */
export function expiredLine(item: ExpiredSignIn): string {
  return item.connector === "calendar"
    ? t("Google Calendar sign-in expired for {email}. Whimbrel can't read its meetings until you sign in again.", { email: item.email })
    : t("Gmail sign-in expired for {email}. Whimbrel can't read or send its mail until you sign in again.", { email: item.email });
}
