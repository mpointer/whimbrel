import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class {} }));

import type {
  ClientControl,
  ClientControlChange,
  ClientDeleteReceipt,
  ClientDue,
  ClientExpiryReceipt,
  ClientExportDone,
  ClientInventory,
  ClientItem,
  ClientItemRef,
  ClientOpErrorKind,
  ClientResult,
  ClientResummaryOffer,
  ClientRetentionDecision,
  ClientReviewFlag,
} from "../clientControls";
import {
  ClientRetentionReviewModel,
  ClientReviewModel,
  ClientSettingsModel,
  PendingDevicesModel,
  clientMessage,
  derivedLine,
  deviceNoLongerListed,
  expiryLeftForYouLine,
  expiryStoppedLine,
  failedLine,
  heldDevicesLine,
  leftForYouLine,
  leftOutLine,
  matchingLimitLine,
  reappearsLine,
  receiptRetryHelps,
  pendingDevicesLine,
  regenerateInTurn,
  receiptHeadline,
  reviewRows,
  groupTitle,
  itemLine,
  kindName,
  kindSingular,
  rollbackCopyRemovedLine,
  rowDetail,
  rowTitle,
  summaryComplete,
  type ClientControlsBridge,
  type PendingDevicesBridge,
} from "./clientControlsModel";

const NAME = "Zorblax";
const ok = <T>(data: T): ClientResult<T> => ({ ok: true, data });
const err = <T>(kind: ClientOpErrorKind): ClientResult<T> => ({
  ok: false,
  error: { kind },
});

const item = (kind: ClientItem["kind"], id: string, also: string | null = null): ClientItem => ({
  kind,
  id,
  also_client: also,
  date_unix_ms: 1_762_000_000_000,
  removed: false,
});

const inventory = (possible: ClientItem[] = [item("email", "7"), item("capture", "i:3")]): ClientInventory => ({
  linked: [item("meeting", "1"), item("email", "2", "Globex")],
  possible,
  possible_truncated: false,
  counts: [
    ["meeting", 1],
    ["email", 1],
  ],
  left_for_you: ["mailbox_originals", "backups_and_cloud"],
});

const receipt = (over: Partial<ClientDeleteReceipt> = {}): ClientDeleteReceipt => ({
  date_unix_ms: 1,
  counts: [["meeting", 1]],
  not_sent: [],
  local_only: [],
  client: "removed",
  left_for_you: [],
  pending_devices: 0,
  failed: [],
  failed_stages: [],
  complete: true,
  retry_helps: false,
  ...over,
});

/** A scripted core. Each list is consumed one reply per call; the last one repeats. */
class FakeBridge implements ClientControlsBridge {
  inventories: ClientResult<ClientInventory>[] = [ok(inventory())];
  deletes: ClientResult<ClientDeleteReceipt>[] = [ok(receipt())];
  deleteCalls: {
    key: string;
    includePossible: ClientItemRef[];
    deleteExports: boolean;
  }[] = [];
  /** Held open until released, to see the screen while a delete runs. */
  gate: Promise<void> | null = null;
  stored: ClientControl = { exclude_hosted: false, keep_months: null };
  /** The changes sent; the page never sends a whole record. */
  updateCalls: ClientControlChange[] = [];
  /** How many re-summaries the fake says were not made. */
  notMade = 0;
  offer: ClientResult<ClientResummaryOffer> = ok({
    meetings: [11, 12],
    already_left_note: "Material that already left cannot be recalled.",
  });
  resummarized: number[][] = [];
  due: ClientResult<ClientDue[]> = ok([]);
  dueFor: ClientResult<ClientItem[]>[] = [ok([])];
  expires: ClientResult<ClientExpiryReceipt>[] = [];
  expireCalls: ClientItemRef[][] = [];
  decisions: ClientRetentionDecision[] = [];
  exports: ClientResult<ClientExportDone | null>[] = [];

