import type { DevicePlatform, DeviceRecord } from "./accounts";

export type DeviceKind = "computer" | "phone";

/** NULL platform means a row from before platforms were recorded; those were computers. */
export function kindOf(platform: string | null): DeviceKind {
  return platform === "ios" ? "phone" : "computer";
}

/** The free plan allows one active computer and one active phone. Active
 * means anything not revoked (approved, pending or resetting), minus any
 * device a pending or resetting swap will replace. */
export function limitCheck(
  devices: DeviceRecord[],
  platform: DevicePlatform | string | null,
  unlimited: boolean,
): { ok: true } | { ok: false; kind: DeviceKind; holders: DeviceRecord[] } {
  if (unlimited) return { ok: true };
  const kind = kindOf(platform);
  const replaced = new Set(devices.filter((d) => (d.status === "pending" || d.status === "resetting") && d.replacesDeviceId).map((d) => d.replacesDeviceId));
  const holders = devices.filter((d) => d.status !== "revoked" && !replaced.has(d.id) && kindOf(d.platform) === kind);
  return holders.length === 0 ? { ok: true } : { ok: false, kind, holders };
}
