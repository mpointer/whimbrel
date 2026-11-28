import { useEffect, useState } from "react";
import { diskAlertDismiss, diskAlertOpen, diskAlertStatus } from "../api";
import type { LowDisk } from "../generated/types";
import { t } from "../l10n";
import { WButton, WINDOW_DESIGN, WWindowFrame, mainActionOnReturn } from "../system";
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

  if (disks.length === 0) return <div className="w-sys w-win-root" />;
  const least = Math.min(...disks.map((d) => d.freeBytes));
  return (
    <WWindowFrame
      design={WINDOW_DESIGN.disk}
      title={t("Your disk is almost full")}
      lines={[t("Only {free} left. Whimbrel may stop recording.", { free: diskSize(least) })]}
      onPrimary={mainActionOnReturn("disk", () => void diskAlertOpen())}
      onDismiss={() => void diskAlertDismiss()}
      actions={
        <>
          <WButton variant="primary" onClick={() => void diskAlertOpen()}>
            {t("Free up space…")}
          </WButton>
          <WButton onClick={() => void diskAlertDismiss()}>{t("Dismiss")}</WButton>
        </>
      }
    />
  );
}