  private next<T>(list: T[]): T {
    return list.length > 1 ? list.shift()! : list[0];
  }
  inventoryCalls = 0;
  inventory = async () => {
    this.inventoryCalls += 1;
    return this.next(this.inventories);
  };
  /** The linked items and token each delete was sent with. */
  deleteExtras: { reviewedLinked: ClientItemRef[]; token: number }[] = [];
  /** What `jobsRunning` answers. */
  running = 0;
  /** Tokens cancelled; `tooLate` makes a cancel answer false. A cancel ends a gated delete with `cancelled`. */
  cancels: number[] = [];
  tooLate = false;
  private cancelled: (() => void) | null = null;
  delete = async (key: string, includePossible: ClientItemRef[], deleteExports: boolean, reviewedLinked: ClientItemRef[], token: number) => {
    this.deleteCalls.push({ key, includePossible, deleteExports });
    this.deleteExtras.push({ reviewedLinked, token });
    if (this.gate) {
      const cancel = new Promise<"cancelled">((r) => (this.cancelled = () => r("cancelled")));
      if ((await Promise.race([this.gate.then(() => "done" as const), cancel])) === "cancelled") return err<ClientDeleteReceipt>("cancelled");
    }
    return this.next(this.deletes);
  };
  jobsRunning = async () => this.running;
  cancelWait = async (token: number) => {
    this.cancels.push(token);
    if (this.tooLate) return false;
    this.cancelled?.();
    return true;
  };
  control = async () => ok({ ...this.stored });
  // Like core's control_update: the change applies to the value stored at that moment, in one step.
  updateControl = async (_key: string, change: ClientControlChange) => {
    this.updateCalls.push(change);
    const before = { ...this.stored };
    this.stored = { ...before, ...change };
    return ok({ before, after: { ...this.stored } });
  };
  resummaryOffer = async () => this.offer;
  resummarize = async (m: number[]) => {
    this.resummarized.push(m);
    return this.notMade;
  };
  reviewFlag = async () => ok<ClientReviewFlag | null>(null);
  dismissReviewFlag = async () => ok(null);
  dueItems = async () => this.due;
  dueItemsFor = async () => this.next(this.dueFor);
  decide = async (_key: string, _item: ClientItemRef, d: ClientRetentionDecision) => {
    this.decisions.push(d);
    return ok(null);
  };
  expire = async (_key: string, items: ClientItemRef[]) => {
    this.expireCalls.push(items);
    return this.next(this.expires);
  };
  export = async () => this.next(this.exports);
  held: number | null = null;
  heldDevices = async () => this.held;
}

async function reviewed(bridge = new FakeBridge(), mode: "delete" | "export" = "delete") {
  const m = new ClientReviewModel("k1", NAME, mode, bridge);
  await m.load();
  return { m, bridge };
}

