import { useEffect, useState } from "react";
import { questionAlertDismiss, questionAlertOpen, questionAlertStatus } from "../api";
import type { QuestionNotice } from "../generated/types";
import { useL10n } from "../l10n";
import { WButton, WINDOW_DESIGN, WWindowFrame, mainActionOnReturn } from "../system";

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
  if (!notice) return <div className="w-sys w-win-root" />;
  const more = notices.length - 1;
  return (
    <WWindowFrame
      design={WINDOW_DESIGN.question}
      title={notice.title}
      lines={[
        ...(notice.subtitle ? [notice.subtitle] : []),
        <span className="w-win-strong">{notice.body}</span>,
        ...(more > 0 ? [t("+{count} more", { count: more })] : []),
      ]}
      onPrimary={mainActionOnReturn("question", () => void questionAlertOpen(notice.id))}
      onDismiss={() => void questionAlertDismiss()}
      actions={
        <>
          <WButton variant="primary" onClick={() => void questionAlertOpen(notice.id)}>
            {t("Open meeting")}
          </WButton>
          <WButton onClick={() => void questionAlertDismiss()}>{t("Dismiss")}</WButton>
        </>
      }
    />
  );
}
