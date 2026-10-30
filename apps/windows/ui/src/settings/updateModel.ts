// Settings → General's "Check for Updates" (the Mac's wording; src-tauri/src/app/update.rs follows the check).

import type { UpdateCheck } from "../generated/types";

/** How often the line follows a check that is still going. */
export const FOLLOW_MS = 1000;

/** The status line under the button; none before the first check. */
export function updateLine(check: UpdateCheck): string | null {
  switch (check.state) {
    case "idle":
      return null;
    case "checking":
      return "Checking…";
    case "upToDate":
      return `Whimbrel is up to date (v${check.version}).`;
    case "downloading":
      return `Downloading v${check.version}…`;
    case "ready":
      return `v${check.version} is ready.`;
    case "failed":
      return "Couldn't check for updates. Try again later.";
  }
}

/** Whether the check is still going, so the line keeps following it and the button waits. */
export const isBusy = (check: UpdateCheck): boolean => check.state === "checking" || check.state === "downloading";
