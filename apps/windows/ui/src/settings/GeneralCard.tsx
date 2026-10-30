import { useEffect, useState } from "react";
import { checkForUpdates, generalSettings, installUpdate, setAutostart, updateCheckStatus } from "../api";
import { describeError, isCommandError } from "../errors";
import type { GeneralSettings, UpdateCheck } from "../generated/types";
import { FOLLOW_MS, isBusy, updateLine } from "./updateModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** "Check for Updates", as the Mac's Settings → General has it. Restarting stops and saves a recording
 *  first, as the update banner says. */
function UpdateRow() {
  const [check, setCheck] = useState<UpdateCheck>({ state: "idle" });
  const [restarting, setRestarting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    updateCheckStatus().then(setCheck, () => {});
  }, []);
  useEffect(() => {
    if (!isBusy(check)) return;
    const id = setInterval(() => updateCheckStatus().then(setCheck, () => {}), FOLLOW_MS);
    return () => clearInterval(id);
  }, [check.state]);

  const line = updateLine(check);
  return (
    <div className="stack-sm">
      <div className="field-row">
        <button
          type="button"
          className="mode"
          disabled={isBusy(check) || restarting}
          onClick={() => {
            setError(null);
            checkForUpdates().then(setCheck, (e: unknown) => setError(message(e)));
          }}
        >
          Check for Updates
        </button>
        {line && <span className="muted small">{line}</span>}
        {check.state === "ready" && (
          <button
            type="button"
            className="mode mode--active"
            disabled={restarting}
            onClick={() => {
              setRestarting(true);
              setError(null);
              installUpdate().catch((e: unknown) => {
                setRestarting(false);
                setError(message(e));
              });
            }}
          >
            {restarting ? "Restarting…" : "Restart to Update"}
          </button>
        )}
      </div>
      {check.state === "ready" && <p className="muted small">A meeting being recorded is stopped and saved first.</p>}
      {error && <p role="alert">{error}</p>}
    </div>
  );
}

/** Settings → General (plan Task 10): start at login, and Check for Updates. */
export function GeneralCard() {
  const [settings, setSettings] = useState<GeneralSettings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    generalSettings()
      .then(setSettings)
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const toggle = (enabled: boolean) =>
    setAutostart(enabled)
      .then((s) => {
        setSettings(s);
        setError(null);
      })
      .catch((e: unknown) => setError(message(e)));

  return (
    <section className="card about" aria-labelledby="general-heading">
      <h2 id="general-heading" className="section-label">
        General
      </h2>
      {settings && (
        <>
          <label className="toggle">
            <input
              type="checkbox"
              checked={settings.autostart}
              disabled={!settings.autostartAvailable}
              onChange={(e) => toggle(e.target.checked)}
            />
            Start Whimbrel when you sign in to Windows
          </label>
          {!settings.autostartAvailable && <p className="muted small">Available in release builds only.</p>}
        </>
      )}
      <UpdateRow />
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
