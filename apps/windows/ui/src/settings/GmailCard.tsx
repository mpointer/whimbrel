import { useCallback, useEffect, useState } from "react";
import { GOOGLE_CLIENT_CHANGED } from "./googleClientModel";
import { gmailAccounts, gmailConnect, gmailReconnect, gmailDeleteStoredMail, gmailDisconnect, googleClientStatus } from "../api";
import { RemoveGoogleAccount } from "./RemoveGoogleAccount";
import { describeError, isCommandError } from "../errors";
import type { GmailAccounts } from "../generated/types";
import { useL10n } from "../l10n";
import {
  accountNote,
  canConnect,
  connectLabel,
  deleteMailQuestion,
  gmailIntro,
  gmailMailNote,
  gmailPermissions,
  gmailReadingIntro,
  readStatus,
  readsMail,
  sendOnlyNote,
} from "./gmailModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → Gmail (M3.4 Task 3): the account Whimbrel sends approved routine emails from. */
export function GmailCard() {
  const { t, locale } = useL10n();
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

  if (!state) return <section className="card about">{error ? <p role="alert">{error}</p> : t("Loading…")}</section>;

  return (
    <section className="card about" aria-labelledby="gmail-heading">
      <h2 id="gmail-heading" className="section-label">
        {"Gmail"}
      </h2>
      {state.blocked && <p role="alert">{state.blocked}</p>}
      <p className="muted">{gmailIntro()}</p>
      {ownClient && <p className="muted">{gmailReadingIntro()}</p>}
      {state.accounts.length > 0 && (
        <ul className="rows">
          {state.accounts.map((account) => (
            <li key={account.email} className="field-row">
              <span>{account.email}</span>
              {account.signInExpired ? (
                <>
                  <span className="small sign-in-expired">{t("Sign-in expired")}</span>
                  <button className="mode mode--active" disabled={busy || !canConnect(state)} onClick={() => run(() => gmailReconnect(account.email))}>
                    {t("Sign in again")}
                  </button>
                </>
              ) : account.needsReconnect ? (
                <button className="mode mode--active" disabled={busy || !canConnect(state)} onClick={() => run(gmailConnect)}>
                  {t("Reconnect")}
                </button>
              ) : null}
              <button className="mode" disabled={busy} onClick={() => setConfirmRemove(account.email)}>
                {t("Disconnect")}
              </button>
              {confirmRemove === account.email && (
                <RemoveGoogleAccount
                  service="gmail"
                  email={account.email}
                  busy={busy}
                  onRemove={() => {
                    setConfirmRemove(null);
                    run(() => gmailDisconnect(account.email));
                  }}
                  onCancel={() => setConfirmRemove(null)}
                />
              )}
              <button className="mode" disabled={busy} onClick={() => setConfirmDelete(account.email)}>
                {t("Delete stored mail…")}
              </button>
              {confirmDelete === account.email && (
                <span role="group" aria-label={t("Confirm deleting stored mail")}>
                  <span className="muted small">{deleteMailQuestion(account.email)}</span>
                  <button className="mode mode--active" disabled={busy} onClick={() => run(() => gmailDeleteStoredMail(account.email))}>
                    {t("Delete mail")}
                  </button>
                  <button
                    className="mode"
                    disabled={busy}
                    onClick={() => run(() => gmailDeleteStoredMail(account.email).then(() => gmailDisconnect(account.email)))}
                  >
                    {t("Delete mail and disconnect")}
                  </button>
                  <button className="mode" onClick={() => setConfirmDelete(null)}>
                    {t("Cancel")}
                  </button>
                </span>
              )}
              {accountNote(account) && <span className="muted small">{accountNote(account)}</span>}
              {account.sendOnly ? (
                <span className="muted small">{sendOnlyNote()}</span>
              ) : (
                <span className="muted small">{readStatus(account, locale)}</span>
              )}
            </li>
          ))}
        </ul>
      )}
      <div className="toolbar">
        <button className="mode mode--active" disabled={busy || !canConnect(state)} onClick={() => run(gmailConnect)}>
          {connectLabel(state.accounts.length)}
        </button>
      </div>
      {readsMail(state, ownClient) && <p className="muted small">{gmailMailNote()}</p>}
      <p className="muted small">{gmailPermissions(ownClient)}</p>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
