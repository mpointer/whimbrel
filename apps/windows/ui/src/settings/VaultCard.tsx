import { useEffect, useState } from "react";
import { chooseVaultFolder, setVaultEnabled, vaultSettings } from "../api";
import { describeError, isCommandError } from "../errors";
import type { VaultView } from "../generated/types";
import { useL10n } from "../l10n";
import { relativeTime } from "./statusModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * Settings → Capture & Privacy: "Keep an Obsidian vault up to date" (the Mac's Your data pane, PR 681).
 * Whimbrel writes each topic's page, its people, organizations and meetings into `<folder>/Whimbrel/`
 * every 5 minutes, keeping your own notes.
 */
export function VaultCard() {
  const { t, locale } = useL10n();
  const [view, setView] = useState<VaultView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () => vaultSettings().then(setView, (e: unknown) => setError(message(e)));
    load();
    // The loop writes in the background: keep "Last updated" and a failure current.
    const timer = setInterval(load, 30_000);
    return () => clearInterval(timer);
  }, []);

  if (!view) return null;
  const run = (p: Promise<VaultView>) =>
    p.then(
      (v) => {
        setView(v);
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );
  const chosen = view.path.trim() !== "";

  return (
    <section className="card about" aria-labelledby="vault-heading">
      <h2 id="vault-heading" className="section-label">
        {t("Obsidian")}
      </h2>
      <label className="toggle">
        <input type="checkbox" checked={view.enabled} onChange={(e) => run(setVaultEnabled(e.target.checked))} />
        {t("Keep an Obsidian vault up to date")}
      </label>
      <div className="field-row">
        <button type="button" className="mode" onClick={() => run(chooseVaultFolder())}>
          {t("Choose folder…")}
        </button>
        {chosen && (
          <span className="muted small mono" title={view.path}>
            {view.path}
          </span>
        )}
      </div>
      {view.enabled && chosen && view.lastUpdatedUnixMs != null && (
        <p className="muted small">{t("Last updated {time}", { time: relativeTime(view.lastUpdatedUnixMs, Date.now(), locale) })}</p>
      )}
      {view.enabled && chosen && view.failed && (
        <p role="alert" className="small">
          {t("Couldn't update the vault. Check that the folder exists and Whimbrel can write to it.")}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
