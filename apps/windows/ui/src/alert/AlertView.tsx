import { useEffect, useState } from "react";
import { alertAccept, alertDismiss, alertStatus } from "../api";
import type { AlertView as AlertStatus } from "../generated/types";

/**
 * The "meeting detected" window (M3.3 Task 6). It asks once; with auto-start on, the
 * countdown answers for you. Unlike the tray popup it stays put when it loses focus —
 * you're in a call when it appears.
 */
export function AlertView() {
  const [status, setStatus] = useState<AlertStatus | null>(null);

  useEffect(() => {
    const load = () =>
      alertStatus()
        .then(setStatus)
        .catch(() => {});
    load();
    const timer = setInterval(load, 500);
    return () => clearInterval(timer);
  }, []);

  const meeting = status?.meeting;
  const counting = status?.autoStart === true && meeting != null;

  useEffect(() => {
    if (counting && status!.countdownSeconds <= 0) void alertAccept(false);
  }, [counting, status]);

  if (!meeting) return <div className="alert" />;
  return (
    <div className="alert">
      <span className="popup-brand">Meeting detected</span>
      <p role="status" className="alert-title">
        {meeting.title}
      </p>
      <p className="muted small">
        {counting
          ? `Recording starts in ${status!.countdownSeconds}s. Audio stays on this PC.`
          : "Record it? Audio stays on this PC."}
      </p>
      <div className="popup-actions">
        <button className="mode mode--active" onClick={() => void alertAccept(false)}>
          Record
        </button>
        <button className="mode" onClick={() => void alertAccept(true)}>
          Record &amp; open
        </button>
        <button className="mode" onClick={() => void alertDismiss()}>
          Not now
        </button>
      </div>
    </div>
  );
}
