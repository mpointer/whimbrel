import { useCallback, useEffect, useState } from "react";
import { calendarAccounts, calendarReconnect, gmailAccounts, gmailReconnect } from "../api";
import { describeError, isCommandError } from "../errors";
import type { CalendarAccounts, GmailAccounts } from "../generated/types";
import { t } from "../l10n";
import { expiredLine, expiredSignIns, type ExpiredSignIn } from "./signInModel";

// Calendar polls every 5 minutes, so an expiry shows within a round or two.
const POLL_MS = 2 * 60 * 1000;

/**
 * "Google sign-in expired": one line per account whose sign-in Google revoked or let lapse, each with
 * Sign in again. Shown on every page, because an expired account otherwise fails quietly for days.
 */
export function SignInBanner() {
  const [calendar, setCalendar] = useState<CalendarAccounts | null>(null);
  const [gmail, setGmail] = useState<GmailAccounts | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const poll = useCallback(() => {
    calendarAccounts().then(setCalendar).catch(() => {});
    gmailAccounts().then(setGmail).catch(() => {});
  }, []);

  useEffect(() => {
    poll();
    const id = setInterval(poll, POLL_MS);
    return () => clearInterval(id);
  }, [poll]);

  const expired = expiredSignIns(calendar, gmail);
  if (expired.length === 0) return null;

  const signIn = (item: ExpiredSignIn) => {
    setBusy(`${item.connector}:${item.email}`);
    setError(null);
    const call = item.connector === "calendar" ? calendarReconnect(item.email).then(setCalendar) : gmailReconnect(item.email).then(setGmail);
    call.catch((e: unknown) => setError(isCommandError(e) ? describeError(e) : String(e))).finally(() => setBusy(null));
  };

  return (
    <section className="card notice" role="region" aria-label={t("Google sign-in expired")}>
      {expired.map((item) => (
        <div key={`${item.connector}:${item.email}`} className="toolbar">
          <p>{expiredLine(item)}</p>
          <button className="mode" disabled={busy !== null} onClick={() => signIn(item)}>
            {busy === `${item.connector}:${item.email}` ? t("Waiting for Google…") : t("Sign in again")}
          </button>
        </div>
      ))}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
