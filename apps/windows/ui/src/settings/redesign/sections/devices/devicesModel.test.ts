import { describe, expect, it } from "vitest";
import type { DeviceView } from "../../../../generated/types";
import { deviceActionName, deviceChip, deviceLines, deviceNotes, syncTone } from "./devicesModel";

const device = (over: Partial<DeviceView> = {}): DeviceView => ({ id: "d1", publicKey: "ABCDEFGHIJKLMNOP", name: "Laptop", status: "approved", thisDevice: false, platform: "windows", enrolledAtUnixMs: 1, comparisonCode: null, ...over });

describe("the Sync status tone", () => {
  it("is good only while sync is on, a problem for an error, and a warning while it waits", () => {
    expect(syncTone("ready")).toBe("good");
    expect(syncTone("error")).toBe("problem");
    for (const s of ["signingIn", "waitingForApproval", "waitingForGroupKey"] as const) expect(syncTone(s)).toBe("warning");
    expect(syncTone("notSignedIn")).toBe("neutral");
    expect(syncTone("notConfigured")).toBe("neutral");
  });
});

describe("a device row", () => {
  it("says This PC for this one and the server's status word for the others", () => {
    expect(deviceChip(device({ thisDevice: true }))).toEqual({ tone: "neutral", text: "This PC" });
    expect(deviceChip(device({ status: "approved" }))).toEqual({ tone: "good", text: "approved" });
    expect(deviceChip(device({ status: "pending" }))).toEqual({ tone: "warning", text: "pending" });
    expect(deviceChip(device({ status: "revoked" }))).toEqual({ tone: "neutral", text: "revoked" });
  });

  it("puts what a pending device must show in its own notes, then the start of its key in the caption", () => {
    const d = device({ status: "pending", comparisonCode: "4821" });
    expect(deviceNotes(d, { collides: false })).toEqual([{ text: "Check this matches the code on the new device: 4821", warning: false }]);
    expect(deviceLines(d, { runnerLastRun: null, locale: "en" })).toEqual(["ABCDEFGHIJKL…"]);
    expect(deviceNotes(device({ status: "approved" }), { collides: false })).toEqual([]);
  });

  it("says a pending device cannot be verified, and when another shows the same code, without a dash", () => {
    const notes = deviceNotes(device({ status: "pending" }), { collides: true });
    expect(notes[0].text).toBe("Can't verify this device");
    expect(notes[1]).toEqual({ text: "Another pending device shows the same code. Do not approve it.", warning: true });
    expect(notes.map((n) => n.text).join(" ")).not.toMatch(/[\u2013\u2014]/);
  });

  it("adds when Whimbrel Cloud last ran, only for an approved one", () => {
    const run = { runnerLastRun: Date.UTC(2026, 10, 9, 12), locale: "en" };
    expect(deviceLines(device({ platform: "cloud" }), run).join(" ")).toMatch(/Last ran/);
    expect(deviceLines(device({ platform: "cloud", status: "revoked" }), run).join(" ")).not.toMatch(/Last ran/);
    expect(deviceLines(device(), run).join(" ")).not.toMatch(/Last ran/);
  });

  it("names the device in each button", () => {
    expect(deviceActionName("Revoke…", device())).toBe("Revoke…, Laptop");
    expect(deviceActionName("Revoke…", device({ name: null }))).toBe("Revoke…, Unnamed device");
  });
});
