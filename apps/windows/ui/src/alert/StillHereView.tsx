import { useState } from "react";
import { stillHereKeep, stillHereStop } from "../api";
import { t } from "../l10n";

/**
 * The "Still in a meeting?" window: a recording started by hand has heard no words for 15
 * minutes. Keep recording asks again after 15 more; Stop (or two minutes with no answer, decided
 * by the shell) stops it, trimmed to the last words once transcribed.
 */
export function StillHereView() {
  const [busy, setBusy] = useState(false);
  const run = (action: () => Promise<void>) => {
    setBusy(true);
    action().catch(() => setBusy(false));
  };
  return (
    <div className="alert">
      <span className="popup-brand">{"Whimbrel"}</span>
      <p role="status" className="alert-title">
        {t("Still in a meeting?")}
      </p>
      <p className="muted small">{t("No one has spoken for 15 minutes.")}</p>
      <div className="popup-actions">
        <button className="mode mode--active" disabled={busy} onClick={() => run(stillHereKeep)}>
          {t("Keep recording")}
        </button>
        <button className="mode" disabled={busy} onClick={() => run(stillHereStop)}>
          {t("Stop")}
        </button>
      </div>
    </div>
  );
}
