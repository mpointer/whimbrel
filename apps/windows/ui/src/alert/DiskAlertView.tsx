import { useEffect, useState } from "react";
import { diskAlertDismiss, diskAlertOpen, diskAlertStatus } from "../api";
import type { LowDisk } from "../generated/types";
import { t } from "../l10n";
import { diskSize } from "../settings/statusModel";

/**
 * The low-disk notice (plan 2026-11-12-storage-visibility): a drive Whimbrel uses has under 10 GB
 * free. Free up space opens Settings where Storage is; recording carries on either way.
 */
export function DiskAlertView() {
  const [disks, setDisks] = useState<LowDisk[]>([]);

  useEffect(() => {
    const load = () => diskAlertStatus().then(setDisks, () => {});
    load();
    const timer = setInterval(load, 1_000);
    return () => clearInterval(timer);
  }, []);

  if (disks.length === 0) return <div className="alert" />;
  const least = Math.min(...disks.map((d) => d.freeBytes));
  return (
    <div className="alert">
      {/* The product name, the same in every language. */}
      <span className="popup-brand">{"Whimbrel"}</span>
      <p role="status" className="alert-title">
        {t("Your disk is almost full")}
      </p>
      <p className="small">{t("Only {free} left. Whimbrel may stop recording.", { free: diskSize(least) })}</p>
      <div className="popup-actions">
        <button className="mode mode--active" onClick={() => void diskAlertOpen()}>
          {t("Free up space…")}
        </button>
        <button className="mode" onClick={() => void diskAlertDismiss()}>
          {t("Dismiss")}
        </button>
      </div>
    </div>
  );
}