describe("delete review", () => {
  it("confirm_requires_exact_name", async () => {
    const { m, bridge } = await reviewed();
    for (const near of ["", "zorblax", "Zorblax ", " Zorblax", "Zorbla"]) {
      m.setConfirmText(near);
      expect(m.canDelete).toBe(false);
      await m.delete();
    }
    expect(bridge.deleteCalls).toEqual([]);
    m.setConfirmText(NAME);
    expect(m.canDelete).toBe(true);
    await m.delete();
    expect(bridge.deleteCalls).toHaveLength(1);
  });

  it("possible_matches_start_unticked", async () => {
    const { m, bridge } = await reviewed();
    expect(m.inventory?.possible).toHaveLength(2);
    expect(m.inventory?.possible.some((p) => m.isTicked(p))).toBe(false);
    m.setTicked(item("email", "7"), true);
    // An item the inventory does not list cannot be ticked.
    m.setTicked(item("email", "999"), true);
    m.setConfirmText(NAME);
    await m.delete();
    expect(bridge.deleteCalls[0]).toEqual({
      key: "k1",
      includePossible: [{ kind: "email", id: "7" }],
      deleteExports: false,
    });
  });

  it("delete_button_disabled_while_running", async () => {
    const bridge = new FakeBridge();
    let release!: () => void;
    bridge.gate = new Promise((r) => (release = r));
    const { m } = await reviewed(bridge);
    m.setConfirmText(NAME);
    const running = m.delete();
    await vi.waitFor(() => expect(bridge.deleteCalls).toHaveLength(1));
    expect(m.isRunning).toBe(true);
    expect(m.canDelete).toBe(false);
    await m.delete();
    await m.retry();
    expect(bridge.deleteCalls).toHaveLength(1);
    release();
    await running;
    expect(m.phase.kind).toBe("receipt");
  });

  it("waits with Cancel while background work runs; Cancel deletes nothing and keeps the ticks and name", async () => {
    const bridge = new FakeBridge();
    bridge.gate = new Promise(() => {});
    bridge.running = 2;
    const { m } = await reviewed(bridge);
    m.waitPollMs = 5;
    m.setTicked(item("email", "7"), true);
    m.setConfirmText(NAME);
    const running = m.delete();
    await vi.waitFor(() => expect(m.phase.kind).toBe("waiting"));
    expect(m.isRunning).toBe(true);
    expect(bridge.deleteExtras[0].reviewedLinked).toEqual([
      { kind: "meeting", id: "1" },
      { kind: "email", id: "2" },
    ]);
    await m.cancelWait();
    await running;
    expect(bridge.cancels).toEqual([bridge.deleteExtras[0].token]);
    expect(m.phase.kind).toBe("review");
    expect(m.notice).toBeNull();
    expect(m.tickedRefs).toEqual([{ kind: "email", id: "7" }]);
    expect(m.confirmText).toBe(NAME);
    // A retry never deletes after a cancel: Delete must be pressed again, with a new token.
    bridge.gate = null;
    await m.retry();
    expect(bridge.deleteCalls).toHaveLength(1);
    await m.delete();
    expect(bridge.deleteCalls).toHaveLength(2);
    expect(bridge.deleteExtras[1].token).toBeGreaterThan(bridge.deleteExtras[0].token);
    expect(bridge.deleteExtras[1].token).toBeLessThan(Number.MAX_SAFE_INTEGER);
  });

  it("shows running, not waiting, when no background work runs", async () => {
    const bridge = new FakeBridge();
    let release!: () => void;
    bridge.gate = new Promise((r) => (release = r));
    const { m } = await reviewed(bridge);
    m.waitPollMs = 5;
    m.setConfirmText(NAME);
    const running = m.delete();
    await vi.waitFor(() => expect(bridge.deleteCalls).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 20));
    expect(m.phase.kind).toBe("running");
    release();
    await running;
    expect(m.phase.kind).toBe("receipt");
  });

  it("a Cancel after the delete started is too late, and the receipt says so", async () => {
    const bridge = new FakeBridge();
    let release!: () => void;
    bridge.gate = new Promise((r) => (release = r));
    bridge.tooLate = true;
    const { m } = await reviewed(bridge);
    m.setConfirmText(NAME);
    const running = m.delete();
    await vi.waitFor(() => expect(bridge.deleteCalls).toHaveLength(1));
    await m.cancelWait();
    expect(m.cancelTooLate).toBe(true);
    release();
    await running;
    expect(m.phase.kind).toBe("receipt");
    expect(m.cancelTooLate).toBe(true);
  });

  it("busy goes back to the review and Try again runs with the confirmed name", async () => {
    const bridge = new FakeBridge();
    bridge.deletes = [err("busy"), ok(receipt())];
    const { m } = await reviewed(bridge);
    m.setConfirmText(NAME);
    await m.delete();
    expect(m.phase.kind).toBe("review");
    expect(m.notice).toBe("busy");
    await m.retry();
    expect(bridge.deleteCalls).toHaveLength(2);
    expect(m.phase.kind).toBe("receipt");
  });

  it("selection_changed reloads the list, drops ticks it no longer lists and asks again", async () => {
    const bridge = new FakeBridge();
    bridge.inventories = [ok(inventory()), ok(inventory([item("capture", "i:3")]))];
    bridge.deletes = [err("selection_changed")];
    const { m } = await reviewed(bridge);
    m.setTicked(item("email", "7"), true);
    m.setTicked(item("capture", "i:3"), true);
    m.setConfirmText(NAME);
    await m.delete();
    expect(m.phase.kind).toBe("review");
    expect(m.notice).toBe("selection_changed");
    expect(m.tickedRefs).toEqual([{ kind: "capture", id: "i:3" }]);
    expect(bridge.deleteCalls).toHaveLength(1);
  });

  it("a partial receipt retries with a selection rebuilt from a fresh inventory", async () => {
    const bridge = new FakeBridge();
    bridge.inventories = [ok(inventory()), ok(inventory([item("capture", "i:3")]))];
    bridge.deletes = [
      ok(
        receipt({
          complete: false,
          retry_helps: true,
          client: "still_present",
          failed: ["capture"],
        }),
      ),
      ok(receipt()),
    ];
    const { m } = await reviewed(bridge);
    m.setTicked(item("email", "7"), true);
    m.setTicked(item("capture", "i:3"), true);
    m.setConfirmText(NAME);
    await m.delete();
    await m.retry();
    expect(bridge.deleteCalls[1].includePossible).toEqual([{ kind: "capture", id: "i:3" }]);
    expect(m.phase.kind).toBe("receipt");
  });

  it("a retry after the client record went keeps the receipt and offers no more retries", async () => {
    const bridge = new FakeBridge();
    bridge.inventories = [ok(inventory()), err("not_found")];
    bridge.deletes = [
      ok(
        receipt({
          client: "still_present",
          complete: false,
          retry_helps: true,
          failed_stages: ["files"],
        }),
      ),
    ];
    const { m } = await reviewed(bridge);
    m.setConfirmText(NAME);
    await m.delete();
    await m.retry();
    expect(m.phase.kind).toBe("receipt");
    expect(m.retryGone).toBe(true);
    await m.retry();
    expect(bridge.deleteCalls).toHaveLength(1);
  });

  it("Try again after a failed load never deletes what was not confirmed", async () => {
    const bridge = new FakeBridge();
    bridge.inventories = [err("failed"), ok(inventory())];
    const { m } = await reviewed(bridge);
    expect(m.phase).toEqual({ kind: "failed", error: "failed" });
    m.setConfirmText(NAME);
    await m.retry();
    expect(m.phase.kind).toBe("review");
    expect(bridge.deleteCalls).toEqual([]);
  });

  it("core's no_device_id is the no-identity state: no delete, no retry", async () => {
    const bridge = new FakeBridge();
    bridge.deletes = [err("no_device_id")];
    const { m } = await reviewed(bridge);
    m.setConfirmText(NAME);
    await m.delete();
    expect(m.phase.kind).toBe("noIdentity");
    expect(m.canDelete).toBe(false);
    await m.retry();
    expect(bridge.deleteCalls).toHaveLength(1);

    const known = new ClientReviewModel("k1", NAME, "delete", new FakeBridge(), false);
    await known.load();
    expect(known.phase.kind).toBe("noIdentity");
  });

  it("loads once: a dialog mounted again keeps its receipt", async () => {
    const bridge = new FakeBridge();
    bridge.inventories = [ok(inventory()), err("not_found")];
    const { m } = await reviewed(bridge);
    m.setConfirmText(NAME);
    await m.delete();
    await m.load();
    expect(m.phase.kind).toBe("receipt");

    const r = new FakeBridge();
    r.due = ok([{ key: "k1", items: [item("email", "1")] }]);
    r.expires = [
      ok({
        date_unix_ms: 1,
        counts: [["email", 1]],
        not_due: [],
        waiting_to_sync: [],
        more: false,
        failed: [],
        failed_stages: [],
        left_for_you: [],
        complete: true,
      }),
    ];
    const retention = new ClientRetentionReviewModel({}, r);
    await retention.load();
    await retention.deleteTicked();
    r.due = ok([]);
    await retention.load();
    expect(retention.phase.kind).toBe("done");
  });

  it("an unreachable export folder offers a run that leaves the exports", async () => {
    const bridge = new FakeBridge();
    bridge.deletes = [
      ok(
        receipt({
          complete: false,
          retry_helps: true,
          client: "still_present",
          failed: ["export_folder"],
        }),
      ),
      ok(receipt()),
    ];
    const { m } = await reviewed(bridge);
    m.setDeleteExports(true);
    m.setConfirmText(NAME);
    await m.delete();
    expect(m.exportFolderUnreachable).toBe(true);
    await m.retryWithoutDeletingExports();
    expect(bridge.deleteCalls.map((c) => c.deleteExports)).toEqual([true, false]);
  });
});

