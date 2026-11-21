import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
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
  syncRegisterRunner,
  syncSetDeviceName,
  routinesCloud,
  syncSignIn,
  syncStatus,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { DeviceView, SyncView } from "../generated/types";
import { useL10n } from "../l10n";
import { ICON_FONT } from "../skinIcons";
import betaQr from "../assets/mobile-beta-qr.svg";
import { PendingDevicesModel, removeDeviceConfirmLine, removeDeviceLabel, tauriPendingDevices } from "../topics/clientControlsModel";
import {
  addRunner,
  addRunnerPrompt,
  isRunner,
  lastRanLine,
  provisionInstructions,
  provisionRows,
  publicKeyLabel,
  registrationError,
  removeRunnerMessage,
  removeRunnerTitle,
  showsAddRunner,
  type RunnerRegistration,
} from "../automations/cloudRoutineModel";
import { collidingPendingIds, contentLine, deviceActions, deviceTitle, indexingLine, offersManage, offersUpgrade, pendingDeviceName, planLine, primaryAction, sellsPlus, settingsLine, stateLabel } from "./syncModel";

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
      {indexingLine(view) && <p className="muted small">{indexingLine(view)}</p>}
      {settingsLine(view, locale) && <p className="muted small">{settingsLine(view, locale)}</p>}
      {view.state === "ready" && <PendingUpdate />}
      {error && <p role="alert">{error}</p>}
      {view.deviceLimit && <LimitUpgrade view={view} busy={busy} act={billing} error={billingError} />}
      {view.state === "ready" && <Plan view={view} busy={busy} act={billing} error={billingError} />}
      {view.state === "ready" && <Devices />}
    </section>
    <PhoneCard />
    </>
  );
}

/** How often the devices-to-update count is read again (each content round stores it; a local read). */
const PENDING_REFRESH_MS = 30_000;

/**
 * "N devices still need to update before they apply client deletes" (spec 13.3), each device named, with "Remove
 * this device…" for a retired one. Nothing for none or unknown.
 */
