import { useEffect, useState } from "react";
import { questionAlertDismiss, questionAlertOpen, questionAlertStatus } from "../api";
import type { QuestionNotice } from "../generated/types";
import { useL10n } from "../l10n";

/**
 * The question alert window (parity item a): a question another device heard someone ask
 * Mike. It shows the newest; Open meeting answers that one, Dismiss clears them all. It
 * stays until answered: Mike is in a call when it appears.
 */
export function QuestionAlertView() {
  const { t } = useL10n();
  const [notices, setNotices] = useState<QuestionNotice[]>([]);

  useEffect(() => {
    const load = () => questionAlertStatus().then(setNotices, () => {});
    load();
    const timer = setInterval(load, 1_000);
    return () => clearInterval(timer);
  }, []);

  const notice = notices[0];
  if (!notice) return <div className="alert" />;
  const more = notices.length - 1;
  return (
    <div className="alert">
      <span className="popup-brand">{notice.title}</span>
      {notice.subtitle && <p className="muted small">{notice.subtitle}</p>}
      <p role="status" className="alert-title">
        {notice.body}
      </p>
      {more > 0 && <p className="muted small">{t("+{count} more", { count: more })}</p>}
      <div className="popup-actions">
        <button className="mode mode--active" onClick={() => void questionAlertOpen(notice.id)}>
          {t("Open meeting")}
        </button>
        <button className="mode" onClick={() => void questionAlertDismiss()}>
          {t("Dismiss")}
        </button>
      </div>
    </div>
  );
}