describe("receipt words", () => {
  it("receipt_shows_pending_devices", () => {
    expect(pendingDevicesLine(null)).toBe(
      "Not known yet how many of your other devices still need to update to apply this delete. Sync works it out on a later round.",
    );
    // A device on an older version writes nothing the count sees: 0 is never "none of your devices".
    expect(pendingDevicesLine(0)).toBe(
      "No device that has updated is still waiting to apply this delete. A device on an older version, or one you removed from the count in Settings, may not have applied it.",
    );
    expect(pendingDevicesLine(1)).toBe("1 device still needs to update before it applies this delete.");
    expect(pendingDevicesLine(3)).toBe("3 devices still need to update before they apply this delete.");
  });

  it("holds no client name, and says Try again only when it can help", () => {
    const r = receipt({
      complete: false,
      retry_helps: false,
      failed: ["meeting"],
      failed_stages: ["derived_text", "no_device_id"],
    });
    const lines = [receiptHeadline(r), failedLine(r) ?? "", ...r.left_for_you.map((l) => leftForYouLine(l)?.text ?? "")];
    expect(lines.join(" ")).not.toContain(NAME);
    expect(receiptHeadline(r)).not.toContain("Try again");
    expect(failedLine(r)).toBe("Not removed: Meetings, text Whimbrel made from this material.");
    expect(receiptHeadline(receipt({ client: "still_present", complete: false, retry_helps: true }))).toContain("Try again");
    expect(receiptHeadline(receipt({ client: "removed_here_only" }))).toContain("this PC only");
  });

  it("says the unencrypted copy went only when the receipt says so (missing from an older core)", () => {
    expect(rollbackCopyRemovedLine(receipt({ rollback_copy_removed: true }))).toBe(
      "The unencrypted copy of the memory database kept from the encryption upgrade was removed too.",
    );
    expect(rollbackCopyRemovedLine(receipt({ rollback_copy_removed: false }))).toBeNull();
    expect(rollbackCopyRemovedLine(receipt())).toBeNull();
    // A copy that would not go is the `files` stage, said by the failed line, never this one.
    const stuck = receipt({ complete: false, failed_stages: ["files"], rollback_copy_removed: false });
    expect(rollbackCopyRemovedLine(stuck)).toBeNull();
  });
});

