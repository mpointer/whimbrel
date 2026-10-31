import { useCallback, useEffect, useState } from "react";
import { calendarAccounts, calendarConnect, calendarDisconnect, calendarSyncNow } from "../api";
import { describeError, isCommandError } from "../errors";
import type { CalendarAccounts } from "../generated/types";
import { t, useL10n } from "../l10n";
import { dateTimeLabel, timeLabel } from "../library/rows";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → Connected accounts (M3.3 Task 7): the Google Calendar Whimbrel reads. */
export function AccountsCard() {
  const [accounts, setAccounts] = useState<CalendarAccounts | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { locale } = useL10n();

  const load = useCallback(
    () =>
      calendarAccounts()
        .then(setAccounts)
        .catch((e: unknown) => setError(message(e))),
    [],
  );

  useEffect(() => {
    void load();
  }, [load]);

  const run = (call: () => Promise<CalendarAccounts>) => {
    setBusy(true);
    setError(null);
    call()
      .then(setAccounts)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  if (!accounts) return <section className="card about">{error ? <p role="alert">{error}</p> : t("Loading…")}</section>;

  return (
    <section className="card about" aria-labelledby="accounts-heading">
      <h2 id="accounts-heading" className="section-label">
        {t("Connected accounts")}
      </h2>
      {accounts.blocked ? (
        <p role="alert">{accounts.blocked}</p>
      ) : (
        <p className="muted">
          {t("A connected Google Calendar lets Whimbrel offer to record a meeting just before it starts.")}
        </p>
      )}
      {accounts.emails.length > 0 && (
        <ul className="rows">
          {accounts.emails.map((email) => (
            <li key={email} className="field-row">
              <span>{email}</span>
              <button className="mode" disabled={busy} onClick={() => run(() => calendarDisconnect(email))}>
                {t("Disconnect")}
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="toolbar">
        <button className="mode mode--active" disabled={busy || accounts.blocked != null} onClick={() => run(calendarConnect)}>
          {accounts.emails.length > 0 ? t("Connect another calendar") : t("Connect Google Calendar")}
        </button>
        {accounts.emails.length > 0 && (
          <button className="mode" disabled={busy} onClick={() => run(calendarSyncNow)}>
            {t("Sync now")}
          </button>
        )}
      </div>
      {accounts.emails.length > 0 && (
        <p className="muted small" role="status">
          {accounts.syncError
            ? t("Last sync failed: {error}", { error: accounts.syncError })
            : [
                accounts.lastSyncedUnixMs
                  ? t("Synced {time}", { time: timeLabel(accounts.lastSyncedUnixMs, locale) })
                  : t("Not synced yet"),
                accounts.upcoming === 0
                  ? t("nothing coming up to record")
                  : t("{count} coming up · next: {meeting} at {time}", {
                      count: accounts.upcoming,
                      meeting: String(accounts.nextMeeting),
                      time: dateTimeLabel(accounts.nextStartUnixMs!, new Date(), locale),
                    }),
              ].join(" · ")}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
