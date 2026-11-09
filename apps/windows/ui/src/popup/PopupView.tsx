import { useEffect, useState } from "react";
import { meetingRecordStop, nextMeetingPrep, popupOpenMain, popupOpenTopic, popupQuit, setCaptureEnabled, trayStatus } from "../api";
import { useTopicLookup } from "../topics/TopicChip";
import { timeLabel } from "../library/rows";
import { elapsedLabel } from "../meetings/recordingModel";
import type { NextMeetingView, TrayStatus } from "../generated/types";
import { useL10n } from "../l10n";

/** The tray's quick-glance popup (plan Task 10), with a running meeting recording (M3.3). */
export function PopupView() {
  const { t, locale } = useL10n();
  // The next meeting and its topic (topics piece 2): the topic opens in the main window.
  const [next, setNext] = useState<NextMeetingView | null>(null);
  const [topic] = useTopicLookup(next ? [{ kind: "event", id: next.eventId, title: next.title }] : []);
  const [status, setStatus] = useState<TrayStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(Date.now());

  const load = () =>
    trayStatus()
      .then(setStatus)
      .catch(() => {});

  useEffect(() => {
    load();
    nextMeetingPrep().then(setNext, () => setNext(null));
    const clock = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(clock);
  }, []);

  const stopRecording = () => {
    setBusy(true);
    meetingRecordStop()
      .then(load)
      .catch(() => {})
      .finally(() => setBusy(false));
  };
  const recordingSince = status?.recordingStartedUnixMs;

  const toggle = () => {
    if (!status) return;
    setBusy(true);
    setCaptureEnabled(!status.captureEnabled)
      .then(load)
      .catch(() => {})
      .finally(() => setBusy(false));
  };

  return (
    <div className="popup">
      <span className="popup-brand">{"Whimbrel"}</span>
      <p className="popup-status" role="status">
        {status?.line ?? t("Loading…")}
      </p>
      {recordingSince != null && (
        <div className="popup-recording">
          <span role="status" className="recording-line">
            <span className="recording-dot" aria-hidden="true" /> {t("Recording · {time}", { time: elapsedLabel(recordingSince, now) })}
          </span>
          <button className="mode" onClick={stopRecording} disabled={busy}>
            {t("Stop")}
          </button>
        </div>
      )}
      {next && (
        <p className="popup-next small">
          {t("Next: {title}, {time}", { title: next.title, time: timeLabel(next.startUnixMs, locale) })}{" "}
          {topic && (
            <button type="button" className="chip citation topic-chip" onClick={() => void popupOpenTopic(topic.topicId)}>
              {topic.name}
            </button>
          )}
        </p>
      )}
      <label className="toggle">
        <input type="checkbox" checked={status?.captureEnabled ?? false} disabled={!status || busy} onChange={toggle} />
        {t("Capture screen text")}
      </label>
      <div className="popup-actions">
        <button className="mode mode--active" onClick={() => void popupOpenMain()}>
          {t("Open Whimbrel")}
        </button>
        <button className="mode" onClick={() => void popupQuit()}>
          {t("Quit")}
        </button>
      </div>
    </div>
  );
}
