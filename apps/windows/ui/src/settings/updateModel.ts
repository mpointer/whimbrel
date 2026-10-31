// Settings → General's "Check for Updates" (the Mac's wording; src-tauri/src/app/update.rs follows the check).

import type { UpdateCheck } from "../generated/types";
import { t } from "../l10n";

/** How often the line follows a check that is still going. */
export const FOLLOW_MS = 1000;

/** The status line under the button; none before the first check. */
export function updateLine(check: UpdateCheck): string | null {
  switch (check.state) {
    case "idle":
      return null;
    case "checking":
      return t("Checking…");
    case "upToDate":
      return t("Whimbrel is up to date (v{version}).", { version: check.version });
    case "downloading":
      return t("Downloading v{version}…", { version: check.version });
    case "ready":
      return t("v{version} is ready.", { version: check.version });
    case "failed":
      return t("Couldn't check for updates. Try again later.");
  }
}

/** Whether the check is still going, so the line keeps following it and the button waits. */
export const isBusy = (check: UpdateCheck): boolean => check.state === "checking" || check.state === "downloading";
