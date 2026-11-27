import { useEffect, useState } from "react";
import { meetingRecordResume, meetingRecordStop, meetingRecordingStatus, stillHereKeep, stillHerePrompt, stillHereStop } from "../api";
import { t, useL10n } from "../l10n";
import { timeLabel } from "../library/rows";
import { useRedesign } from "../redesign";
import { WButton, WINDOW_DESIGN, WWindowFrame, mainActionOnReturn } from "../system";

/** Whole seconds as m:ss, for the "Still recording?" countdown. */
export function countdownClock(seconds: number): string {
  const s = Math.max(0, Math.ceil(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Seconds left until `deadline` (unix ms), counted down once a second; null with no deadline. */
function useSecondsLeft(deadline: number | null): number | null {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (deadline == null) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [deadline]);
  return deadline == null ? null : Math.max(0, (deadline - now) / 1000);
}

/**
 * The "Still in a meeting?" window: a recording started by hand has heard no words for 15
 * minutes. Keep recording asks again after 15 more; Stop (or two minutes with no answer, decided
 * by the shell) stops it, trimmed to the last words once transcribed.
 *
 * The same window asks "Still recording?" when a recording has run the hours set in Settings → Meetings:
 * Keep recording asks again in an hour, Stop now stops it as it is, and five minutes with no answer
 * (decided by the shell) stops it, trimmed to the last words.
 */
export function StillHereView() {
  const { locale } = useL10n();
  const [busy, setBusy] = useState(false);
  // Which question the shell opened this window for, read in both looks.
  // Nothing is drawn until it is known, so Narrator never reads the wrong question first.
  const [hourlyDeadline, setHourlyDeadline] = useState<number | null>(null);
  const [known, setKnown] = useState(false);
  useEffect(() => {
    stillHerePrompt().then(
      (p) => {
        setHourlyDeadline(p?.kind === "hourly" ? (p.deadlineUnixMs ?? null) : null);
        setKnown(true);
      },
      () => setKnown(true),
    );
  }, []);
  const secondsLeft = useSecondsLeft(hourlyDeadline);
  const hourly = hourlyDeadline != null;
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
  if (!known) return null;
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
    ) : hourly ? (
      <WWindowFrame
        design={WINDOW_DESIGN.stillHereHourly}
        title={t("Still recording?")}
        lines={[t("This recording has been running for a while.")]}
        countdown={<span>{t("Stopping in {time}", { time: countdownClock(secondsLeft ?? 0) })}</span>}
        onPrimary={busy ? undefined : mainActionOnReturn("stillHere", () => run(stillHereKeep))}
        onDismiss={busy ? undefined : dismiss}
        actions={
          <>
            <WButton variant="primary" disabled={busy} onClick={() => run(stillHereKeep)}>
              {t("Keep recording")}
            </WButton>
            <WButton disabled={busy} onClick={() => run(stillHereStop)}>
              {t("Stop now")}
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
  if (hourly) {
    return (
      <div className="alert">
        <span className="popup-brand">{"Whimbrel"}</span>
        <p role="status" className="alert-title">
          {t("Still recording?")}
        </p>
        <p className="muted small">{t("This recording has been running for a while.")}</p>
        <p className="muted small">{t("Stopping in {time}", { time: countdownClock(secondsLeft ?? 0) })}</p>
        <div className="popup-actions">
          <button className="mode mode--active" disabled={busy} onClick={() => run(stillHereKeep)}>
            {t("Keep recording")}
          </button>
          <button className="mode" disabled={busy} onClick={() => run(stillHereStop)}>
            {t("Stop now")}
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
