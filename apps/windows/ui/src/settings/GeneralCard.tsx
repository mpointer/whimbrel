import { useEffect, useState } from "react";
import { generalSettings, setAutostart } from "../api";
import { describeError, isCommandError } from "../errors";
import type { GeneralSettings } from "../generated/types";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → General (plan Task 10): start at login. */
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
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
