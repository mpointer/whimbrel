// What the redesigned Sync & Devices section says, apart from the components. Pure: the old syncModel decides the
// states and the actions; this maps them onto the parts' tones and lines.
import type { DeviceView, SyncState } from "../../../../generated/types";
import { t } from "../../../../l10n";
import { lastRanLine, isRunner } from "../../../../automations/cloudRoutineModel";
import type { WStatusTone } from "../../../../system";
import { deviceTitle } from "../../../syncModel";

/** The tone of the Sync status chip. The words next to it say the state; the tone only repeats it. */
export function syncTone(state: SyncState): WStatusTone {
  switch (state) {
    case "ready":
      return "good";
    case "error":
      return "problem";
    case "waitingForApproval":
    case "waitingForGroupKey":
    case "signingIn":
      return "warning";
    case "notSignedIn":
    case "notConfigured":
      return "neutral";
  }
}

/** The chip on a device row: "This PC" for this one, else its status word as the sync server sends it. */
export function deviceChip(d: Pick<DeviceView, "thisDevice" | "status">): { tone: WStatusTone; text: string } {
  if (d.thisDevice) return { tone: "neutral", text: t("This PC") };
  const tone: WStatusTone = d.status === "approved" ? "good" : d.status === "pending" ? "warning" : "neutral";
  return { tone, text: d.status };
}

/**
 * What a pending device must show before it is approved, at the primary text size next to the device: the code to
 * compare and, if another pending device shows the same code, the warning not to approve it. Approving is the one thing
 * here that is hard to take back, so these are not a muted caption.
 */
export function deviceNotes(d: DeviceView, opts: { collides: boolean }): { text: string; warning: boolean }[] {
  if (d.status !== "pending") return [];
  const notes = [{ text: d.comparisonCode ? t("Check this matches the code on the new device: {code}", { code: d.comparisonCode }) : t("Can't verify this device"), warning: false }];
  if (opts.collides) notes.push({ text: t("Another pending device shows the same code. Do not approve it."), warning: true });
  return notes;
}

/** The muted lines under a device's name: the start of its key, and when Whimbrel Cloud last ran. */
export function deviceLines(d: DeviceView, opts: { runnerLastRun: number | null; locale: string }): string[] {
  const lines: string[] = [`${d.publicKey.slice(0, 12)}…`];
  if (isRunner(d) && d.status === "approved" && opts.runnerLastRun !== null) lines.push(lastRanLine(opts.runnerLastRun, opts.locale));
  return lines;
}

/** The accessible name of a device's button: the action and which device, so a list of "Revoke…" is not a list of the same name. */
export const deviceActionName = (action: string, d: DeviceView): string => `${action}, ${deviceTitle(d)}`;
