import { useEffect, useState } from "react";
import { syncStatus } from "../api";
import type { SyncView } from "../generated/types";
import { t } from "../l10n";
import { syncStopped } from "./statusModel";

const POLL_MS = 60 * 1000;

/** The sidebar's "Sync stopped" line (issue 590): shown while a sign-in problem stops sync; opens Settings → Status. */
export function SyncStoppedLine({ onOpen }: { onOpen: () => void }) {
  const [view, setView] = useState<SyncView | null>(null);

  useEffect(() => {
    const poll = () => syncStatus().then(setView).catch(() => {});
    poll();
    const id = setInterval(poll, POLL_MS);
    return () => clearInterval(id);
  }, []);

  if (!syncStopped(view)) return null;
  return (
    <div className="sidebar-footer small sync-stopped" role="status">
      <button className="link" onClick={onOpen}>
        {t("Sync stopped")}
      </button>
    </div>
  );
}
