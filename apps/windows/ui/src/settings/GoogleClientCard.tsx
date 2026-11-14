import { useEffect, useState } from "react";
import { googleClientClear, googleClientSave, googleClientStatus } from "../api";
import { describeError, isCommandError } from "../errors";
import type { GoogleClientStatus } from "../generated/types";
import { useL10n } from "../l10n";
import { GOOGLE_CLIENT_CHANGED } from "./googleClientModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * Settings → Connectors → Advanced: "Use your own Google Cloud project" (2026-11-12). Public builds
 * ask Google only to send mail; a Desktop OAuth client from the user's own project lets Gmail read
 * too. The secret goes to Credential Manager and is never shown again: the field stays empty.
 */
export function GoogleClientCard() {
  const { t } = useL10n();
  const [status, setStatus] = useState<GoogleClientStatus | null>(null);
  const [clientId, setClientId] = useState("");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    googleClientStatus()
      .then((s) => {
        setStatus(s);
        setClientId(s.clientId ?? "");
      })
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const run = (call: () => Promise<GoogleClientStatus>) => {
    setBusy(true);
    setError(null);
    call()
      .then((s) => {
        setStatus(s);
        setClientId(s.clientId ?? "");
        setSecret("");
        // Connected Google accounts now need signing in again; their cards reload to say so.
        window.dispatchEvent(new Event(GOOGLE_CLIENT_CHANGED));
      })
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const saved = status?.clientId ?? null;

  return (
    <section className="card about">
      <details open={saved != null}>
        <summary className="small">{t("Advanced")}</summary>
        <h3>{t("Use your own Google Cloud project")}</h3>
        <p className="muted small">
          {t(
            "For Gmail reading. Create a Desktop OAuth client in your own Google Cloud project, add yourself as its user, and publish it (an unverified app is fine for yourself).",
          )}
        </p>
        <label className="field">
          <span>{t("Client ID")}</span>
          <input className="input" value={clientId} autoComplete="off" spellCheck={false} onChange={(e) => setClientId(e.target.value)} />
        </label>
        <label className="field">
          <span>{t("Client secret")}</span>
          <input
            className="input"
            type="password"
            value={secret}
            autoComplete="off"
            placeholder={saved ? "••••••••" : undefined}
            onChange={(e) => setSecret(e.target.value)}
          />
        </label>
        <div className="toolbar">
          <button
            className="mode mode--active"
            disabled={busy || !clientId.trim() || !secret.trim()}
            onClick={() => run(() => googleClientSave(clientId, secret))}
          >
            {t("Save")}
          </button>
          <button className="mode" disabled={busy || saved == null} onClick={() => run(googleClientClear)}>
            {t("Clear")}
          </button>
        </div>
        {error && <p role="alert">{error}</p>}
      </details>
    </section>
  );
}
