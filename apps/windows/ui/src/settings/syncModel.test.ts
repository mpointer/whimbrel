import { describe, expect, it } from "vitest";
import type { DeviceView } from "../generated/types";
import { collidingPendingIds, contentLine, deviceActions, deviceTitle, primaryAction } from "./syncModel";

describe("contentLine", () => {
  const ready = { state: "ready" as const, message: null, deviceId: "k" };
  it("is silent until enrolled", () => {
    expect(contentLine({ ...ready, state: "waitingForApproval" })).toBeNull();
  });
  it("reports first-round, success and failure", () => {
    expect(contentLine(ready)).toMatch(/hasn't synced yet/);
    expect(contentLine({ ...ready, contentLastSyncedMs: Date.now() })).toMatch(/^Content synced at /);
    expect(contentLine({ ...ready, contentLastSyncedMs: Date.now(), contentError: "timed out after 30s" })).toBe(
      "Content sync failed: timed out after 30s. Will retry automatically.",
    );
  });
});

const device = (over: Partial<DeviceView>): DeviceView => ({
  id: "d1",
  publicKey: "PK",
  name: null,
  platform: null,
  status: "approved",
  thisDevice: false,
  enrolledAtUnixMs: 0,
  ...over,
});

describe("primaryAction", () => {
  it("offers the next step for each state", () => {
    expect(primaryAction("notSignedIn")).toBe("signIn");
    expect(primaryAction("error")).toBe("signIn");
    expect(primaryAction("signingIn")).toBe("cancel");
    expect(primaryAction("waitingForApproval")).toBe("cancel");
    expect(primaryAction("waitingForGroupKey")).toBe("checkAgain");
    expect(primaryAction("ready")).toBe("none");
    expect(primaryAction("notConfigured")).toBe("none");
  });
});

describe("deviceActions", () => {
  it("approves pending devices and re-sends keys to approved ones", () => {
    expect(deviceActions(device({ status: "pending" }))).toEqual(["approve", "revoke"]);
    expect(deviceActions(device({ status: "approved" }))).toEqual(["resendKey", "revoke"]);
  });

  it("offers nothing for this PC or a revoked device", () => {
    expect(deviceActions(device({ thisDevice: true }))).toEqual([]);
    expect(deviceActions(device({ status: "revoked" }))).toEqual([]);
  });
});

describe("deviceTitle", () => {
  it("uses the Worker name, else says it's unnamed", () => {
    expect(deviceTitle(device({ name: "MacBook Pro" }))).toBe("MacBook Pro");
    expect(deviceTitle(device({ name: "  " }))).toBe("Unnamed device");
    expect(deviceTitle(device({ thisDevice: true }))).toBe("This PC (unnamed)");
  });
});

describe("collidingPendingIds", () => {
  const dev = (id: string, status: string, comparisonCode: string | null) =>
    ({ id, status, comparisonCode, publicKey: id, thisDevice: false, enrolledAtUnixMs: 0 }) as DeviceView;
  it("flags pending devices that share a code", () => {
    const ids = collidingPendingIds([dev("a", "pending", "X"), dev("b", "pending", "X"), dev("c", "pending", "Y")]);
    expect([...ids].sort()).toEqual(["a", "b"]);
  });
  it("ignores missing codes and non-pending devices", () => {
    expect(collidingPendingIds([dev("a", "pending", null), dev("b", "pending", null), dev("c", "approved", "X"), dev("d", "pending", "X")]).size).toBe(0);
  });
});
