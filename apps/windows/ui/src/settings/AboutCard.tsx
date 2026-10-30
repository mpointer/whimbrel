import { useEffect, useState } from "react";
import { appStatus, sendFeedback } from "../api";
import { describeError, isCommandError } from "../errors";
import type { AppStatus } from "../generated/types";

export function AboutCard() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [feedbackError, setFeedbackError] = useState<string | null>(null);

  useEffect(() => {
    appStatus()
      .then(setStatus)
      .catch((e: unknown) => setError(isCommandError(e) ? describeError(e) : String(e)));
  }, []);

  if (error) return <div className="card">Couldn't read the app status: {error}</div>;
  if (!status) return <div className="card">Loading…</div>;

  return (
    <section className="card about" aria-labelledby="about-heading">
      <h2 id="about-heading" className="section-label">
        About this PC
      </h2>
      <dl>
        <dt>Version</dt>
        <dd>{status.version}</dd>
        <dt>Device ID</dt>
        <dd className="mono">{status.deviceId ?? "not available"}</dd>
        <dt>Data folder</dt>
        <dd className="mono">{status.dataDir}</dd>
        <dt>Database</dt>
        <dd>{status.storeOpen ? "Open" : "Not open"}</dd>
        <dt>Semantic search model</dt>
        <dd>{status.embeddingModelLoaded ? "Loaded" : "Not downloaded yet"}</dd>
      </dl>
      <div className="field-row">
        <button
          type="button"
          className="mode"
          onClick={() => {
            setFeedbackError(null);
            sendFeedback().catch((e: unknown) => setFeedbackError(isCommandError(e) ? describeError(e) : String(e)));
          }}
        >
          Send feedback…
        </button>
        <span className="muted small">Opens an email to the Whimbrel team with this PC's version filled in.</span>
      </div>
      {feedbackError && <p className="muted small">{feedbackError}</p>}
      {status.startupErrors.length > 0 && (
        <div role="alert" className="startup-errors">
          <strong>Startup problems</strong>
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
