import { useCallback, useEffect, useState } from "react";
import { calendarAccounts, calendarConnect, calendarDisconnect, calendarSyncNow } from "../api";
import { describeError, isCommandError } from "../errors";
import type { CalendarAccounts } from "../generated/types";
import { dateTimeLabel, timeLabel } from "../library/rows";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → Connected accounts (M3.3 Task 7): the Google Calendar Whimbrel reads. */
export function AccountsCard() {
  const [accounts, setAccounts] = useState<CalendarAccounts | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  if (!accounts) return <section className="card about">{error ? <p role="alert">{error}</p> : "Loading…"}</section>;

  return (
    <section className="card about" aria-labelledby="accounts-heading">
      <h2 id="accounts-heading" className="section-label">
        Connected accounts
      </h2>
      {accounts.blocked ? (
        <p role="alert">{accounts.blocked}</p>
      ) : (
        <p className="muted">
          A connected Google Calendar lets Whimbrel offer to record a meeting just before it starts.
        </p>
      )}
      {accounts.emails.length > 0 && (
        <ul className="rows">
          {accounts.emails.map((email) => (
            <li key={email} className="field-row">
              <span>{email}</span>
              <button className="mode" disabled={busy} onClick={() => run(() => calendarDisconnect(email))}>
                Disconnect
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="toolbar">
        <button className="mode mode--active" disabled={busy || accounts.blocked != null} onClick={() => run(calendarConnect)}>
          {accounts.emails.length > 0 ? "Connect another calendar" : "Connect Google Calendar"}
        </button>
        {accounts.emails.length > 0 && (
          <button className="mode" disabled={busy} onClick={() => run(calendarSyncNow)}>
            Sync now
          </button>
        )}
      </div>
      {accounts.emails.length > 0 && (
        <p className="muted small" role="status">
          {accounts.syncError
            ? `Last sync failed: ${accounts.syncError}`
            : [
                accounts.lastSyncedUnixMs ? `Synced ${timeLabel(accounts.lastSyncedUnixMs)}` : "Not synced yet",
                accounts.upcoming === 0
                  ? "nothing coming up to record"
                  : `${accounts.upcoming} coming up · next: ${accounts.nextMeeting} at ${dateTimeLabel(accounts.nextStartUnixMs!)}`,
              ].join(" · ")}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
