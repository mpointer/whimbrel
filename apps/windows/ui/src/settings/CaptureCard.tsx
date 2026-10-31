import { useEffect, useState } from "react";
import { captureSettings, captureStatus, librarySettings, setCaptureEnabled, setExcludedApps, setShowScreenCaptures } from "../api";
import { describeError, isCommandError } from "../errors";
import type { CaptureSettings, CaptureStatus } from "../generated/types";
import { t } from "../l10n";
import { captureSummary } from "./captureSummary";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

export function CaptureCard() {
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  const [showCaptures, setShowCaptures] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const refresh = () => captureStatus().then(setStatus).catch((e: unknown) => setError(String(e)));
    refresh();
    const timer = setInterval(refresh, 5000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    librarySettings()
      .then((s) => setShowCaptures(s.showScreenCaptures))
      .catch(() => {});
  }, []);

  const toggle = (enabled: boolean) =>
    setCaptureEnabled(enabled)
      .then(setStatus)
      .catch((e: unknown) => setError(message(e)));

  const toggleShow = (show: boolean) =>
    setShowScreenCaptures(show)
      .then((s) => setShowCaptures(s.showScreenCaptures))
      .catch((e: unknown) => setError(message(e)));

  return (
    <section className="card about" aria-labelledby="capture-heading">
      <h2 id="capture-heading" className="section-label">
        {t("Activity capture")}
      </h2>
      {status ? (
        <>
          <label className="toggle">
            <input type="checkbox" checked={status.enabled} onChange={(e) => toggle(e.target.checked)} />
            {t("Capture the text of the window you're working in")}
          </label>
          {showCaptures !== null && (
            <label className="toggle">
              <input type="checkbox" checked={showCaptures} onChange={(e) => toggleShow(e.target.checked)} />
              {t("Show screen captures in the Library")}
            </label>
          )}
          <p role="status" className="muted">
            {captureSummary(status)}
          </p>
          <dl>
            <dt>{t("Stored")}</dt>
            <dd>{status.stored}</dd>
            <dt>{t("Skipped (unchanged)")}</dt>
            <dd>{status.unchanged}</dd>
            <dt>{t("Skipped (excluded apps)")}</dt>
            <dd>{status.excluded}</dd>
            <dt>{t("OCR fallbacks")}</dt>
            <dd>{status.ocrRuns}</dd>
          </dl>
          <p className="muted small">
            {t(
              "Never captured: the lock screen, sign-in and UAC prompts, password managers, InPrivate and Incognito windows, and Whimbrel itself. Screenshots are never saved.",
            )}
          </p>
          <Exclusions onError={setError} />
        </>
      ) : (
        <p>{t("Loading…")}</p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}

function Exclusions(props: { onError: (e: string | null) => void }) {
  const [settings, setSettings] = useState<CaptureSettings | null>(null);
  const [pick, setPick] = useState("");

  const load = () =>
    captureSettings()
      .then(setSettings)
      .catch((e: unknown) => props.onError(message(e)));
  useEffect(() => {
    load();
  }, []);

  if (!settings) return null;
  const excludedLower = new Set(settings.excluded.map((e) => e.toLowerCase()));
  const candidates = settings.runningApps.filter((a) => !excludedLower.has(a.appId.toLowerCase()));

  const save = (excluded: string[]) =>
    setExcludedApps(excluded)
      .then((list) => {
        setSettings({ ...settings, excluded: list });
        props.onError(null);
      })
      .catch((e: unknown) => props.onError(message(e)));

  const ocr = settings.ocr;
  return (
    <div className="stack-sm">
      <h3 className="section-label">{t("Excluded apps")}</h3>
      {settings.excluded.length === 0 ? (
        <p className="muted small">{t("No apps excluded.")}</p>
      ) : (
        <ul className="rows">
          {settings.excluded.map((id) => (
            <li key={id} className="field-row">
              <span className="mono grow">{id}</span>
              <button className="mode" onClick={() => save(settings.excluded.filter((e) => e !== id))}>
                {t("Remove")}
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        className="field-row"
        onSubmit={(e) => {
          e.preventDefault();
          if (pick) save([...settings.excluded, pick]).then(() => setPick(""));
        }}
      >
        <select className="input grow" value={pick} onChange={(e) => setPick(e.target.value)} aria-label={t("App to exclude")}>
          <option value="">{t("Choose an open app…")}</option>
          {candidates.map((a) => (
            <option key={a.appId} value={a.appId}>
              {a.exe} — {a.title.length > 50 ? `${a.title.slice(0, 50)}…` : a.title}
            </option>
          ))}
        </select>
        <button type="submit" className="mode" disabled={!pick}>
          {t("Exclude")}
        </button>
        <button type="button" className="mode" onClick={load}>
          {t("Refresh")}
        </button>
      </form>
      <h3 className="section-label">{t("Text recognition (OCR)")}</h3>
      {ocr.covered ? (
        <p className="muted small">{t("OCR languages on this PC: {languages}.", { languages: ocr.available.join(", ") })}</p>
      ) : (
        <p role="alert" className="small">
          {ocr.profile[0] != null
            ? t(
                "Windows has no OCR language pack for {language}, so apps without accessible text (some video and remote-desktop windows) won't be captured. Add the language under Settings → Time & language → Language & region, including \"Optical character recognition\".",
                { language: ocr.profile[0] },
              )
            : t(
                "Windows has no OCR language pack for your display language, so apps without accessible text (some video and remote-desktop windows) won't be captured. Add the language under Settings → Time & language → Language & region, including \"Optical character recognition\".",
              )}
          {ocr.available.length > 0 && ` ${t("Installed: {languages}.", { languages: ocr.available.join(", ") })}`}
        </p>
      )}
    </div>
  );
}