describe("devices still to update", () => {
  class FakePending implements PendingDevicesBridge {
    n: number | null = 2;
    list: string[] | null = ["pk-old", "pk-attic"];
    retired: string[] = [];
    answer: ClientResult<{ pending_devices: number | null; topic_rows_held_devices: number | null }> | null = null;
    count = async () => this.n;
    ids = async () => this.list;
    retire = async (id: string) => {
      this.retired.push(id);
      if (this.answer) return this.answer;
      this.list = (this.list ?? []).filter((d) => d !== id);
      this.n = this.list.length;
      return ok({ pending_devices: this.n, topic_rows_held_devices: null });
    };
  }

  it("names each device and removes one only after the confirmation", async () => {
    const bridge = new FakePending();
    const m = new PendingDevicesModel(bridge);
    await m.load();
    expect(m.line).toBe("2 devices still need to update before they apply client deletes.");
    expect(m.ids).toEqual(["pk-old", "pk-attic"]);
    await m.confirm();
    expect(bridge.retired).toEqual([]);
    m.ask("pk-unknown");
    expect(m.confirming).toBeNull();
    m.ask("pk-old");
    m.cancel();
    await m.confirm();
    expect(bridge.retired).toEqual([]); // Cancel removes nothing
    m.ask("pk-old");
    await m.confirm();
    expect(bridge.retired).toEqual(["pk-old"]);
    expect(m.ids).toEqual(["pk-attic"]);
    expect(m.line).toBe("1 device still needs to update before it applies client deletes.");
    await m.confirm();
    expect(bridge.retired).toEqual(["pk-old"]); // one confirmation, one removal

    // It updated meanwhile: core no longer lists it, and the list is read again.
    bridge.answer = err("not_found");
    m.ask("pk-attic");
    bridge.n = 0;
    bridge.list = [];
    await m.confirm();
    expect(m.errorText).toBe(deviceNoLongerListed());
    expect(m.line).toBeNull();
    expect(m.ids).toEqual([]);
  });

  it("shows nothing for none or not known", async () => {
    const bridge = new FakePending();
    bridge.n = null;
    const m = new PendingDevicesModel(bridge);
    await m.load();
    expect(m.line).toBeNull();
    expect(m.ids).toEqual([]);
  });
});

describe("vault rule", () => {
  it("with vault sync off, Left for you says the vault was not touched", async () => {
    expect(leftForYouLine("vault_sync_off")).toEqual({
      text: "Notes in your vault are not touched while vault sync is off. Delete them yourself if they name this client.",
      paths: [],
    });
    const bridge = new FakeBridge();
    bridge.inventories = [ok({ ...inventory(), left_for_you: ["vault_sync_off", "mailbox_originals"] })];
    bridge.deletes = [ok(receipt({ left_for_you: ["vault_sync_off", "copies_on_other_devices"] }))];
    const { m } = await reviewed(bridge);
    expect(m.inventory?.left_for_you.map((l) => leftForYouLine(l)?.text)).toContain(leftForYouLine("vault_sync_off")?.text);
    m.setConfirmText(NAME);
    await m.delete();
    expect(m.phase.kind).toBe("receipt");
    const shown = m.phase.kind === "receipt" ? m.phase.receipt.left_for_you.map((l) => leftForYouLine(l)?.text) : [];
    expect(shown[0]).toMatch(/^Notes in your vault are not touched/);
    expect(shown.join(" ")).not.toContain("removes this client's notes");
  });
});

describe("plurals", () => {
  it("one item expired here is said in the singular", () => {
    expect(leftForYouLine({ expired_here_only: 1 })?.text).toBe(
      "1 item was removed earlier on this PC only by Keep for N months. Your other devices may still hold it.",
    );
    expect(leftForYouLine({ expired_here_only: 2 })?.text).toBe(
      "2 items were removed earlier on this PC only by Keep for N months. Your other devices may still hold them.",
    );
  });
});

describe("row labels", () => {
  // That no receipt holds a label is guarded in core (`labels_reach_the_review_rows_and_never_a_receipt`): receipts
  // carry no items. What this shell can get wrong is sending the label back, which the last line checks.
  it("a row shows its label over its kind and date, and only kind and id go back to core", async () => {
    const labelled: ClientItem = { ...item("email", "7"), label: "Ostrander quarterly figures" };
    expect(rowTitle(labelled, "en")).toBe("Ostrander quarterly figures");
    expect(rowDetail(labelled, "en")).toMatch(/^Email, /);
    const bare = item("meeting", "1");
    expect(rowTitle(bare, "en")).toMatch(/^Meeting, /);
    expect(rowDetail(bare, "en")).toBeNull();
    expect(rowTitle({ ...bare, label: "   " }, "en")).toMatch(/^Meeting, /);

    const bridge = new FakeBridge();
    bridge.inventories = [ok(inventory([labelled]))];
    const { m } = await reviewed(bridge);
    m.setTicked(labelled, true);
    m.setConfirmText(NAME);
    await m.delete();
    expect(m.phase.kind).toBe("receipt");
    expect(bridge.deleteCalls[0].includePossible).toEqual([{ kind: "email", id: "7" }]);
  });
});

