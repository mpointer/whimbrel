import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args), Channel: class {} }));

import {
  clientControlGet,
  clientCancelWait,
  clientDelete,
  clientJobsRunning,
  clientDueItems,
  clientExpire,
  clientExport,
  clientInventory,
  clientPendingDevices,
  clientPendingDeviceIds,
  clientRetireDevice,
  clientResummaryOffer,
  clientRetentionDecide,
  clientReviewFlag,
  clientReviewFlagDismiss,
} from "./api";
import { isExclusionRefusal } from "./errors";

describe("client control wrappers", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue({ ok: true, data: null });
  });

  it("send the argument shapes the shell's commands read", async () => {
    await clientInventory("k1");
    expect(invoke).toHaveBeenLastCalledWith("client_inventory", { key: "k1" });

    const sel = {
      key: "k1",
      includePossible: [{ kind: "email" as const, id: "7" }, { kind: "capture" as const, id: "i:3" }],
      deleteExports: true,
      reviewedLinked: [{ kind: "meeting" as const, id: "1" }],
      token: 42,
    };
    await clientDelete(sel);
    expect(invoke).toHaveBeenLastCalledWith("client_delete", { selection: sel });
    await clientCancelWait(42);
    expect(invoke).toHaveBeenLastCalledWith("client_cancel_wait", { token: 42 });
    await clientJobsRunning();
    expect(invoke).toHaveBeenLastCalledWith("client_jobs_running");

    // The shell picks the folder: the page never names one.
    await clientExport("k1", [{ kind: "meeting", id: "4" }], false);
    expect(invoke).toHaveBeenLastCalledWith("client_export", {
      key: "k1",
      includePossible: [{ kind: "meeting", id: "4" }],
      includeMedia: false,
    });

    await clientControlGet("k1");
    expect(invoke).toHaveBeenLastCalledWith("client_control_get", { key: "k1" });

    await clientDueItems();
    expect(invoke).toHaveBeenLastCalledWith("client_due_items", { key: null });
    await clientDueItems("k1");
    expect(invoke).toHaveBeenLastCalledWith("client_due_items", { key: "k1" });

    await clientRetentionDecide("k1", { kind: "email", id: "7" }, { decision: "snooze", until_ms: 99 });
    expect(invoke).toHaveBeenLastCalledWith("client_retention_decide", {
      key: "k1",
      item: { kind: "email", id: "7" },
      decision: { decision: "snooze", until_ms: 99 },
    });
    await clientRetentionDecide("k1", { kind: "email", id: "7" }, { decision: "keep" });
    expect(invoke).toHaveBeenLastCalledWith("client_retention_decide", { key: "k1", item: { kind: "email", id: "7" }, decision: { decision: "keep" } });

    await clientExpire("k1", [{ kind: "episode", id: "2" }], 3000);
    expect(invoke).toHaveBeenLastCalledWith("client_expire", { key: "k1", items: [{ kind: "episode", id: "2" }], budgetMs: 3000 });

    await clientReviewFlag();
    expect(invoke).toHaveBeenLastCalledWith("client_review_flag");
    await clientReviewFlagDismiss();
    expect(invoke).toHaveBeenLastCalledWith("client_review_flag_dismiss");
    await clientResummaryOffer("k1");
    expect(invoke).toHaveBeenLastCalledWith("client_resummary_offer", { key: "k1" });
    await clientPendingDevices();
    expect(invoke).toHaveBeenLastCalledWith("client_pending_devices");
    await clientPendingDeviceIds();
    expect(invoke).toHaveBeenLastCalledWith("client_pending_device_ids");
    await clientRetireDevice("pk-old");
    expect(invoke).toHaveBeenLastCalledWith("client_retire_device", { deviceId: "pk-old" });
  });

  it("hand back core's envelope as it came", async () => {
    for (const kind of ["selection_changed", "busy", "cancelled", "folder_not_allowed"] as const) {
      invoke.mockResolvedValueOnce({ ok: false, error: { kind } });
      expect(await clientDelete({ key: "k1", includePossible: [], deleteExports: false, reviewedLinked: [], token: 1 })).toEqual({ ok: false, error: { kind } });
    }
    invoke.mockResolvedValueOnce({ ok: true, data: null });
    expect(await clientExport("k1", [], false)).toEqual({ ok: true, data: null });
    invoke.mockResolvedValueOnce({ ok: false, error: { kind: "no_device_id" } });
    const r = await clientDelete({ key: "k1", includePossible: [], deleteExports: false, reviewedLinked: [], token: 1 });
    expect(r).toEqual({ ok: false, error: { kind: "no_device_id" } });
  });
});

describe("exclusion refusals", () => {
  it("are told apart from other provider failures", () => {
    expect(isExclusionRefusal({ kind: "provider_failure", reason: { kind: "excluded_client" } })).toBe(true);
    expect(isExclusionRefusal({ kind: "provider_failure", reason: { kind: "exclusion_unreadable" } })).toBe(true);
    expect(isExclusionRefusal({ kind: "provider_failure", reason: { kind: "ollama_not_running" } })).toBe(false);
    expect(isExclusionRefusal({ kind: "provider_failure" })).toBe(false);
    expect(isExclusionRefusal({ kind: "store", message: "x" })).toBe(false);
  });
});
