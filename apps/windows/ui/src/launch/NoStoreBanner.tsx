// The no-store notice at the top of the app and in the tray popup (brief 3.3.8).

import { t } from "../l10n";
import { useRedesign } from "../redesign";
import { WBanner, WButton } from "../system";

export function NoStoreBanner({ onOpenSettings }: { onOpenSettings?: () => void }) {
  const redesign = useRedesign();
  if (redesign)
    return (
      <WBanner
        label={t("The memory database could not be opened. Recording is off. Restart Whimbrel to try again.")}
        tone="warning"
        actions={onOpenSettings ? <WButton onClick={onOpenSettings}>{t("Open Settings")}</WButton> : undefined}
      >
        <p>{t("The memory database could not be opened. Recording is off. Restart Whimbrel to try again.")}</p>
      </WBanner>
    );
  return (
    <div className="card notice no-store-banner" role="status">
      <span>{t("The memory database could not be opened. Recording is off. Restart Whimbrel to try again.")}</span>
      {onOpenSettings && (
        <button className="link-button" onClick={onOpenSettings}>
          {t("Open Settings")}
        </button>
      )}
    </div>
  );
}

/** The caption beside a disabled recording start. */
export function RecordingOffCaption() {
  return <span className="muted small">{t("Recording is off because the memory database could not be opened.")}</span>;
}
