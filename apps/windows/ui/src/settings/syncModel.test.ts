import { describe, expect, it } from "vitest";
import type { DeviceView, PlanView } from "../generated/types";
import { collidingPendingIds, contentLine, deviceActions, deviceTitle, offersManage, offersUpgrade, planLine, primaryAction, settingsLine } from "./syncModel";

describe("contentLine", () => {
  const ready = { state: "ready" as const, message: null, deviceId: "k", deviceLimit: false, planUnknown: false };
  it("is silent until enrolled", () => {
    expect(contentLine({ ...ready, state: "waitingForApproval" }, "en")).toBeNull();
  });
  it("reports first-round, success and failure", () => {
    expect(contentLine(ready, "en")).toMatch(/hasn't synced yet/);
    expect(contentLine({ ...ready, contentLastSyncedMs: Date.now() }, "en")).toMatch(/^Content synced at /);
    expect(contentLine({ ...ready, contentLastSyncedMs: Date.now(), contentError: "timed out after 30s" }, "en")).toBe(
      "Content sync failed: timed out after 30s. Will retry automatically.",
    );
  });
});

describe("settingsLine", () => {
  const ready = { state: "ready" as const, message: null, deviceId: "k", deviceLimit: false, planUnknown: false };
  it("is silent until enrolled and until the first round", () => {
    expect(settingsLine({ ...ready, state: "waitingForApproval", settingsError: "x" }, "en")).toBeNull();
    expect(settingsLine(ready, "en")).toBeNull();
  });
  it("reports success and failure", () => {
    expect(settingsLine({ ...ready, settingsLastSyncedMs: Date.now() }, "en")).toMatch(/^Your profile synced at /);
    expect(settingsLine({ ...ready, settingsLastSyncedMs: Date.now(), settingsError: "offline" }, "en")).toBe(
      "Profile sync failed: offline. Will retry automatically.",
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

describe("the plan (Cloud v1)", () => {
  const view = (plan: PlanView | null) => ({ state: "ready" as const, message: null, deviceId: "k", deviceLimit: false, planUnknown: false, plan });
  const free = { plan: "free", untilUnixMs: null, interval: null, billing: true };
  const cloud = { plan: "cloud", untilUnixMs: Date.UTC(2027, 10, 9, 12), interval: "year", billing: true };

  it("reads Free or Cloud until a date", () => {
    expect(planLine(free, "en-US")).toBe("Free: 1 computer + 1 phone");
    expect(planLine(cloud, "en-US")).toBe("Cloud until November 9, 2027");
    expect(planLine({ ...cloud, renews: true }, "en-US")).toBe("Cloud until November 9, 2027");
    expect(planLine({ ...cloud, renews: false }, "en-US")).toBe("Cloud ends November 9, 2027. It won't renew.");
  });

  it("offers Upgrade on Free and Manage on Cloud, only with billing open", () => {
    expect([offersUpgrade(view(free)), offersManage(view(free))]).toEqual([true, false]);
    expect([offersUpgrade(view(cloud)), offersManage(view(cloud))]).toEqual([false, true]);
    expect([offersUpgrade(view({ ...free, billing: false })), offersManage(view({ ...cloud, billing: false }))]).toEqual([false, false]);
    expect([offersUpgrade(view(null)), offersManage(view(null))]).toEqual([false, false]);
  });
});
