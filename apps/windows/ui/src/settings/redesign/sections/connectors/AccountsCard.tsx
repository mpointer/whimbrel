import { useCallback, useEffect, useState } from "react";
import { calendarAccounts, calendarConnect, calendarDisconnect, calendarReconnect, calendarSyncNow } from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import type { CalendarAccounts } from "../../../../generated/types";
import { t, useL10n } from "../../../../l10n";
import { dateTimeLabel, timeLabel } from "../../../../library/rows";
import { WButton, WSettingsCard, WSettingsControlRow, WSettingsRow, WStatusChip } from "../../../../system";
import { calendarAccessNote } from "../../../accountsModel";
import { GOOGLE_CLIENT_CHANGED } from "../../../googleClientModel";
import { GoogleRemoveDialog } from "./GoogleRemoveDialog";
import { forAccount } from "./connectorsModel";
import "../devices/devices.css";
import "./connectors.css";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Connected accounts: the Google Calendars Whimbrel reads. Connect, sign in again, disconnect (asks first), sync now. */
export function AccountsCard() {
  const { locale } = useL10n();
  const [accounts, setAccounts] = useState<CalendarAccounts | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
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
    // Saving or clearing your own Google client means signing in again: show it at once.
    const reload = () => void load();
    window.addEventListener(GOOGLE_CLIENT_CHANGED, reload);
    return () => window.removeEventListener(GOOGLE_CLIENT_CHANGED, reload);
  }, [load]);

  const run = (call: () => Promise<CalendarAccounts>) => {
    setBusy(true);
    setError(null);
    call()
      .then(setAccounts)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const blocked = accounts?.blocked ?? null;
  const status = accounts && accounts.emails.length > 0 ? syncStatusLine(accounts, locale) : null;
  return (
    <>
    <WSettingsCard
      id="connectors.accounts"
      title={t("Connected accounts")}
      headingLevel={3}
      description={blocked ? undefined : t("A connected Google Calendar lets Whimbrel offer to record a meeting just before it starts.")}
      loading={!accounts && !error}
      error={[blocked, error]}
    >
      {accounts && (
        <>
          {accounts.emails.map((email) => {
            const expired = accounts.signInExpired.includes(email);
            return (
              <WSettingsControlRow key={email} label={email} description={calendarAccessNote(accounts, email) ?? undefined} stacked>
                <span className="rs-devconn-actions">
                  {expired && (
                    <>
                      <WStatusChip tone="warning">{t("Sign-in expired")}</WStatusChip>
                      <WButton variant="primary" aria-label={forAccount(t("Sign in again"), email)} disabled={busy || blocked != null} onClick={() => run(() => calendarReconnect(email))}>
                        {t("Sign in again")}
                      </WButton>
                    </>
                  )}
                  <WButton aria-label={forAccount(t("Disconnect"), email)} disabled={busy} onClick={() => setConfirmRemove(email)}>
                    {t("Disconnect")}
                  </WButton>
                </span>
              </WSettingsControlRow>
            );
          })}
          <WSettingsRow
            variant="button"
            label={t("Google Calendar")}
            actionLabel={accounts.emails.length > 0 ? t("Connect another calendar") : t("Connect Google Calendar")}
            disabled={busy || blocked != null}
            disabledReason={blocked ?? undefined}
            onPress={() => run(calendarConnect)}
          />
          {accounts.emails.length > 0 && <WSettingsRow variant="button" label={t("Calendar sync")} actionLabel={t("Sync now")} disabled={busy} onPress={() => run(calendarSyncNow)} />}
          {status && (
            <p className="w-setting-note" role="status">
              {status}
            </p>
          )}
        </>
      )}
    </WSettingsCard>
      {confirmRemove && (
        <GoogleRemoveDialog
          service="calendar"
          email={confirmRemove}
          busy={busy}
          onRemove={() => {
            const email = confirmRemove;
            setConfirmRemove(null);
            run(() => calendarDisconnect(email));
          }}
          onCancel={() => setConfirmRemove(null)}
        />
      )}
    </>
  );
}

/** "Synced 9:30 AM · 2 coming up · next: Standup at Tomorrow 9:00 AM", or why the last sync failed. */
export function syncStatusLine(accounts: CalendarAccounts, locale: string): string {
  if (accounts.syncError) return t("Last sync failed: {error}", { error: accounts.syncError });
  return [
    accounts.lastSyncedUnixMs ? t("Synced {time}", { time: timeLabel(accounts.lastSyncedUnixMs, locale) }) : t("Not synced yet"),
    accounts.upcoming === 0
      ? t("nothing coming up to record")
      : t("{count} coming up · next: {meeting} at {time}", {
          count: accounts.upcoming,
          meeting: String(accounts.nextMeeting),
          time: dateTimeLabel(accounts.nextStartUnixMs!, new Date(), locale),
        }),
  ].join(" · ");
}
