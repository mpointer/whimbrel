// The tray popup's timed pause (redesign flag on only). A paused capture says when it comes back and offers Resume
// now; a running one offers the three timed pauses in one list, small enough for the popup's size.

import { useState } from "react";
import "./shell.css";
import { capturePauseUntil, captureResumeNow } from "../../api";
import type { TrayStatus } from "../../generated/types";
import { useL10n } from "../../l10n";
import { pauseChoiceLabel } from "./captureChoices";
import { PAUSE_CHOICES, captureStateOf, pauseDeadline, pauseEndLabel, type PauseChoice } from "./captureModel";

export function PausePicker({ status, now, onChanged }: { status: Pick<TrayStatus, "captureEnabled" | "resumeAtUnixMs">; now: number; onChanged: () => void }) {
  const { t, locale } = useL10n();
  const [busy, setBusy] = useState(false);
  const state = captureStateOf({ enabled: status.captureEnabled, resumeAtUnixMs: status.resumeAtUnixMs }, now);
  if (!state || state.kind === "off") return null;
  const run = (call: () => Promise<unknown>) => {
    setBusy(true);
    call()
      .then(onChanged)
      .catch(() => {})
      .finally(() => setBusy(false));
  };
  if (state.kind === "paused") {
    return (
      <div className="popup-pause">
        <span role="status">{t("Paused until {time}", { time: pauseEndLabel(state.resumeAt, now, locale) })}</span>
        <button type="button" className="mode" disabled={busy} onClick={() => run(captureResumeNow)}>
          {t("Resume now")}
        </button>
      </div>
    );
  }
  return (
    <div className="popup-pause">
      <select
        aria-label={t("Pause memory")}
        value=""
        disabled={busy}
        onChange={(e) => {
          const choice = e.target.value as PauseChoice;
          if (choice) run(() => capturePauseUntil(pauseDeadline(choice, Date.now())));
        }}
      >
        <option value="">{t("Pause memory")}</option>
        {PAUSE_CHOICES.map((c) => (
          <option key={c} value={c}>
            {pauseChoiceLabel(c)}
          </option>
        ))}
      </select>
    </div>
  );
}