describe("settings", () => {
  it("enabling_exclusion_shows_already_left_sentence", async () => {
    const bridge = new FakeBridge();
    const s = new ClientSettingsModel("k1", bridge);
    await s.load();
    await s.setExcludeHosted(true);
    expect(s.exclusionNotice?.alreadyLeft).toBe("Material that already left cannot be recalled.");
    expect(s.exclusionNotice?.hostedSummaries).toEqual([11, 12]);
    const first = s.resummarize();
    await s.resummarize();
    await first;
    expect(bridge.resummarized).toEqual([[11, 12]]);
    expect(s.exclusionNotice?.resummaryNotMade).toBe(0);

    // Core's sentence, or ours when the offer can't be read.
    const quiet = new FakeBridge();
    quiet.offer = err("failed");
    const q = new ClientSettingsModel("k1", quiet);
    await q.load();
    await q.setExcludeHosted(true);
    expect(q.exclusionNotice?.alreadyLeft).toBe("Material that already left cannot be recalled.");
    await q.setExcludeHosted(false);
    expect(q.exclusionNotice).toBeNull();
  });

  it("a write applies to the value synced since the load, not the stale one", async () => {
    const bridge = new FakeBridge();
    const s = new ClientSettingsModel("k1", bridge);
    await s.load();
    expect(s.excludeHosted).toBe(false);
    // Another device turns exclusion on; the page has not seen it.
    bridge.stored = { exclude_hosted: true, keep_months: null };
    await s.setKeepMonths(12);
    expect(bridge.updateCalls).toEqual([{ keep_months: 12 }]);
    expect(bridge.stored).toEqual({ exclude_hosted: true, keep_months: 12 });
    expect(s.excludeHosted).toBe(true);
    expect(s.exclusionNotice).toBeNull();
  });

  it("writes one at a time, so keep and exclude never undo each other", async () => {
    const bridge = new FakeBridge();
    const s = new ClientSettingsModel("k1", bridge);
    await s.load();
    await Promise.all([s.setKeepMonths(6), s.setExcludeHosted(true)]);
    expect(bridge.stored).toEqual({ exclude_hosted: true, keep_months: 6 });
  });

  it("says how many re-summaries were not made", async () => {
    const bridge = new FakeBridge();
    bridge.notMade = 1;
    const s = new ClientSettingsModel("k1", bridge);
    await s.load();
    await s.setExcludeHosted(true);
    await s.resummarize();
    expect(s.exclusionNotice?.resummaryNotMade).toBe(1);
  });

  it("re-summaries wait out only the busy refusal; any other refusal is not made at once", async () => {
    const busy = { kind: "invalid_input", message: "Whimbrel is still working on a meeting. Try again when it's done." };
    const other = { kind: "invalid_input", message: "This meeting has no transcript to summarize." };
    const replies: Record<number, unknown[]> = { 1: [busy, busy, null], 2: [other, null], 3: [null] };
    const calls: number[] = [];
    const waits: number[] = [];
    const notMade = await regenerateInTurn(
      [1, 2, 3],
      async (id) => {
        calls.push(id);
        const r = replies[id].shift();
        if (r) throw r;
      },
      async (ms) => {
        waits.push(ms);
      },
    );
    expect(calls).toEqual([1, 1, 1, 2, 3]);
    expect(waits).toHaveLength(2);
    expect(notMade).toBe(1);
  });

  it("a topic that is not a live client shows no section", async () => {
    const bridge = new FakeBridge();
    bridge.control = async () => err("not_found");
    const s = new ClientSettingsModel("k1", bridge);
    await s.load();
    expect(s.isClient).toBe(false);
  });
});

