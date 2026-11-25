import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { syncCancelSignIn, syncCheckAgain, syncDevices, syncSignIn, syncUpgrade } from "../../../../api";
import type { DeviceView, SyncView } from "../../../../generated/types";
import { t, useL10n } from "../../../../l10n";
import { WButton, WInlineConfirm, WNoticeBox, WSettingsCard, WSettingsControlRow, WSettingsList, WSettingsListRow, WSettingsNote, WSettingsStatusRow } from "../../../../system";
import { PendingDevicesModel, removeDeviceConfirmLine, removeDeviceLabel, tauriPendingDevices } from "../../../../topics/clientControlsModel";
import { purchaseOffers } from "../../../PlanControls";
import { contentLine, indexingLine, offersUpgrade, pendingDeviceName, primaryAction, sellsPlus, settingsLine, stateLabel } from "../../../syncModel";
import { syncTone } from "./devicesModel";
import type { SyncStatus } from "./useSyncStatus";

/** How often the devices-to-update count is read again (each content round stores it; a local read). */
const PENDING_REFRESH_MS = 30_000;

/** Sync: whether it is on, the one button for the state it is in, what last synced, devices that still need to update, and the way past the free plan's device limit. */
export function SyncStatusCard({ sync }: { sync: SyncStatus }) {
  const { locale } = useL10n();
  const { view, error, busy, act } = sync;
  const action = view ? primaryAction(view.state) : "none";
  const notes = view ? [contentLine(view, locale), indexingLine(view), settingsLine(view, locale)].filter((l): l is string => l !== null) : [];
  return (
    <WSettingsCard id="devices.sync" title={t("Sync")} headingLevel={3} loading={!view} error={error}>
      {view && (
        <>
          <WSettingsStatusRow
            label={t("Status")}
            live
            description={view.message ?? undefined}
            status={{ tone: syncTone(view.state), children: stateLabel(view.state) }}
            action={
              action === "signIn" ? (
                <WButton variant="primary" disabled={busy} onClick={() => act(syncSignIn)}>
                  {t("Enable Sync (sign in with Google)")}
                </WButton>
              ) : action === "cancel" ? (
                <WButton disabled={busy} onClick={() => act(syncCancelSignIn)}>
                  {t("Cancel")}
                </WButton>
              ) : action === "checkAgain" ? (
                <WButton variant="primary" disabled={busy} onClick={() => act(syncCheckAgain)}>
                  {t("Check Again")}
                </WButton>
              ) : undefined
            }
          />
          {notes.map((line) => (
            <WSettingsNote key={line}>{line}</WSettingsNote>
          ))}
          {view.state === "ready" && <PendingUpdate />}
          {view.deviceLimit && <LimitUpgrade view={view} sync={sync} />}
        </>
      )}
    </WSettingsCard>
  );
}

/** At the free plan's device limit: the way past it, the same commands as the old card (syncUpgrade month or year). */
export function LimitUpgrade({ view, sync }: { view: SyncView; sync: Pick<SyncStatus, "busy" | "billing" | "billingError"> }) {
  if (view.planUnknown) return <WSettingsNote>{t("Couldn't check your plan. Try again in a moment.")}</WSettingsNote>;
  if (!offersUpgrade(view)) return null;
  const plus = sellsPlus(view);
  const { busy, billing, billingError } = sync;
  return (
    <WSettingsControlRow
      label={plus ? t("Upgrade to Plus for unlimited devices.") : t("Upgrade to Cloud for unlimited devices.")}
      description={
        plus
          ? t("Plus includes hosted sync on all your devices and a monthly Whimbrel AI allowance. Tax is added at checkout where it applies.")
          : t("Plus tax where applicable. Opens Stripe in your browser. Cloud adds unlimited devices on our sync server.")
      }
      error={billingError}
    >
      <span className="rs-devconn-actions">
        {/* The prices and the words are the plan section's own offers (purchaseOffers), not a second copy of them. */}
        {purchaseOffers(plus).map((o) => (
          <WButton key={o.interval} variant={o.primary ? "primary" : "secondary"} disabled={busy} onClick={() => billing(() => syncUpgrade(o.interval))}>
            {o.label}
          </WButton>
        ))}
      </span>
    </WSettingsControlRow>
  );
}

/**
 * "N devices still need to update before they apply client deletes" (spec 13.3), each device named, with "Remove
 * this device…" for a retired one. Nothing for none or unknown.
 */
export function PendingUpdate() {
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
  if (!model.line) return <WNoticeBox tone="error">{model.errorText}</WNoticeBox>;
  return (
    <WSettingsList label={model.line} error={model.errorText}>
      {model.ids.map((id) => (
        <WSettingsListRow
          key={id}
          title={pendingDeviceName(id, devices)}
          actions={
            model.confirming === id ? (
              <WInlineConfirm
                question={removeDeviceConfirmLine()}
                confirmLabel={t("Remove")}
                busy={model.isRemoving}
                onConfirm={() =>
                  void model.confirm().then(() => {
                    loadNames();
                    // The row that had focus is gone: focus goes to the list's first remaining button, or the card's first button.
                    setTimeout(() => {
                      try {
                        (document.querySelector<HTMLElement>("#devices\\.sync .w-lrow button") ?? document.querySelector<HTMLElement>("#devices\\.sync button"))?.focus();
                      } catch {
                        // No page to focus in (a test stand-in): nothing to do.
                      }
                    }, 0);
                  })
                }
                onCancel={() => model.cancel()}
                returnFocusTo={`rs-pend-${id}`}
              />
            ) : (
              <WButton id={`rs-pend-${id}`} disabled={model.isRemoving} onClick={() => model.ask(id)}>
                {removeDeviceLabel()}
              </WButton>
            )
          }
        />
      ))}
    </WSettingsList>
  );
}
