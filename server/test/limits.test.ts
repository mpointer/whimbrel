import { it, expect } from "vitest";
import { limitCheck } from "../src/limits";
import type { DeviceRecord } from "../src/accounts";

let n = 0;
function dev(platform: string | null, status: DeviceRecord["status"] = "approved", replacesDeviceId: string | null = null): DeviceRecord {
  n += 1;
  return {
    id: `d${n}`,
    accountId: "a",
    publicKey: `k${n}`,
    name: `Device ${n}`,
    platform,
    status,
    enrolledAtUnixMs: n,
    revokedAtUnixMs: null,
    replacesDeviceId,
  };
}

it("a second computer is over the limit", () => {
  const mac = dev("mac");
  expect(limitCheck([mac], "windows", false)).toEqual({ ok: false, kind: "computer", holders: [mac] });
});

it("a phone fits beside one computer", () => {
  expect(limitCheck([dev("mac")], "ios", false)).toEqual({ ok: true });
});

it("a second phone is over the limit", () => {
  const phone = dev("ios", "pending");
  expect(limitCheck([dev("mac"), phone], "ios", false)).toEqual({ ok: false, kind: "phone", holders: [phone] });
});

it("revoked devices don't count", () => {
  expect(limitCheck([dev("mac", "revoked")], "mac", false)).toEqual({ ok: true });
});

it("a device being replaced doesn't count", () => {
  const old = dev("mac");
  const swap = dev("mac", "pending", old.id);
  // The swap itself still holds the computer slot, so check without it.
  expect(limitCheck([old, dev("ios")], "mac", false)).toMatchObject({ ok: false });
  expect(limitCheck([old, swap], "mac", false)).toEqual({ ok: false, kind: "computer", holders: [swap] });
  expect(limitCheck([old, dev("windows", "revoked", old.id)], "mac", false)).toMatchObject({ ok: false });
});

it("legacy rows without a platform count as computers", () => {
  expect(limitCheck([dev(null)], "mac", false)).toMatchObject({ ok: false, kind: "computer" });
});

it("unlimited is never over", () => {
  expect(limitCheck([dev("mac"), dev("windows"), dev("ios")], "mac", true)).toEqual({ ok: true });
});

it("a resetting device holds its slot and stands in for the device it replaces", () => {
  const old = dev("mac");
  const resetting = dev("mac", "resetting", old.id);
  expect(limitCheck([old, resetting], "mac", false)).toEqual({ ok: false, kind: "computer", holders: [resetting] });
});