describe("keep for N months review", () => {
  const due = (id: string) => item("email", id);
  const expiry = (over: Partial<ClientExpiryReceipt> = {}): ClientExpiryReceipt => ({
    date_unix_ms: 1,
    counts: [["email", 1]],
    not_due: [],
    waiting_to_sync: [],
    more: false,
    failed: [],
    failed_stages: [],
    left_for_you: [],
    complete: true,
    ...over,
  });

  it("keeps, snoozes and expires the rest while core says more", async () => {
    const bridge = new FakeBridge();
    bridge.due = ok([{ key: "k1", items: [due("1"), due("2"), due("3"), due("4")] }]);
    bridge.expires = [ok(expiry({ more: true })), ok(expiry())];
    bridge.dueFor = [ok([due("2"), due("9")])];
    const m = new ClientRetentionReviewModel({ k1: NAME }, bridge, () => 1_000);
    await m.load();
    expect(m.tickedCount).toBe(4);
    await m.keep("k1", due("3"));
    await m.snooze("k1", due("4"));
    expect(bridge.decisions).toEqual([{ decision: "keep" }, { decision: "snooze", until_ms: 1_000 + 30 * 86_400_000 }]);
    await m.deleteTicked();
    expect(bridge.expireCalls).toEqual([
      [
        { kind: "email", id: "1" },
        { kind: "email", id: "2" },
      ],
      [{ kind: "email", id: "2" }],
    ]);
    expect(m.phase.kind === "done" && summaryComplete(m.phase.summary)).toBe(true);
  });

  it("a failed re-read in the expiry loop never reports done", async () => {
    const bridge = new FakeBridge();
    bridge.due = ok([{ key: "k1", items: [due("1"), due("2")] }]);
    bridge.expires = [ok(expiry({ more: true }))];
    bridge.dueFor = [err("failed")];
    const m = new ClientRetentionReviewModel({}, bridge);
    await m.load();
    await m.deleteTicked();
    expect(m.phase.kind).toBe("done");
    if (m.phase.kind === "done") {
      expect(m.phase.summary.stopped).toBe("failed");
      expect(summaryComplete(m.phase.summary)).toBe(false);
    }
  });
});

describe("export", () => {
  it("a cancelled pick goes back to the review; a network share says why", async () => {
    const bridge = new FakeBridge();
    bridge.exports = [
      ok(null),
      err("folder_not_allowed"),
      ok({
        folder: "C:\\x\\Export",
        media: { files: 1, skipped: 0, failed: 1 },
        partial: true,
      }),
    ];
    const { m } = await reviewed(bridge, "export");
    await m.export();
    expect(m.phase.kind).toBe("review");
    await m.export();
    expect(m.phase).toEqual({ kind: "failed", error: "folder_not_allowed" });
    m.backToReview();
    m.setIncludeMedia(true);
    await m.export();
    expect(m.phase).toEqual({
      kind: "exported",
      outcome: {
        folder: "C:\\x\\Export",
        media: { files: 1, skipped: 0, failed: 1 },
        partial: true,
      },
    });
  });
});

