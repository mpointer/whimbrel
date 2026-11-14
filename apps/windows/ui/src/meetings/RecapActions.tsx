import { useEffect, useState } from "react";
import { recapDraft, recapMailto, recapOpenMail, recapSend, recapSent, recapText, type RecapFormat } from "../api";
import { describeError, isCommandError } from "../errors";
import type { RecapDraft, RecapSentView, WorkspaceView } from "../generated/types";
import { t, useL10n } from "../l10n";
import { addressLooksUsable, chosenRecipients, recapInput, recapSentLine, recapSubject } from "./recapModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * The summary's Copy (plain text, with Markdown and Slack in a menu) and Send recap (the Mac's
 * PR 821). Every text comes from core (`api::recap`), from the summary as the page shows it.
 * Nothing is sent until the user clicks Send in the sheet.
 */
export function RecapActions({ view, summary }: { view: WorkspaceView; summary: string | null }) {
  const [copied, setCopied] = useState(false);
  const [menu, setMenu] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sheet, setSheet] = useState<RecapDraft | null>(null);
  const [sent, setSent] = useState<RecapSentView | null>(null);
  const id = view.row.id;

  useEffect(() => {
    let live = true;
    recapSent(id).then((s) => live && setSent(s), () => {});
    return () => {
      live = false;
    };
  }, [id]);

  const copy = (format: RecapFormat) => {
    setMenu(false);
    setError(null);
    recapText(recapInput(view, summary), format)
      .then((text) => navigator.clipboard.writeText(text))
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      }, (e: unknown) => setError(message(e)));
  };

  const openSheet = () => {
    setError(null);
    recapDraft(id, recapInput(view, summary)).then(setSheet, (e: unknown) => setError(message(e)));
  };

  return (
    <div className="field">
      <div className="toolbar">
        <button className="mode" onClick={() => copy("plain")}>
          {copied ? t("Copied") : t("Copy")}
        </button>
        <span className="speaker-menu">
          <button className="mode" aria-label={t("Copy as Markdown")} aria-haspopup="menu" aria-expanded={menu} onClick={() => setMenu(!menu)}>
            ▾
          </button>
          {menu && (
            <span role="menu" className="speaker-menu-list">
              <button role="menuitem" className="mode" onClick={() => copy("markdown")}>
                {t("Copy as Markdown")}
              </button>
              <button role="menuitem" className="mode" onClick={() => copy("slack")}>
                {t("Copy as Slack")}
              </button>
            </span>
          )}
        </span>
        {!view.readOnly && (
          <button className="mode" onClick={openSheet}>
            {t("Send recap")}
          </button>
        )}
      </div>
      {sent && <p className="muted small">{recapSentLine(sent.recipientCount)}</p>}
      {error && <p role="alert">{error}</p>}
      {sheet && (
        <RecapSheet
          view={view}
          draft={sheet}
          onClose={() => setSheet(null)}
          onSent={(s) => {
            setSent(s);
            setSheet(null);
          }}
        />
      )}
    </div>
  );
}

function RecapSheet({ view, draft, onClose, onSent }: { view: WorkspaceView; draft: RecapDraft; onClose: () => void; onSent: (s: RecapSentView) => void }) {
  const { locale } = useL10n();
  const [ticked, setTicked] = useState<string[]>(draft.recipients.map((r) => r.email));
  const [added, setAdded] = useState<string[]>([]);
  const [typed, setTyped] = useState("");
  const [subject, setSubject] = useState(() => recapSubject(view.row.title, view.row.tsUnixMs, locale));
  const [body, setBody] = useState(draft.body);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const to = chosenRecipients(ticked, added);
  const gmail = draft.gmailFrom;

  const toggle = (email: string) => setTicked(ticked.includes(email) ? ticked.filter((e) => e !== email) : [...ticked, email]);
  const add = () => {
    const email = typed.trim();
    if (!addressLooksUsable(email)) return;
    setAdded([...added, email]);
    setTyped("");
  };

  const send = async () => {
    setError(null);
    if (to.length === 0) {
      setError(t("Add at least one email address."));
      return;
    }
    setBusy(true);
    try {
      if (gmail) {
        onSent(await recapSend(view.row.id, to, subject, body));
        return;
      }
      // No Gmail: a draft in the mail app. A body too long for the link goes on the clipboard
      // first and the draft says to paste it. Nothing is recorded: we can't know it was sent.
      const draftLink = await recapMailto(to, subject, body);
      if (!draftLink.bodyInUrl) await navigator.clipboard.writeText(body);
      try {
        await recapOpenMail(draftLink.url);
        onClose();
      } catch (e) {
        await navigator.clipboard.writeText(body).catch(() => {});
        setError(message(e));
      }
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="card confirm" role="dialog" aria-label={t("Send recap")}>
      <p className="row-title">{t("Send recap")}</p>
      <p className="muted small">{gmail ? t("Sends from {email}.", { email: gmail }) : t("Gmail isn't connected, so this opens in your mail app.")}</p>
      <fieldset className="field">
        <legend>{t("Recipients")}</legend>
        {draft.recipients.length === 0 && added.length === 0 && <p className="muted small">{t("No invitee has an email address. Add one below.")}</p>}
        {draft.recipients.map((r) => (
          <label key={r.email} className="check">
            <input type="checkbox" checked={ticked.includes(r.email)} onChange={() => toggle(r.email)} /> {r.name && r.name !== r.email ? `${r.name} <${r.email}>` : r.email}
          </label>
        ))}
        {added.map((email) => (
          <label key={email} className="check">
            <input type="checkbox" checked onChange={() => setAdded(added.filter((e) => e !== email))} /> {email}
          </label>
        ))}
        <div className="toolbar">
          <input
            className="input"
            type="email"
            placeholder={t("Add an email address")}
            aria-label={t("Add an email address")}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && add()}
          />
          <button className="mode" disabled={!addressLooksUsable(typed)} onClick={add}>
            {t("Add")}
          </button>
        </div>
      </fieldset>
      <label className="field">
        <span>{t("Email subject")}</span>
        <input className="input" value={subject} onChange={(e) => setSubject(e.target.value)} />
      </label>
      <textarea className="input" rows={10} aria-label={t("Send recap")} value={body} onChange={(e) => setBody(e.target.value)} />
      {error && <p role="alert">{error}</p>}
      <div className="toolbar">
        <button className="mode mode--active" disabled={busy || to.length === 0} onClick={() => void send()}>
          {gmail ? t("Send") : t("Open in Mail")}
        </button>
        <button className="mode" disabled={busy} onClick={onClose}>
          {t("Cancel")}
        </button>
      </div>
    </div>
  );
}
