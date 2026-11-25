import { useEffect, useState } from "react";
import { meetingRecordResume, meetingRecordStop, meetingRecordingStatus, stillHereKeep, stillHereStop } from "../api";
import { t, useL10n } from "../l10n";
import { timeLabel } from "../library/rows";
import { useRedesign } from "../redesign";
import { WButton, WINDOW_DESIGN, WWindowFrame, mainActionOnReturn } from "../system";

/**
 * The "Still in a meeting?" window: a recording started by hand has heard no words for 15
 * minutes. Keep recording asks again after 15 more; Stop (or two minutes with no answer, decided
 * by the shell) stops it, trimmed to the last words once transcribed.
 */
export function StillHereView() {
  const { locale } = useL10n();
  const [busy, setBusy] = useState(false);
  // A paused recording asks with its own words and never stops itself (redesign only; with it off nothing is read).
  const redesign = useRedesign();
  const [paused, setPaused] = useState(false);
  const [pausedSince, setPausedSince] = useState<number | null>(null);
  useEffect(() => {
    if (redesign)
      meetingRecordingStatus().then(
        (v) => {
          setPaused(v.paused);
          setPausedSince(v.pausedSinceUnixMs ?? null);
        },
        () => {},
      );
  }, [redesign]);
  const run = (action: () => Promise<void>) => {
    setBusy(true);
    action().catch(() => setBusy(false));
  };
  if (redesign) {
    // Escape never stops the recording: it closes the window and leaves things as they are (Keep recording).
    const dismiss = () => run(stillHereKeep);
    return paused ? (
      <WWindowFrame
        design={WINDOW_DESIGN.stillHere}
        title={t("This recording is still paused.")}
        lines={pausedSince != null ? [t("Paused since {time}.", { time: timeLabel(pausedSince, locale) })] : []}
        // No Return shortcut: Resume starts audio again (`WINDOW_KEYS.stillHerePaused`). Escape keeps things as they are.
        onPrimary={busy ? undefined : mainActionOnReturn("stillHerePaused", () => run(() => meetingRecordResume().then(() => undefined)))}
        onDismiss={busy ? undefined : dismiss}
        actions={
          <>
            <WButton variant="primary" disabled={busy} onClick={() => run(() => meetingRecordResume().then(() => undefined))}>
              {t("Resume")}
            </WButton>
            <WButton disabled={busy} onClick={() => run(() => meetingRecordStop().then(() => undefined))}>
              {t("Stop")}
            </WButton>
          </>
        }
      />
    ) : (
      <WWindowFrame
        design={WINDOW_DESIGN.stillHere}
        title={t("Still in a meeting?")}
        lines={[t("No one has spoken for 15 minutes.")]}
        onPrimary={busy ? undefined : mainActionOnReturn("stillHere", () => run(stillHereKeep))}
        onDismiss={busy ? undefined : dismiss}
        actions={
          <>
            <WButton variant="primary" disabled={busy} onClick={() => run(stillHereKeep)}>
              {t("Keep recording")}
            </WButton>
            <WButton disabled={busy} onClick={() => run(stillHereStop)}>
              {t("Stop")}
            </WButton>
          </>
        }
      />
    );
  }
  if (paused) {
    return (
      <div className="alert">
        <span className="popup-brand">{"Whimbrel"}</span>
        <p role="status" className="alert-title">
          {t("This recording is still paused.")}
        </p>
        <div className="popup-actions">
          <button className="mode mode--active" disabled={busy} onClick={() => run(() => meetingRecordResume().then(() => undefined))}>
            {t("Resume")}
          </button>
          <button className="mode" disabled={busy} onClick={() => run(() => meetingRecordStop().then(() => undefined))}>
            {t("Stop")}
          </button>
        </div>
      </div>
    );
  }
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