describe("final review copy (14b)", () => {
  it("Left for you says derived copies stay on other devices until their own retention", () => {
    const line = leftForYouLine("copies_on_other_devices")!.text;
    expect(line).toContain("Routine results, profile suggestions, people's names and meeting briefs");
    expect(line).toContain("the delete does not remove them there");
    // No retention removes routine results or people: the line promises no end date.
    expect(line).not.toContain("retention");
    // A keep-for-N-months run never reaches other devices: its result has its own line.
    const expiry = expiryLeftForYouLine("copies_on_other_devices")!.text;
    expect(expiry).toContain("on this PC only");
    expect(expiry).not.toContain("applies the delete");
    expect(expiryLeftForYouLine("mailbox_originals")).toEqual(leftForYouLine("mailbox_originals"));
    expect(line).not.toBe("Copies on your other devices, until each one syncs and applies the delete.");
    expect(derivedLine()).toMatch(/^On this PC, /);
    expect(reappearsLine()).toContain("new mail or a new meeting");
  });

  it("a clean-up that failed after the client record went offers no Try again and says what to do", async () => {
    const r = receipt({ client: "removed", complete: false, retry_helps: true, failed_stages: ["cleanup"] });
    expect(receiptHeadline(r)).not.toContain("Try again");
    expect(receiptHeadline(r)).toContain("can't be run again");
    expect(receiptHeadline(r)).toContain("next time you delete a client");
    expect(receiptRetryHelps(r)).toBe(false);
    expect(receiptRetryHelps({ ...r, client: "removed_here_only" })).toBe(false);
    expect(receiptRetryHelps({ ...r, client: "still_present" })).toBe(true);
    const bridge = new FakeBridge();
    bridge.deletes = [ok(r)];
    const { m } = await reviewed(bridge);
    m.setConfirmText(NAME);
    await m.delete();
    const before = bridge.inventoryCalls;
    await m.retry();
    expect(bridge.deleteCalls).toHaveLength(1);
    expect(bridge.inventoryCalls).toBe(before);
    expect(m.phase.kind).toBe("receipt");
  });

  it("the Exclude toggle warns while devices hold the setting back, and says nothing when unknown", async () => {
    expect(heldDevicesLine(null)).toBeNull();
    expect(heldDevicesLine(0)).toBeNull();
    expect(heldDevicesLine(1)).toMatch(/^1 device on your sync hasn't updated yet\./);
    expect(heldDevicesLine(3)).toMatch(/^3 devices on your sync haven't updated yet\./);
    expect(heldDevicesLine(3)).toContain("could still send this client's material to a hosted model");
    expect(heldDevicesLine(3)).toContain("Update those devices.");
    expect(heldDevicesLine(1)).not.toContain("Set it on each device you use");
    const bridge = new FakeBridge();
    bridge.held = 2;
    const s = new ClientSettingsModel("k1", bridge);
    await s.load();
    expect(s.heldDevices).toBe(2);
    const unknown = new FakeBridge();
    unknown.heldDevices = async () => {
      throw new Error("no shell");
    };
    const u = new ClientSettingsModel("k1", unknown);
    await u.load();
    expect(u.heldDevices).toBeNull();
    expect(u.saved).not.toBeNull();
  });

  it("states the matcher's limit, and the left-out note counts items in the right number", () => {
    expect(matchingLimitLine()).toContain("whole words");
    expect(leftOutLine(0)).toBeNull();
    expect(leftOutLine(1)).toBe("Left out 1 item that involves an excluded client.");
    expect(leftOutLine(4)).toBe("Left out 4 items that involve an excluded client.");
  });

  it("an expiry refused for no device id says so in its own words", async () => {
    const bridge = new FakeBridge();
    bridge.due = ok([{ key: "k1", items: [item("email", "7")] }]);
    bridge.expires = [err("no_device_id")];
    const m = new ClientRetentionReviewModel({ k1: NAME }, bridge);
    await m.load();
    await m.deleteTicked();
    expect(m.phase.kind).toBe("done");
    if (m.phase.kind !== "done") return;
    expect(m.phase.summary.stopped).toBe("no_device_id");
    const line = expiryStoppedLine("no_device_id");
    expect(line).toContain("no device identity");
    expect(line).not.toContain("can't delete a client");
    expect(expiryStoppedLine("busy")).toBe(clientMessage("busy"));
  });
});

describe("review rows (PR 875)", () => {
  const cap = (id: string, ms: number, app?: string, label?: string): ClientItem => ({
    ...item("capture", id),
    date_unix_ms: ms,
    ...(app ? { app } : {}),
    ...(label ? { label } : {}),
  });
  const day = new Date(2026, 10, 18, 10).getTime();
  const nextDay = new Date(2026, 10, 19, 10).getTime();

  it("folds captures from one app on one day into one group at the first one's place", () => {
    const rows = reviewRows([
      cap("i:1", day, "WindowsTerminal"),
      item("email", "7"),
      cap("i:2", day + 60_000, "WindowsTerminal"),
      cap("i:3", nextDay, "WindowsTerminal"),
      cap("i:4", day, "MSEdge"),
    ]);
    expect(rows.map((r) => (r.kind === "item" ? r.item.id : r.group.items.map((i) => i.id).join("+")))).toEqual([
      "i:1+i:2",
      "7",
      "i:3",
      "i:4",
    ]);
    const g = rows[0];
    if (g.kind !== "group") throw new Error("group");
    expect(g.group.source).toBe("WindowsTerminal");
    expect(groupTitle(g.group, "en-US")).toMatch(/^Screen captures from WindowsTerminal, .+: 2$/);
  });

  it("groups by window title when a capture has no app, and leaves bare captures alone", () => {
    const rows = reviewRows([cap("i:1", day, undefined, "Quote.xlsx"), cap("i:2", day, undefined, "Quote.xlsx"), cap("i:3", day), cap("i:4", day)]);
    expect(rows.map((r) => r.kind)).toEqual(["group", "item", "item"]);
  });

  it("a group ticks every item, and the delete sends exactly the ticked refs", async () => {
    const caps = [cap("i:1", day, "WindowsTerminal"), cap("i:2", day, "WindowsTerminal")];
    const bridge = new FakeBridge();
    bridge.inventories = [ok(inventory([...caps, item("email", "7")]))];
    const { m } = await reviewed(bridge);
    m.setGroupTicked(caps, true);
    expect(m.isGroupTicked(caps)).toBe(true);
    m.setTicked(caps[1], false);
    expect(m.isGroupTicked(caps)).toBe(false);
    expect(m.tickedCount(caps)).toBe(1);
    m.setConfirmText(NAME);
    await m.delete();
    expect(bridge.deleteCalls[0].includePossible).toEqual([{ kind: "capture", id: "i:1" }]);
  });
});

it("an_added_item_says_what_it_is", () => {
  expect(kindName("mcp_item")).toBe("Added items");
  expect(kindSingular("mcp_item")).toBe("Added item");
  const item: ClientItem = { kind: "mcp_item", id: "12", also_client: null, date_unix_ms: 1_700_000_000_000, removed: false };
  expect(itemLine(item, "en-US").startsWith("Added item, ")).toBe(true);
});
