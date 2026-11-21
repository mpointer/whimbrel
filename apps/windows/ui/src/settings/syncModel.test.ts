import { describe, expect, it } from "vitest";
import type { DeviceView, PlanView } from "../generated/types";
import { collidingPendingIds, contentLine, deviceActions, deviceTitle, indexingLine, offersManage, offersUpgrade, pendingUpdateLine, planLine, sellsPlus, primaryAction, settingsLine } from "./syncModel";

describe("pendingUpdateLine", () => {
  it("says how many devices still need to update, and nothing for none or not known", () => {
    expect(pendingUpdateLine(null)).toBeNull();
    expect(pendingUpdateLine(0)).toBeNull();
    expect(pendingUpdateLine(1)).toBe("1 device still needs to update before it applies client deletes.");
    expect(pendingUpdateLine(4)).toBe("4 devices still need to update before they apply client deletes.");
  });
});

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
  const free = { plan: "free", untilUnixMs: null, interval: null, billing: true, plusAvailable: false };
  const cloud = { plan: "cloud", untilUnixMs: Date.UTC(2027, 10, 9, 12), interval: "year", billing: true, plusAvailable: false };
  const plus = { ...cloud, planTier: "plus" };

  it("reads Free or Cloud until a date", () => {
    expect(planLine(free, "en-US")).toBe("Free: 1 computer + 1 phone");
    expect(planLine(cloud, "en-US")).toBe("Cloud until November 9, 2027");
    expect(planLine({ ...cloud, renews: true }, "en-US")).toBe("Cloud until November 9, 2027");
    expect(planLine({ ...cloud, renews: false }, "en-US")).toBe("Cloud ends November 9, 2027. It won't renew.");
  });

  it("says Plus, never Cloud, on a Plus plan", () => {
    expect(planLine(plus, "en-US")).toBe("Plus until November 9, 2027");
    expect(planLine({ ...plus, renews: false }, "en-US")).toBe("Plus ends November 9, 2027. It won't renew.");
    expect(planLine({ ...plus, untilUnixMs: null }, "en-US")).toBe("Plus");
    expect(planLine({ ...cloud, planTier: "cloud" }, "en-US")).toBe("Cloud until November 9, 2027");
    expect([offersUpgrade(view(plus)), offersManage(view(plus))]).toEqual([false, true]);
  });

  it("sells Plus only when the server says so", () => {
    expect(sellsPlus(view({ ...free, plusAvailable: true }))).toBe(true);
    expect(sellsPlus(view(free))).toBe(false);
    expect(sellsPlus(view(null))).toBe(false);
  });

  it("offers Upgrade on Free and Manage on Cloud, only with billing open", () => {
    expect([offersUpgrade(view(free)), offersManage(view(free))]).toEqual([true, false]);
    expect([offersUpgrade(view(cloud)), offersManage(view(cloud))]).toEqual([false, true]);
    expect([offersUpgrade(view({ ...free, billing: false })), offersManage(view({ ...cloud, billing: false }))]).toEqual([false, false]);
    expect([offersUpgrade(view(null)), offersManage(view(null))]).toEqual([false, false]);
  });
});

describe("Whimbrel Cloud in the device list", () => {
  const runner: DeviceView = {
    id: "r",
    publicKey: "RUNNER",
    name: "Whimbrel Cloud",
    platform: "cloud",
    status: "approved",
    thisDevice: false,
    enrolledAtUnixMs: 0,
    comparisonCode: null,
  };

  it("is named Whimbrel Cloud and offers only Remove, never Re-send Key or Revoke", () => {
    expect(deviceTitle({ ...runner, name: null })).toBe("Whimbrel Cloud");
    expect(deviceActions(runner)).toEqual(["removeRunner"]);
    expect(deviceActions({ ...runner, status: "revoked" })).toEqual([]);
  });

  it("leaves a person's devices as they were", () => {
    expect(deviceActions({ ...runner, platform: "windows" })).toEqual(["resendKey", "revoke"]);
    expect(deviceActions({ ...runner, platform: "macos", status: "pending" })).toEqual(["approve", "revoke"]);
  });
});

describe("indexingLine", () => {
  const ready = { state: "ready" as const, message: null, deviceId: "k", deviceLimit: false, planUnknown: false };
  it("shows the percentage only while the indexing runs", () => {
    expect(indexingLine({ ...ready, kindBackfillPercent: 61 })).toBe("Indexing history: 61%");
    expect(indexingLine({ ...ready, kindBackfillPercent: 0 })).toBe("Indexing history: 0%");
    expect(indexingLine(ready)).toBeNull();
    expect(indexingLine({ ...ready, kindBackfillPercent: null })).toBeNull();
    expect(indexingLine({ ...ready, state: "waitingForApproval", kindBackfillPercent: 61 })).toBeNull();
  });
});
