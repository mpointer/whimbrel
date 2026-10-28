import { useEffect, useState } from "react";
import { meetingRecordStop, popupOpenMain, popupQuit, setCaptureEnabled, trayStatus } from "../api";
import { elapsedLabel } from "../meetings/recordingModel";
import type { TrayStatus } from "../generated/types";

/** The tray's quick-glance popup (plan Task 10), with a running meeting recording (M3.3). */
export function PopupView() {
  const [status, setStatus] = useState<TrayStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(Date.now());

  const load = () =>
    trayStatus()
      .then(setStatus)
      .catch(() => {});

  useEffect(() => {
    load();
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
      <span className="popup-brand">Whimbrel</span>
      <p className="popup-status" role="status">
        {status?.line ?? "Loading…"}
      </p>
      {recordingSince != null && (
        <div className="popup-recording">
          <span role="status" className="recording-line">
            <span className="recording-dot" aria-hidden="true" /> Recording · {elapsedLabel(recordingSince, now)}
          </span>
          <button className="mode" onClick={stopRecording} disabled={busy}>
            Stop
          </button>
        </div>
      )}
      <label className="toggle">
        <input type="checkbox" checked={status?.captureEnabled ?? false} disabled={!status || busy} onChange={toggle} />
        Capture screen text
      </label>
      <div className="popup-actions">
        <button className="mode mode--active" onClick={() => void popupOpenMain()}>
          Open Whimbrel
        </button>
        <button className="mode" onClick={() => void popupQuit()}>
          Quit
        </button>
      </div>
    </div>
  );
}
