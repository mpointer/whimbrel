import { useCallback, useEffect, useState } from "react";
import {
  openExternal,
  syncApproveDevice,
  syncCancelSignIn,
  syncCheckAgain,
  syncDevices,
  syncManageSubscription,
  syncRefreshPlan,
  syncUpgrade,
  syncRevokeDevice,
  syncSetDeviceName,
  syncSignIn,
  syncStatus,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { DeviceView, SyncView } from "../generated/types";
import { useL10n } from "../l10n";
import { ICON_FONT } from "../skinIcons";
import betaQr from "../assets/mobile-beta-qr.svg";
import { collidingPendingIds, contentLine, deviceActions, deviceTitle, offersManage, offersUpgrade, planLine, primaryAction, settingsLine, stateLabel } from "./syncModel";

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
  // Upgrade and Manage report under their own buttons: at the top of the card an error was off screen.
  const [billingError, setBillingError] = useState<string | null>(null);

  useEffect(() => {
    const refresh = () => syncStatus().then(setView).catch((e: unknown) => setError(message(e)));
    refresh();
    // Sign-in and approval happen in the background; keep the card current.
    const timer = setInterval(refresh, 2000);
    // The plan is read when the card opens and when the window comes back (from Stripe, say).
    const plan = () => void syncRefreshPlan().then(setView, () => {});
    plan();
    window.addEventListener("focus", plan);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", plan);
    };
  }, []);

  const act = (f: () => Promise<SyncView>) => {
    setBusy(true);
    setError(null);
    f()
      .then(setView)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const billing = (f: () => Promise<SyncView>) => {
    setBusy(true);
    setBillingError(null);
    f()
      .then(setView)
      .catch((e: unknown) => setBillingError(message(e)))
      .finally(() => setBusy(false));
  };

  if (!view) return <section className="card about">{t("Loading…")}</section>;
  const action = primaryAction(view.state);

  return (
    <>
    <section className="card about" aria-labelledby="sync-heading">
      <h2 id="sync-heading" className="section-label">
        {t("Sync")}
      </h2>
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
      {view.deviceLimit && <LimitUpgrade view={view} busy={busy} act={billing} error={billingError} />}
      {view.state === "ready" && <Plan view={view} busy={busy} act={billing} error={billingError} />}
      {view.state === "ready" && <Devices />}
    </section>
    <PhoneCard />
    </>
  );
}

/** The phone apps' beta (the Mac's card in Devices): a button, and a QR code to scan with the phone. */
export function PhoneCard() {
  const { t } = useL10n();
  return (
    <section className="card about phone-card" aria-labelledby="phone-heading">
      <span className="phone-card__icon" style={{ fontFamily: ICON_FONT }} aria-hidden="true">
        {"\uE8EA"}
      </span>
      <div className="phone-card__text">
        <h2 id="phone-heading" className="section-label">
          {t("Whimbrel on your phone")}
        </h2>
        <p className="small">{t("Read your meeting notes, decisions and action items on iPhone or Android. The apps are in beta.")}</p>
        <button type="button" className="mode mode--active" onClick={() => void openExternal(MOBILE_BETA_URL)}>
          {t("Join the beta")}
        </button>
      </div>
      <figure className="phone-card__qr">
        <img src={betaQr} width={84} height={84} alt={t("QR code for the phone beta page")} />
        <figcaption className="muted small">{t("Scan with your phone")}</figcaption>
      </figure>
    </section>
  );
}

type Act = (f: () => Promise<SyncView>) => void;

/** What Upgrade or Manage subscription last reported, right under the buttons. */
function BillingError({ error }: { error: string | null }) {
  return error ? (
    <p role="alert" className="small">
      {error}
    </p>
  ) : null;
}

/** The two Upgrade buttons (Cloud v1), shown only on the free plan with billing open. */
function UpgradeButtons({ busy, act }: { busy: boolean; act: Act }) {
  const { t } = useL10n();
  return (
    <div className="toolbar">
      <button className="mode mode--active" disabled={busy} onClick={() => act(() => syncUpgrade("month"))}>
        {t("Upgrade to Cloud: $5 a month")}
      </button>
      <button className="mode" disabled={busy} onClick={() => act(() => syncUpgrade("year"))}>
        {t("Upgrade to Cloud: $50 a year")}
      </button>
    </div>
  );
}

function Caption() {
  const { t } = useL10n();
  return <p className="muted small">{t("Plus tax where applicable. Opens Stripe in your browser. Cloud adds unlimited devices on our sync server.")}</p>;
}

/** At the free plan's device limit: the way past it. */
export function LimitUpgrade({ view, busy, act, error }: { view: SyncView; busy: boolean; act: Act; error: string | null }) {
  const { t } = useL10n();
  if (view.planUnknown) return <p className="muted small">{t("Couldn't check your plan. Try again in a moment.")}</p>;
  if (!offersUpgrade(view)) return null;
  return (
    <div className="stack-sm">
      <p>{t("Upgrade to Cloud for unlimited devices.")}</p>
      <UpgradeButtons busy={busy} act={act} />
      <BillingError error={error} />
      <Caption />
    </div>
  );
}

/** Settings → Sync & Devices' Plan block: the plan, then Upgrade or Manage. */
export function Plan({ view, busy, act, error }: { view: SyncView; busy: boolean; act: Act; error: string | null }) {
  const { t, locale } = useL10n();
  if (!view.plan) return null;
  return (
    <div className="stack-sm" aria-labelledby="plan-heading">
      <h3 id="plan-heading" className="section-label">
        {t("Plan")}
      </h3>
      <p>{planLine(view.plan, locale)}</p>
      {offersUpgrade(view) && (
        <>
          <UpgradeButtons busy={busy} act={act} />
          <BillingError error={error} />
          <Caption />
        </>
      )}
      {offersManage(view) && (
        <div className="toolbar">
          <button className="mode" disabled={busy} onClick={() => act(syncManageSubscription)}>
            {t("Manage subscription")}
          </button>
        </div>
      )}
      {offersManage(view) && <BillingError error={error} />}
    </div>
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
