import type { CalendarAccounts } from "../generated/types";
import { t } from "../l10n";

/** What an account's row says when Google keeps Whimbrel from its calendar (core's codes), or null. */
export function calendarAccessNote(accounts: Pick<CalendarAccounts, "accessIssues">, email: string): string | null {
  switch (accounts.accessIssues.find((i) => i.email === email)?.code) {
    case "calendar_access_not_granted":
      return `${t("Google didn't give Whimbrel access to your calendar.")} ${t("Click Connect another calendar, sign in as this address again, and allow calendar access.")}`;
    case "calendar_access_blocked":
      return t("Google is blocking calendar access for this account. Your organization may need to allow Whimbrel.");
    default:
      return null;
  }
}
