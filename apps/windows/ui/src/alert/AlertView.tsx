import { useEffect, useState } from "react";
import { alertAccept, alertDismiss, alertStatus } from "../api";
import type { AlertView as AlertStatus } from "../generated/types";
import { useL10n } from "../l10n";
import { WButton, WINDOW_DESIGN, WWindowFrame, mainActionOnReturn } from "../system";
import { dismissWords } from "./meetingAlertModel";

/**
 * The "meeting detected" window (M3.3 Task 6). It asks once; with auto-start on, the
 * countdown answers for you. Unlike the tray popup it stays put when it loses focus —
 * you're in a call when it appears.
 */
export function AlertView() {
  const { t } = useL10n();
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

  const dismiss = dismissWords(counting, t);

  if (!meeting) return <div className="w-sys w-win-root" />;
  return (
    <WWindowFrame
      design={WINDOW_DESIGN.alert}
      title={t("Meeting detected")}
      lines={[<span className="w-win-strong">{meeting.title}</span>]}
      // Pinned, not in the scroller: what recording means is in view at every text size.
      pinned={t("Record it? Audio stays on this PC.")}
      // The countdown answers for you. Its way out is the action row's dismiss button, which reads Don't record while it runs.
      countdown={counting ? <span>{t("Recording starts in {seconds} s", { seconds: status!.countdownSeconds })}</span> : undefined}
      // No Return shortcut: Record starts a recording, and a key typed into the app behind this window must not decide that.
      // Return presses the focused button; Escape is Not now (`WINDOW_KEYS.meetingAlert`).
      onPrimary={mainActionOnReturn("meetingAlert", () => void alertAccept(false))}
      onDismiss={() => void alertDismiss()}
      actions={
        <>
          <WButton variant="primary" onClick={() => void alertAccept(false)}>
            {t("Record")}
          </WButton>
          <WButton onClick={() => void alertAccept(true)}>{t("Record & open")}</WButton>
          <WButton onClick={() => void alertDismiss()} title={dismiss.help}>
            {dismiss.label}
          </WButton>
        </>
      }
    />
  );
}
