import { useCallback, useEffect, useState } from "react";
import {
  openExternal,
  syncApproveDevice,
  syncCancelSignIn,
  syncCheckAgain,
  syncDevices,
  syncRevokeDevice,
  syncSetDeviceName,
  syncSignIn,
  syncStatus,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { DeviceView, SyncView } from "../generated/types";
import { useL10n } from "../l10n";
import { collidingPendingIds, contentLine, deviceActions, deviceTitle, primaryAction, settingsLine, stateLabel } from "./syncModel";

/** The phone app's beta sign-up. The backend opens only pages on its allowlist, and this is one. */
const MOBILE_BETA_URL = "https://whimbrelapp.com/mobile-beta/";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

export function SyncCard() {
  const { t, locale } = useL10n();
  const [view, setView] = useState<SyncView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const refresh = () => syncStatus().then(setView).catch((e: unknown) => setError(message(e)));
    refresh();
    // Sign-in and approval happen in the background; keep the card current.
    const timer = setInterval(refresh, 2000);
    return () => clearInterval(timer);
  }, []);

  const act = (f: () => Promise<SyncView>) => {
    setBusy(true);
    setError(null);
    f()
      .then(setView)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  if (!view) return <section className="card about">{t("Loading…")}</section>;
  const action = primaryAction(view.state);

  return (
    <section className="card about" aria-labelledby="sync-heading">
      <h2 id="sync-heading" className="section-label">
        {t("Sync")}
      </h2>
      <p className="small">
        <button type="button" className="link small" onClick={() => void openExternal(MOBILE_BETA_URL)}>
          {t("Whimbrel for iPhone and Android is in beta. Request to join")}
        </button>
      </p>
      <p role="status">
        <strong>{stateLabel(view.state)}</strong>
        {view.message && <span className="muted"> · {view.message}</span>}
      </p>
      <div className="toolbar">
        {action === "signIn" && (
          <button className="mode mode--active" disabled={busy} onClick={() => act(syncSignIn)}>
            {t("Enable Sync (sign in with Google)")}
          </button>
        )}
        {action === "cancel" && (
          <button className="mode" disabled={busy} onClick={() => act(syncCancelSignIn)}>
            {t("Cancel")}
          </button>
        )}
        {action === "checkAgain" && (
          <button className="mode mode--active" disabled={busy} onClick={() => act(syncCheckAgain)}>
            {t("Check Again")}
          </button>
        )}
      </div>
      {contentLine(view, locale) && <p className="muted small">{contentLine(view, locale)}</p>}
      {settingsLine(view, locale) && <p className="muted small">{settingsLine(view, locale)}</p>}
      {error && <p role="alert">{error}</p>}
      {view.state === "ready" && <Devices />}
    </section>
  );
}

function Devices() {
  const { t } = useL10n();
  const [devices, setDevices] = useState<DeviceView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [confirmRevoke, setConfirmRevoke] = useState<string | null>(null);

  const load = useCallback(
    () =>
      syncDevices()
        .then((d) => {
          setDevices(d);
          setError(null);
        })
        .catch((e: unknown) => setError(message(e))),
    [],
  );

  useEffect(() => {
    load();
    const timer = setInterval(load, 5000); // the Mac's Devices pane refreshes every 5 s too
    return () => clearInterval(timer);
  }, [load]);

  const me = devices?.find((d) => d.thisDevice);
  useEffect(() => {
    if (me && !name) setName(me.name ?? "");
  }, [me, name]);

  const run = (f: () => Promise<DeviceView[]>) =>
    f()
      .then((d) => {
        setDevices(d);
        setError(null);
        setConfirmRevoke(null);
      })
      .catch((e: unknown) => setError(message(e)));

  const collisions = collidingPendingIds(devices ?? []);
  return (
    <div className="stack">
      <h3 className="section-label">{t("Devices on this account")}</h3>
      {me && (
        <form
          className="search"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => syncSetDeviceName(name));
          }}
        >
          <input value={name} onChange={(e) => setName(e.target.value)} aria-label={t("This PC's name")} maxLength={64} />
          <button type="submit" className="mode">
            {t("Save name")}
          </button>
        </form>
      )}
      {error && <p role="alert">{error}</p>}
      {devices === null ? (
        <p className="muted">{t("Loading devices…")}</p>
      ) : (
        <ul className="rows">
          {devices.map((d) => (
            <li key={d.id} className="row-card" style={{ cursor: "default" }}>
              <span className="row-top">
                <span className="row-title">{deviceTitle(d)}</span>
                <span className={d.thisDevice ? "chip chip--this" : "chip"}>{d.thisDevice ? t("This PC") : d.status}</span>
              </span>
              <span className="row-meta muted small mono">{d.publicKey.slice(0, 12)}…</span>
              {d.status === "pending" && (
                <span className="row-meta small">
                  {d.comparisonCode
                    ? t("Check this matches the code on the new device: {code}", { code: d.comparisonCode })
                    : t("Can't verify this device")}
                  {collisions.has(d.id) && ` ${t("Another pending device shows the same code — don't approve it")}`}
                </span>
              )}
              {confirmRevoke === d.id ? (
                <span className="toolbar">
                  <span className="small">{t("Revoking signs every device out of sync until it re-authenticates.")}</span>
                  <button className="mode mode--active" onClick={() => run(() => syncRevokeDevice(d.id))}>
                    {t("Revoke")}
                  </button>
                  <button className="mode" onClick={() => setConfirmRevoke(null)}>
                    {t("Keep")}
                  </button>
                </span>
              ) : (
                <span className="toolbar">
                  {deviceActions(d).map((a) =>
                    a === "revoke" ? (
                      <button key={a} className="mode" onClick={() => setConfirmRevoke(d.id)}>
                        {t("Revoke…")}
                      </button>
                    ) : (
                      <button key={a} className="mode" disabled={a === "approve" && collisions.has(d.id)} onClick={() => run(() => syncApproveDevice(d.id))}>
                        {a === "approve" ? t("Approve") : t("Re-send Key")}
                      </button>
                    ),
                  )}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
