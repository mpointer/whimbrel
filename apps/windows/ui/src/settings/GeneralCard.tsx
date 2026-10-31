import { useEffect, useState } from "react";
import {
  checkForUpdates,
  generalSettings,
  installUpdate,
  languageSettings,
  setAutostart,
  setLanguage as saveLanguage,
  setWriteIn,
  updateCheckStatus,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { GeneralSettings, LanguageSettings, UpdateCheck } from "../generated/types";
import { setLanguage, t } from "../l10n";
import { languageOptions, localModelNote, writeInOptions } from "./languageModel";
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
          {t("Check for Updates")}
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
            {restarting ? t("Restarting…") : t("Restart to Update")}
          </button>
        )}
      </div>
      {check.state === "ready" && <p className="muted small">{t("A meeting being recorded is stopped and saved first.")}</p>}
      {error && <p role="alert">{error}</p>}
    </div>
  );
}

/** The app's language and the one notes are written in, as the Mac's General pane has them. Both stay
 *  on this PC; a change applies at once. */
function LanguageRows() {
  const [settings, setSettings] = useState<LanguageSettings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    languageSettings().then(setSettings, (e: unknown) => setError(message(e)));
  }, []);

  const applied = (s: LanguageSettings) => {
    setSettings(s);
    setError(null);
    setLanguage(s.uiLanguage);
  };

  if (!settings) return error ? <p role="alert">{error}</p> : null;
  const note = localModelNote(settings);
  return (
    <div className="stack-sm">
      <label className="field">
        <span>{t("Language")}</span>
        <select
          className="input"
          value={settings.choice}
          onChange={(e) => saveLanguage(e.target.value).then(applied, (err: unknown) => setError(message(err)))}
        >
          {languageOptions(settings).map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>{t("Write notes and summaries in")}</span>
        <select
          className="input"
          value={settings.writeIn}
          onChange={(e) => setWriteIn(e.target.value).then(applied, (err: unknown) => setError(message(err)))}
        >
          {writeInOptions(settings).map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </label>
      {note && <p className="muted small">{note}</p>}
      <p className="muted small">{t("Routines run on your primary device and write in that device's language.")}</p>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}

/** Settings → General (plan Task 10): start at login, the language, and Check for Updates. */
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
        {t("General")}
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
            {t("Start Whimbrel when you sign in to Windows")}
          </label>
          {!settings.autostartAvailable && <p className="muted small">{t("Available in release builds only.")}</p>}
        </>
      )}
      <LanguageRows />
      <UpdateRow />
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
