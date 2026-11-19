// The one non-blocking notice when the library went missing (X4 Task 13c item 4, core's
// NoticeLibraryMissing): a launch here had opened a store before, but no database or copy of it
// was left, so a new, empty one opened. Only on that launch; Dismiss hides it for the rest of it
// (the shell forgets the note, so a rebuilt window does not show it again).

import { useEffect, useState } from "react";
import { launchDismissLibraryMissing, launchLibraryMissing, openExternal } from "../api";
import { t } from "../l10n";

/** The help page the notice links to (src-tauri commands::launch::RESTORE_HELP_URL). */
export const RESTORE_HELP_URL = "https://whimbrelapp.com/help/restore-library-backup/";

export function LibraryMissingBannerView({ onOpenHelp, onDismiss }: { onOpenHelp: () => void; onDismiss: () => void }) {
  return (
    <section className="card notice" role="status">
      <p>
        {t(
          "Your library couldn't be found, so Whimbrel started a new, empty one. If you have a backup of your library, the help page \"Restoring a library backup\" explains how to put it back.",
        )}
      </p>
      <div className="toolbar">
        <button className="link-button" onClick={onOpenHelp}>
          {t("Restoring a library backup")}
        </button>
        <button className="mode" onClick={onDismiss}>
          {t("Dismiss")}
        </button>
      </div>
    </section>
  );
}

export function LibraryMissingBanner() {
  const [show, setShow] = useState(false);
  useEffect(() => {
    let live = true;
    launchLibraryMissing().then(
      (missing) => {
        if (live) setShow(missing);
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, []);
  if (!show) return null;
  return (
    <LibraryMissingBannerView
      onOpenHelp={() => void openExternal(RESTORE_HELP_URL).catch(() => {})}
      onDismiss={() => {
        setShow(false);
        void launchDismissLibraryMissing().catch(() => {});
      }}
    />
  );
}
