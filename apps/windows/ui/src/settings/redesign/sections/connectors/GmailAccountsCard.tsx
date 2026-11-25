import { useCallback, useEffect, useState } from "react";
import { gmailAccounts, gmailConnect, gmailDeleteStoredMail, gmailDisconnect, gmailReconnect, googleClientStatus } from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import type { GmailAccounts } from "../../../../generated/types";
import { t, useL10n } from "../../../../l10n";
import { WButton, WDialog, WSettingsCard, WSettingsControlRow, WSettingsNote, WSettingsRow, WStatusChip } from "../../../../system";
import { GOOGLE_CLIENT_CHANGED } from "../../../googleClientModel";
import { accountNote, canConnect, connectLabel, deleteMailQuestion, gmailIntro, gmailMailNote, gmailPermissions, gmailReadingIntro, readStatus, readsMail, sendOnlyNote } from "../../../gmailModel";
import { GoogleRemoveDialog } from "./GoogleRemoveDialog";
import { forAccount } from "./connectorsModel";
import "../devices/devices.css";
import "./connectors.css";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Gmail: the account Whimbrel sends approved routine emails from (and reads mail with, on your own Google project). */
export function GmailAccountsCard() {
  const { locale } = useL10n();
  const [state, setState] = useState<GmailAccounts | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [ownClient, setOwnClient] = useState(false);

  const load = useCallback(
    () =>
      Promise.all([gmailAccounts(), googleClientStatus()])
        .then(([accounts, client]) => {
          setState(accounts);
          setOwnClient(client.clientId != null);
        })
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

  const run = (call: () => Promise<GmailAccounts>) => {
    setBusy(true);
    setError(null);
    setConfirmDelete(null);
    call()
      .then(setState)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const blocked = state?.blocked ?? null;
  return (
    <>
    <WSettingsCard id="connectors.gmail" title="Gmail" headingLevel={3} description={gmailIntro()} loading={!state && !error} error={[blocked, error]}>
      {state && (
        <>
          {ownClient && <WSettingsNote>{gmailReadingIntro()}</WSettingsNote>}
          {state.accounts.map((account) => {
            const note = [accountNote(account), account.sendOnly ? sendOnlyNote() : readStatus(account, locale)].filter(Boolean).join(" ");
            return (
              <WSettingsControlRow key={account.email} label={account.email} description={note} stacked>
                <span className="rs-devconn-actions">
                  {account.signInExpired ? (
                    <>
                      <WStatusChip tone="warning">{t("Sign-in expired")}</WStatusChip>
                      <WButton variant="primary" aria-label={forAccount(t("Sign in again"), account.email)} disabled={busy || !canConnect(state)} onClick={() => run(() => gmailReconnect(account.email))}>
                        {t("Sign in again")}
                      </WButton>
                    </>
                  ) : account.needsReconnect ? (
                    <WButton variant="primary" aria-label={forAccount(t("Reconnect"), account.email)} disabled={busy || !canConnect(state)} onClick={() => run(gmailConnect)}>
                      {t("Reconnect")}
                    </WButton>
                  ) : null}
                  <WButton aria-label={forAccount(t("Disconnect"), account.email)} disabled={busy} onClick={() => setConfirmRemove(account.email)}>
                    {t("Disconnect")}
                  </WButton>
                  <WButton aria-label={forAccount(t("Delete stored mail…"), account.email)} disabled={busy} onClick={() => setConfirmDelete(account.email)}>
                    {t("Delete stored mail…")}
                  </WButton>
                </span>
              </WSettingsControlRow>
            );
          })}
          <WSettingsRow variant="button" label={t("Gmail account")} actionLabel={connectLabel(state.accounts.length)} disabled={busy || !canConnect(state)} disabledReason={blocked ?? undefined} onPress={() => run(gmailConnect)} />
          {readsMail(state, ownClient) && <WSettingsNote>{gmailMailNote()}</WSettingsNote>}
          <WSettingsNote>{gmailPermissions(ownClient)}</WSettingsNote>
        </>
      )}
    </WSettingsCard>
      {confirmRemove && (
        <GoogleRemoveDialog
          service="gmail"
          email={confirmRemove}
          busy={busy}
          onRemove={() => {
            const email = confirmRemove;
            setConfirmRemove(null);
            run(() => gmailDisconnect(email));
          }}
          onCancel={() => setConfirmRemove(null)}
        />
      )}
      {confirmDelete && (
        <WDialog title={t("Confirm deleting stored mail")} body={deleteMailQuestion(confirmDelete)} onCancel={() => setConfirmDelete(null)}>
          <WButton data-initial onClick={() => setConfirmDelete(null)}>
            {t("Cancel")}
          </WButton>
          <WButton variant="destructive" disabled={busy} onClick={() => run(() => gmailDeleteStoredMail(confirmDelete))}>
            {t("Delete mail")}
          </WButton>
          <WButton variant="destructive" disabled={busy} onClick={() => run(() => gmailDeleteStoredMail(confirmDelete).then(() => gmailDisconnect(confirmDelete)))}>
            {t("Delete mail and disconnect")}
          </WButton>
        </WDialog>
      )}
    </>
  );
}