function PendingUpdate() {
  const { t } = useL10n();
  const [model] = useState(() => new PendingDevicesModel(tauriPendingDevices));
  useSyncExternalStore(model.subscribe, model.getVersion);
  const [devices, setDevices] = useState<DeviceView[]>([]);
  // The names come from the account's device list, a call to the sync server: read when the card opens and after
  // a removal, never on the timer.
  const loadNames = useCallback(() => void syncDevices().then(setDevices, () => {}), []);
  useEffect(() => {
    void model.load();
    loadNames();
    const timer = setInterval(() => void model.load(), PENDING_REFRESH_MS);
    return () => clearInterval(timer);
  }, [model, loadNames]);
  if (!model.line && !model.errorText) return null;
  return (
    <div className="stack">
      {model.line && <p className="small">{model.line}</p>}
      {model.line && (
        <ul className="rows">
          {model.ids.map((id) => (
            <li key={id} className="row-card" style={{ cursor: "default" }}>
              <span className="row-top">
                <span className="row-title" title={id}>
                  {pendingDeviceName(id, devices)}
                </span>
              </span>
              {model.confirming === id ? (
                <span className="toolbar">
                  <span className="small">{removeDeviceConfirmLine()}</span>
                  <button className="mode mode--active" disabled={model.isRemoving} onClick={() => void model.confirm().then(loadNames)}>
                    {t("Remove")}
                  </button>
                  <button className="mode" onClick={() => model.cancel()}>
                    {t("Cancel")}
                  </button>
                </span>
              ) : (
                <span className="toolbar">
                  <button className="mode" disabled={model.isRemoving} onClick={() => model.ask(id)}>
                    {removeDeviceLabel()}
                  </button>
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
      {model.errorText && <p role="alert">{model.errorText}</p>}
    </div>
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

/**
 * The two Upgrade buttons (Cloud v1), shown only on the free plan with billing open. When the server
 * sells Plus they are the Plus ones instead (the shell asks for Plus then); never both.
 */
function UpgradeButtons({ busy, act, plus }: { busy: boolean; act: Act; plus: boolean }) {
  const { t } = useL10n();
  if (plus) {
    return (
      <div className="toolbar">
        <button className="mode mode--active" disabled={busy} onClick={() => act(() => syncUpgrade("month"))}>
          {t("Get Plus: $15 a month")}
        </button>
        <button className="mode" disabled={busy} onClick={() => act(() => syncUpgrade("year"))}>
          {t("Get Plus: $144 a year")}
        </button>
      </div>
    );
  }
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

function Caption({ plus }: { plus: boolean }) {
  const { t } = useL10n();
  if (plus) {
    return (
      <p className="muted small">
        {t("Plus includes hosted sync on all your devices and a monthly Whimbrel AI allowance. Tax is added at checkout where it applies.")}
      </p>
    );
  }
  return <p className="muted small">{t("Plus tax where applicable. Opens Stripe in your browser. Cloud adds unlimited devices on our sync server.")}</p>;
}

/** At the free plan's device limit: the way past it. */
export function LimitUpgrade({ view, busy, act, error }: { view: SyncView; busy: boolean; act: Act; error: string | null }) {
  const { t } = useL10n();
  if (view.planUnknown) return <p className="muted small">{t("Couldn't check your plan. Try again in a moment.")}</p>;
  if (!offersUpgrade(view)) return null;
  const plus = sellsPlus(view);
  return (
    <div className="stack-sm">
      <p>{plus ? t("Upgrade to Plus for unlimited devices.") : t("Upgrade to Cloud for unlimited devices.")}</p>
      <UpgradeButtons busy={busy} act={act} plus={plus} />
      <BillingError error={error} />
      <Caption plus={plus} />
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
          <UpgradeButtons busy={busy} act={act} plus={sellsPlus(view)} />
          <BillingError error={error} />
          <Caption plus={sellsPlus(view)} />
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

/** How often the runner's last round is read again (from the synced settings; a local read). */
const RUNNER_REFRESH_MS = 60_000;

function Devices() {
  const { t, locale } = useL10n();
  const [devices, setDevices] = useState<DeviceView[] | null>(null);
  const [runnerLastRun, setRunnerLastRun] = useState<number | null>(null);
  // The hidden cloud switch (author-only phase): "Add Whimbrel Cloud" shows only with it on.
  const [addRunnerShown, setAddRunnerShown] = useState(false);
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

  useEffect(() => {
    const read = () =>
      void routinesCloud().then(
        (v) => {
          setRunnerLastRun(v.runner_last_run_ms);
          setAddRunnerShown(showsAddRunner(v));
        },
        () => {},
      );
    read();
    const timer = setInterval(read, RUNNER_REFRESH_MS);
    return () => clearInterval(timer);
  }, []);

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
              {isRunner(d) && d.status === "approved" && runnerLastRun !== null && (
                <span className="row-meta muted small">{lastRanLine(runnerLastRun, locale)}</span>
              )}
              {d.status === "pending" && (
                <span className="row-meta small">
                  {d.comparisonCode
                    ? t("Check this matches the code on the new device: {code}", { code: d.comparisonCode })
                    : t("Can't verify this device")}
                  {collisions.has(d.id) && ` ${t("Another pending device shows the same code — don't approve it")}`}
                </span>
              )}
              {confirmRevoke === d.id && isRunner(d) ? (
                <span className="stack-sm">
                  <strong className="small">{removeRunnerTitle()}</strong>
                  <span className="small">{removeRunnerMessage()}</span>
                  <span className="toolbar">
                    <button className="mode mode--active" onClick={() => run(() => syncRevokeDevice(d.id))}>
                      {t("Remove")}
                    </button>
                    <button className="mode" onClick={() => setConfirmRevoke(null)}>
                      {t("Cancel")}
                    </button>
                  </span>
                </span>
              ) : confirmRevoke === d.id ? (
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
                    a === "removeRunner" ? (
                      <button key={a} className="mode" onClick={() => setConfirmRevoke(d.id)}>
                        {t("Remove")}
                      </button>
                    ) : a === "revoke" ? (
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
      {addRunnerShown && <AddRunner onAdded={load} />}
    </div>
  );
}

/**
 * "Add Whimbrel Cloud": the user pastes the runner's public key (`whimbrel-cloud-runner init` prints it); this PC
 * registers the runner and shows, once, the values its host's secret store needs. They live only in this
 * component's state until Done, and are never logged or saved.
 */
export function AddRunner({ onAdded }: { onAdded: () => void }) {
  const { t } = useL10n();
  const [open, setOpen] = useState(false);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [added, setAdded] = useState<Extract<RunnerRegistration, { status: "added" }> | null>(null);

  const close = () => {
    setOpen(false);
    setKey("");
    setError(null);
    setAdded(null);
  };

  const submit = () => {
    setBusy(true);
    setError(null);
    syncRegisterRunner(key)
      .then((r) => {
        if (r.status === "added") {
          setAdded(r);
          setKey("");
          onAdded();
        } else {
          setError(registrationError(r));
        }
      })
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  if (added) {
    return (
      <div className="stack-sm">
        <p className="small">{provisionInstructions()}</p>
        <ul className="rows">
          {provisionRows(added).map(([name, value]) => (
            <li key={name} className="row-card" style={{ cursor: "default" }}>
              <span className="row-top">
                <span className="row-title mono small">{name}</span>
                <button className="mode" onClick={() => void navigator.clipboard?.writeText(value)}>
                  {t("Copy")}
                </button>
              </span>
              <span className="row-meta muted small mono" style={{ overflowWrap: "anywhere" }}>
                {value}
              </span>
            </li>
          ))}
        </ul>
        <div className="toolbar">
          <button className="mode mode--active" onClick={close}>
            {t("Done")}
          </button>
        </div>
      </div>
    );
  }

  if (!open) {
    return (
      <div className="toolbar">
        <button className="mode" onClick={() => setOpen(true)}>
          {addRunner()}
        </button>
      </div>
    );
  }

  return (
    <form
      className="stack-sm"
      onSubmit={(e) => {
        e.preventDefault();
        if (!busy && key.trim()) submit();
      }}
    >
      <p className="small">{addRunnerPrompt()}</p>
      <input className="input mono" value={key} onChange={(e) => setKey(e.target.value)} aria-label={publicKeyLabel()} placeholder={publicKeyLabel()} />
      {error && <p role="alert">{error}</p>}
      <div className="toolbar">
        <button type="button" className="mode" onClick={close}>
          {t("Cancel")}
        </button>
        <button type="submit" className="mode mode--active" disabled={busy || !key.trim()}>
          {busy ? t("Working…") : t("Add")}
        </button>
      </div>
    </form>
  );
}
