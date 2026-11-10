import type { DeviceView, PlanView, SyncState, SyncView } from "../generated/types";
import { t } from "../l10n";

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
      return t("Not configured");
    case "notSignedIn":
      return t("Off");
    case "signingIn":
      return t("Signing in…");
    case "waitingForApproval":
      return t("Waiting for approval");
    case "waitingForGroupKey":
      return t("Waiting for the account key");
    case "ready":
      return t("On");
    case "error":
      return t("Needs attention");
  }
}

/** The content-sync line under a Ready status, or null when not enrolled. */
export function contentLine(view: SyncView, locale: string): string | null {
  if (view.state !== "ready") return null;
  if (view.contentError) return t("Content sync failed: {error}. Will retry automatically.", { error: view.contentError });
  if (view.contentLastSyncedMs == null) return t("Content hasn't synced yet this session; the first round runs within a minute.");
  const time = new Date(view.contentLastSyncedMs).toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" });
  return t("Content synced at {time}. It syncs every minute.", { time });
}

/** The settings-sync line (the Profile, for now) under a Ready status; null until its first round. */
export function settingsLine(view: SyncView, locale: string): string | null {
  if (view.state !== "ready") return null;
  if (view.settingsError) return t("Profile sync failed: {error}. Will retry automatically.", { error: view.settingsError });
  if (view.settingsLastSyncedMs == null) return null;
  const time = new Date(view.settingsLastSyncedMs).toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" });
  return t("Your profile synced at {time}.", { time });
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
  return device.thisDevice ? t("This PC (unnamed)") : t("Unnamed device");
}

// --- Cloud v1 (docs/superpowers/plans/2026-11-09-windows-cloud-v1.md) ---------------------------

/** "Free: 1 computer + 1 phone", "Cloud until November 9, 2027", or once cancelled "Cloud ends November 9,
 *  2027. It won't renew." A server that doesn't send `renews` is read as renewing. */
export function planLine(plan: PlanView, locale: string): string {
  if (plan.plan !== "cloud") return t("Free: 1 computer + 1 phone");
  if (plan.untilUnixMs == null) return t("Cloud");
  const date = new Date(plan.untilUnixMs).toLocaleDateString(locale, { month: "long", day: "numeric", year: "numeric" });
  return plan.renews === false ? t("Cloud ends {date}. It won't renew.", { date }) : t("Cloud until {date}", { date });
}

/** Upgrade shows only on the free plan with billing open. */
export const offersUpgrade = (view: SyncView): boolean => !!view.plan && view.plan.billing && view.plan.plan !== "cloud";

/** Manage subscription shows only on Cloud, with billing open. */
export const offersManage = (view: SyncView): boolean => !!view.plan && view.plan.billing && view.plan.plan === "cloud";
