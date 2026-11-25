import { useCallback, useEffect, useState } from "react";
import { routinesCloud, syncApproveDevice, syncRegisterRunner, syncRevokeDevice, syncSetDeviceName, syncDevices } from "../../../../api";
import {
  addRunner,
  addRunnerPrompt,
  provisionInstructions,
  provisionRows,
  publicKeyLabel,
  registrationError,
  removeRunnerMessage,
  removeRunnerTitle,
  showsAddRunner,
  isRunner,
  type RunnerRegistration,
} from "../../../../automations/cloudRoutineModel";
import type { DeviceView } from "../../../../generated/types";
import { t, useL10n } from "../../../../l10n";
import { WButton, WInlineConfirm, WSettingsCard, WSettingsControlRow, WSettingsList, WSettingsListRow, WSettingsValueList } from "../../../../system";
import { collidingPendingIds, deviceActions, deviceTitle } from "../../../syncModel";
import { deviceActionName, deviceChip, deviceLines, deviceNotes } from "./devicesModel";
import { message } from "./useSyncStatus";
import "./devices.css";

/** How often the runner's last round is read again (from the synced settings; a local read). */
const RUNNER_REFRESH_MS = 60_000;

/** Devices on this account: this PC's name, then each device with approve, re-send key and revoke (each revoke asks first). Shown only while sync is on. */
export function DevicesListCard() {
  const { locale } = useL10n();
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
    void load();
    const timer = setInterval(() => void load(), 5000); // the Mac's Devices pane refreshes every 5 s too
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

  const run = (f: () => Promise<DeviceView[]>, afterRemoval = false) =>
    f()
      .then((d) => {
        setDevices(d);
        setError(null);
        setConfirmRevoke(null);
        // The row that had focus is gone: focus goes to this PC's name box, the top of the list, not to the page.
        if (afterRemoval) setTimeout(() => document.getElementById("rs-devices-name")?.focus(), 0);
      })
      .catch((e: unknown) => setError(message(e)));

  const collisions = collidingPendingIds(devices ?? []);
  return (
    <WSettingsCard id="devices.list" title={t("Devices on this account")} headingLevel={3} loading={devices === null && error === null} loadingLabel={t("Loading devices…")} error={error}>
      {me && (
        <WSettingsControlRow label={t("This PC's name")} stacked>
          <form
            className="rs-devconn-name"
            onSubmit={(e) => {
              e.preventDefault();
              run(() => syncSetDeviceName(name));
            }}
          >
            <input id="rs-devices-name" className="w-input" value={name} onChange={(e) => setName(e.target.value)} aria-label={t("This PC's name")} maxLength={64} autoComplete="off" />
            <WButton type="submit">{t("Save name")}</WButton>
          </form>
        </WSettingsControlRow>
      )}
      {devices !== null && (
        <WSettingsList label={t("Devices")} isEmpty={devices.length === 0} empty={t("None yet.")}>
          {devices.map((d) => {
            const chip = deviceChip(d);
            const runner = isRunner(d);
            const asking = confirmRevoke === d.id;
            return (
              <WSettingsListRow
                key={d.id}
                title={deviceTitle(d)}
                chip={{ tone: chip.tone, children: chip.text }}
                details={deviceNotes(d, { collides: collisions.has(d.id) }).map((n) => ({ text: n.text, id: n.warning ? `rs-dev-${d.id}-warn` : undefined }))}
                caption={deviceLines(d, { runnerLastRun, locale }).join(" · ")}
                actions={
                  asking ? (
                    <WInlineConfirm
                      question={runner ? `${removeRunnerTitle()} ${removeRunnerMessage()}` : t("Revoking signs every device out of sync until it re-authenticates.")}
                      confirmLabel={runner ? t("Remove") : t("Revoke")}
                      cancelLabel={runner ? t("Cancel") : t("Keep")}
                      onConfirm={() => run(() => syncRevokeDevice(d.id), true)}
                      onCancel={() => setConfirmRevoke(null)}
                      returnFocusTo={`rs-dev-${d.id}-revoke`}
                    />
                  ) : (
                    <>
                      {deviceActions(d).map((a) =>
                        a === "removeRunner" ? (
                          <WButton key={a} id={`rs-dev-${d.id}-revoke`} aria-label={deviceActionName(t("Remove"), d)} onClick={() => setConfirmRevoke(d.id)}>
                            {t("Remove")}
                          </WButton>
                        ) : a === "revoke" ? (
                          <WButton key={a} id={`rs-dev-${d.id}-revoke`} aria-label={deviceActionName(t("Revoke…"), d)} onClick={() => setConfirmRevoke(d.id)}>
                            {t("Revoke…")}
                          </WButton>
                        ) : (
                          <WButton
                            key={a}
                            aria-label={deviceActionName(a === "approve" ? t("Approve") : t("Re-send Key"), d)}
                            // While two pending devices show one code, Approve is neither the default-looking button nor on: the warning says why.
                            variant={a === "approve" && !collisions.has(d.id) ? "primary" : "secondary"}
                            disabled={a === "approve" && collisions.has(d.id)}
                            aria-describedby={a === "approve" && collisions.has(d.id) ? `rs-dev-${d.id}-warn` : undefined}
                            onClick={() => run(() => syncApproveDevice(d.id))}
                          >
                            {a === "approve" ? t("Approve") : t("Re-send Key")}
                          </WButton>
                        ),
                      )}
                    </>
                  )
                }
              />
            );
          })}
        </WSettingsList>
      )}
      {addRunnerShown && <AddRunner onAdded={() => void load()} />}
    </WSettingsCard>
  );
}

/**
 * "Add Whimbrel Cloud": the user pastes the runner's public key (`whimbrel-cloud-runner init` prints it); this PC
 * registers the runner and shows, once, the values its host's secret store needs. They live only in this
 * component's state until Done, and are never logged or saved.
 */
export function AddRunner({ onAdded }: { onAdded: () => void }) {
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
      <WSettingsControlRow label={t("Whimbrel Cloud")} description={provisionInstructions()} stacked>
        <div className="rs-devconn-stack">
          <WSettingsValueList label={t("Whimbrel Cloud")} items={provisionRows(added).map(([n, value]) => ({ key: n, label: n, value, copy: true, mono: true }))} />
          <div className="rs-devconn-actions">
            <WButton variant="primary" onClick={close}>
              {t("Done")}
            </WButton>
          </div>
        </div>
      </WSettingsControlRow>
    );
  }

  if (!open) {
    return (
      <WSettingsControlRow label={t("Whimbrel Cloud")}>
        <WButton onClick={() => setOpen(true)}>{addRunner()}</WButton>
      </WSettingsControlRow>
    );
  }

  return (
    <WSettingsControlRow label={publicKeyLabel()} description={addRunnerPrompt()} error={error} stacked>
      <form
        className="rs-devconn-name"
        onSubmit={(e) => {
          e.preventDefault();
          if (!busy && key.trim()) submit();
        }}
      >
        <input className="w-input w-input-mono" value={key} onChange={(e) => setKey(e.target.value)} aria-label={publicKeyLabel()} placeholder={publicKeyLabel()} autoComplete="off" spellCheck={false} />
        <WButton type="button" onClick={close}>
          {t("Cancel")}
        </WButton>
        <WButton type="submit" variant="primary" disabled={busy || !key.trim()}>
          {busy ? t("Working…") : t("Add")}
        </WButton>
      </form>
    </WSettingsControlRow>
  );
}
