import { useCallback, useEffect, useState } from "react";
import { gmailAccounts, gmailConnect, gmailDeleteStoredMail, gmailDisconnect } from "../api";
import { describeError, isCommandError } from "../errors";
import type { GmailAccounts } from "../generated/types";
import { GMAIL_INTRO, deleteMailQuestion, GMAIL_MAIL_NOTE, GMAIL_PERMISSIONS, accountNote, canConnect, connectLabel, readStatus } from "./gmailModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → Gmail (M3.4 Task 3): the account Whimbrel sends approved routine emails from. */
export function GmailCard() {
  const [state, setState] = useState<GmailAccounts | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const load = useCallback(
    () =>
      gmailAccounts()
        .then(setState)
        .catch((e: unknown) => setError(message(e))),
    [],
  );

  useEffect(() => {
    void load();
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

  if (!state) return <section className="card about">{error ? <p role="alert">{error}</p> : "Loading…"}</section>;

  return (
    <section className="card about" aria-labelledby="gmail-heading">
      <h2 id="gmail-heading" className="section-label">
        Gmail
      </h2>
      {state.blocked && <p role="alert">{state.blocked}</p>}
      <p className="muted">{GMAIL_INTRO}</p>
      {state.accounts.length > 0 && (
        <ul className="rows">
          {state.accounts.map((account) => (
            <li key={account.email} className="field-row">
              <span>{account.email}</span>
              {account.needsReconnect ? (
                <button className="mode mode--active" disabled={busy || !canConnect(state)} onClick={() => run(gmailConnect)}>
                  Reconnect
                </button>
              ) : null}
              <button className="mode" disabled={busy} onClick={() => run(() => gmailDisconnect(account.email))}>
                Disconnect
              </button>
              <button className="mode" disabled={busy} onClick={() => setConfirmDelete(account.email)}>
                Delete stored mail�
              </button>
              {confirmDelete === account.email && (
                <span role="group" aria-label="Confirm deleting stored mail">
                  <span className="muted small">{deleteMailQuestion(account.email)}</span>
                  <button className="mode mode--active" disabled={busy} onClick={() => run(() => gmailDeleteStoredMail(account.email))}>
                    Delete mail
                  </button>
                  <button
                    className="mode"
                    disabled={busy}
                    onClick={() => run(() => gmailDeleteStoredMail(account.email).then(() => gmailDisconnect(account.email)))}
                  >
                    Delete mail and disconnect
                  </button>
                  <button className="mode" onClick={() => setConfirmDelete(null)}>
                    Cancel
                  </button>
                </span>
              )}
              {accountNote(account) && <span className="muted small">{accountNote(account)}</span>}
              <span className="muted small">{readStatus(account)}</span>
            </li>
          ))}
        </ul>
      )}
      <div className="toolbar">
        <button className="mode mode--active" disabled={busy || !canConnect(state)} onClick={() => run(gmailConnect)}>
          {connectLabel(state.accounts.length)}
        </button>
      </div>
      <p className="muted small">{GMAIL_MAIL_NOTE}</p>
      <p className="muted small">{GMAIL_PERMISSIONS}</p>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
