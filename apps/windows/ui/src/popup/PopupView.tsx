import { useEffect, useState } from "react";
import { emit } from "@tauri-apps/api/event";
import {
  launchState,
  meetingRecordStop,
  nextMeetingPrep,
  popupOpenMain,
  popupOpenTopic,
  popupQuit,
  setCaptureEnabled,
  trayStatus,
} from "../api";
import { NoStoreBanner } from "../launch/NoStoreBanner";
import { useTopicLookup } from "../topics/TopicChip";
import { timeLabel } from "../library/rows";
import { elapsedLabel } from "../meetings/recordingModel";
import type { NextMeetingView, TrayStatus } from "../generated/types";
import { useL10n } from "../l10n";
import { useRedesign } from "../redesign";
import { RedesignedPopup } from "./RedesignedPopup";

/** The tray's quick-glance popup: the redesigned one while the hidden switch is on, else the classic one. */
export function PopupView() {
  return useRedesign() ? <RedesignedPopup /> : <ClassicPopup />;
}

/** The tray's quick-glance popup (plan Task 10), with a running meeting recording (M3.3). */
function ClassicPopup() {
  const { t, locale } = useL10n();
  // The next meeting and its topic (topics piece 2): the topic opens in the main window.
  const [next, setNext] = useState<NextMeetingView | null>(null);
  const [topic] = useTopicLookup(next ? [{ kind: "event", id: next.eventId, title: next.title }] : []);
  const [status, setStatus] = useState<TrayStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(Date.now());
  // Running without the memory database (no-store mode): the popup says so too.
  const [noStore, setNoStore] = useState(false);

  const load = () => {
    return trayStatus()
      .then(setStatus)
      .catch(() => {});
  };

  useEffect(() => {
    load();
    nextMeetingPrep().then(setNext, () => setNext(null));
    launchState().then((v) => setNoStore(v.stage.kind === "openedWithoutStore"), () => {});
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
  // The word "paused" shows whenever the shell says so, whichever look is on.
  const paused = status?.recordingPaused === true;

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
      {noStore && (
        <NoStoreBanner onOpenSettings={() => void popupOpenMain().then(() => emit("open-settings", "capture"), () => {})} />
      )}
      <p className="popup-status" role="status">
        {status?.line ?? t("Loading…")}
      </p>
      {recordingSince != null && (
        <div className="popup-recording">
          <span role="status" className="recording-line">
            {paused ? (
              // Paused is not recording: the paused glyph in the paused color, never the red dot.
              <svg className="popup-paused-glyph" width="12" height="12" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
                <rect x="3.5" y="2.5" width="3" height="11" rx="1" fill="currentColor" />
                <rect x="9.5" y="2.5" width="3" height="11" rx="1" fill="currentColor" />
              </svg>
            ) : (
              <span className="recording-dot" aria-hidden="true" />
            )}{" "}
            {paused ? t("Recording paused") : t("Recording · {time}", { time: elapsedLabel(recordingSince, now) })}
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
