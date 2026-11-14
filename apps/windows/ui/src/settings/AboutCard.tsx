import { useEffect, useState } from "react";
import { appStatus, openExternal, perfReportText, sendFeedback } from "../api";
import { describeError, isCommandError } from "../errors";
import type { AppStatus, FeedbackResult } from "../generated/types";
import { t } from "../l10n";

/** The Whimbrel Discord invite; `open_external` allows only this exact URL (commands/ai.rs DISCORD_URL). */
export const DISCORD_URL = "https://discord.gg/fetZZA8Ax";

export function AboutCard() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [feedbackError, setFeedbackError] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [feedback, setFeedback] = useState<FeedbackResult | null>(null);
  const [reportCopied, setReportCopied] = useState(false);

  useEffect(() => {
    appStatus()
      .then(setStatus)
      .catch((e: unknown) => setError(isCommandError(e) ? describeError(e) : String(e)));
  }, []);

  if (error) return <div className="card">{t("Couldn't read the app status: {error}", { error })}</div>;
  if (!status) return <div className="card">{t("Loading…")}</div>;

  return (
    <section className="card about" aria-labelledby="about-heading">
      <h2 id="about-heading" className="section-label">
        {t("About this PC")}
      </h2>
      <dl>
        <dt>{t("Version")}</dt>
        <dd>{status.version}</dd>
        <dt>{t("Device ID")}</dt>
        <dd className="mono">{status.deviceId ?? t("not available")}</dd>
        <dt>{t("Data folder")}</dt>
        <dd className="mono">{status.dataDir}</dd>
        <dt>{t("Database")}</dt>
        <dd>{status.storeOpen ? t("Open") : t("Not open")}</dd>
        <dt>{t("Semantic search model")}</dt>
        <dd>{status.embeddingModelLoaded ? t("Loaded") : t("Not downloaded yet")}</dd>
      </dl>
      <div className="field-row">
        <button
          type="button"
          className="mode"
          disabled={preparing}
          onClick={() => {
            setFeedbackError(null);
            setFeedback(null);
            setPreparing(true);
            sendFeedback()
              .then(setFeedback, (e: unknown) => setFeedbackError(isCommandError(e) ? describeError(e) : String(e)))
              .finally(() => setPreparing(false));
          }}
        >
          {preparing ? t("Preparing…") : t("Send feedback…")}
        </button>
        <span className="muted small">
          {t(
            "Opens an email to the Whimbrel team and saves a diagnostics file to attach. It has recent logs and settings status, with email addresses and meeting titles removed. No recordings, transcripts or keys.",
          )}
        </span>
      </div>
      {feedback?.bundlePath && <p className="muted small">{t("Saved {file}. Attach it to the email.", { file: feedback.bundlePath })}</p>}
      {feedback?.bundleError && (
        <p className="muted small">{t("Couldn't save the diagnostics file: {error}", { error: feedback.bundleError })}</p>
      )}
      {feedbackError && <p className="muted small">{feedbackError}</p>}
      <div className="field-row">
        <button
          type="button"
          className="mode"
          onClick={() => {
            setReportCopied(false);
            perfReportText()
              .then((text) => navigator.clipboard.writeText(text))
              .then(() => setReportCopied(true), () => {});
          }}
        >
          {t("Copy performance report")}
        </button>
        <span className="muted small">
          {reportCopied
            ? t("Copied. Paste it into an email or message.")
            : t("The last hour of Whimbrel's CPU and memory use, as numbers only: no meeting titles, text or names.")}
        </span>
      </div>
      <div className="field-row">
        <button type="button" className="mode" onClick={() => void openExternal(DISCORD_URL).catch(() => {})}>
          {t("Join the Whimbrel Discord…")}
        </button>
      </div>
      {status.startupErrors.length > 0 && (
        <div role="alert" className="startup-errors">
          <strong>{t("Startup problems")}</strong>
          <ul>
            {status.startupErrors.map((e) => (
              <li key={e}>{e}</li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}
