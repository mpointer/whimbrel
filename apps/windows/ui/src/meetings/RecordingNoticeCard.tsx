import { useState } from "react";
import { t } from "../l10n";
import { dismissNotice, noticeText, noticeVisible, useRecordingNotice } from "./recordingNotice";

/** Shown once per meeting while this PC records it, when the recording notice is on (the Mac's
 *  `RecordingNoticeCard`). Windows posts no chat line, so it is always the reminder. */
export function RecordingNoticeCard({ meetingId }: { meetingId: number | null | undefined }) {
  const { settings, dismissed } = useRecordingNotice();
  const [copied, setCopied] = useState(false);
  if (!noticeVisible(settings, meetingId, dismissed) || meetingId == null) return null;
  const text = noticeText(settings!.text);
  const copy = () => {
    void navigator.clipboard?.writeText(text).then(() => setCopied(true), () => {});
  };

  return (
    <div className="card notice" role="region" aria-label={t("Let people know you're recording.")}>
      <p className="row-title">{t("Let people know you're recording.")}</p>
      <p className="muted small">{text}</p>
      <div className="toolbar">
        <button className="mode" onClick={copy}>
          {copied ? t("Copied") : t("Copy")}
        </button>
        <button className="mode" onClick={() => dismissNotice(meetingId)}>
          {t("Done")}
        </button>
      </div>
    </div>
  );
}
