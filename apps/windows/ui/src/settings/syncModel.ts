import type { DeviceView, SyncState, SyncView } from "../generated/types";

export type SyncAction = "signIn" | "cancel" | "checkAgain" | "none";

/** The one button the Sync card offers in each state. */
export function primaryAction(state: SyncState): SyncAction {
  switch (state) {
    case "notSignedIn":
    case "error":
      return "signIn";
    case "signingIn":
    case "waitingForApproval":
      return "cancel";
    case "waitingForGroupKey":
      return "checkAgain";
    case "ready":
    case "notConfigured":
      return "none";
  }
}

export function stateLabel(state: SyncState): string {
  switch (state) {
    case "notConfigured":
      return "Not configured";
    case "notSignedIn":
      return "Off";
    case "signingIn":
      return "Signing in…";
    case "waitingForApproval":
      return "Waiting for approval";
    case "waitingForGroupKey":
      return "Waiting for the account key";
    case "ready":
      return "On";
    case "error":
      return "Needs attention";
  }
}

/** The content-sync line under a Ready status, or null when not enrolled. */
export function contentLine(view: SyncView): string | null {
  if (view.state !== "ready") return null;
  if (view.contentError) return `Content sync failed: ${view.contentError}. Will retry automatically.`;
  if (view.contentLastSyncedMs == null) return "Content hasn't synced yet this session; the first round runs within a minute.";
  const at = new Date(view.contentLastSyncedMs).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  return `Content synced at ${at}. It syncs every minute.`;
}

export type DeviceAction = "approve" | "resendKey" | "revoke";

/** What this PC can do to a device in the list. It can't approve or revoke itself
 *  here; revoking this PC is done from another device. */
export function deviceActions(device: DeviceView): DeviceAction[] {
  if (device.thisDevice || device.status === "revoked") return [];
  const hand = device.status === "pending" ? "approve" : "resendKey";
  return [hand, "revoke"];
}

/** Ids of pending devices whose comparison code is shared with another pending device. */
export function collidingPendingIds(devices: DeviceView[]): Set<string> {
  const byCode = new Map<string, string[]>();
  for (const d of devices) {
    if (d.status !== "pending" || !d.comparisonCode) continue;
    byCode.set(d.comparisonCode, [...(byCode.get(d.comparisonCode) ?? []), d.id]);
  }
  return new Set([...byCode.values()].filter((ids) => ids.length > 1).flat());
}

export function deviceTitle(device: DeviceView): string {
  const name = device.name?.trim();
  if (name) return name;
  return device.thisDevice ? "This PC (unnamed)" : "Unnamed device";
}
