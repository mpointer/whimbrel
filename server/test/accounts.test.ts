import { it, expect, beforeEach } from "vitest";
import {
  getOrCreateAccount,
  findAccountByGoogleSub,
  findDeviceByPublicKey,
  findDeviceById,
  listDevicesForAccount,
  approveDevice,
  revokeDevice,
  setDeviceName,
  countDevicesForAccount,
  countActiveDevicesForAccount,
  enrollDevice,
} from "../src/accounts";
import { FakeSqlExecutor } from "./testHelpers/fakeSqlExecutor";

let db: FakeSqlExecutor;

beforeEach(() => {
  db = new FakeSqlExecutor();
});

it("finds no account for a google_sub that has never signed in", async () => {
  expect(await findAccountByGoogleSub(db, "google-sub-1")).toBeNull();
});

it("creates a new account with the given data-plane database name and url", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  expect(account.email).toBe("user@example.com");
  expect(account.googleSub).toBe("google-sub-1");
  expect(account.dataDbName).toBe("kestrel-sync-abc123");
  expect(account.dataDbUrl).toBe("https://kestrel-sync-abc123-test-org.turso.io");
});

it("returns the same account (and its original data-plane database name and url) on a later sign-in", async () => {
  const first = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  const second = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    2000,
    "whimbrel-sync-DIFFERENT-name",
    "https://whimbrel-sync-DIFFERENT-name-test-org.turso.io",
  );
  expect(second.id).toBe(first.id);
  expect(second.dataDbName).toBe("kestrel-sync-abc123");
  expect(second.dataDbUrl).toBe("https://kestrel-sync-abc123-test-org.turso.io");
});

it("finds an existing account via findAccountByGoogleSub", async () => {
  const created = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  const found = await findAccountByGoogleSub(db, "google-sub-1");
  expect(found).toEqual(created);
});

it("gives two different accounts different ids and different data-plane databases", async () => {
  const a = await getOrCreateAccount(
    db,
    "a@example.com",
    "google-sub-a",
    1000,
    "whimbrel-sync-aaa",
    "https://whimbrel-sync-aaa-test-org.turso.io",
  );
  const b = await getOrCreateAccount(
    db,
    "b@example.com",
    "google-sub-b",
    1000,
    "whimbrel-sync-bbb",
    "https://whimbrel-sync-bbb-test-org.turso.io",
  );
  expect(a.id).not.toBe(b.id);
  expect(a.dataDbName).not.toBe(b.dataDbName);
  expect(a.dataDbUrl).not.toBe(b.dataDbUrl);
});

it("finds no device for a public key that was never enrolled", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  const found = await findDeviceByPublicKey(db, account.id, "some-public-key");
  expect(found).toBeNull();
});

it("counts zero devices for a brand new account", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  expect(await countDevicesForAccount(db, account.id)).toBe(0);
  expect(await countActiveDevicesForAccount(db, account.id)).toBe(0);
});

it("enrolls the first device as approved and a later device as pending", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );

  const first = await enrollDevice(db, account.id, "device-a-key", "approved", 1000);
  expect(first.status).toBe("approved");
  expect(await countDevicesForAccount(db, account.id)).toBe(1);

  const second = await enrollDevice(db, account.id, "device-b-key", "pending", 2000);
  expect(second.status).toBe("pending");
  expect(await countDevicesForAccount(db, account.id)).toBe(2);
});

it("finds an enrolled device by its public key", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  await enrollDevice(db, account.id, "device-a-key", "approved", 1000);

  const found = await findDeviceByPublicKey(db, account.id, "device-a-key");

  expect(found).not.toBeNull();
  expect(found?.status).toBe("approved");
  expect(found?.accountId).toBe(account.id);
});

it("counts only non-revoked devices as active, unlike countDevicesForAccount", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  await enrollDevice(db, account.id, "device-a-key", "approved", 1000);
  await enrollDevice(db, account.id, "device-b-key", "revoked", 2000);

  expect(await countActiveDevicesForAccount(db, account.id)).toBe(1);
  expect(await countDevicesForAccount(db, account.id)).toBe(2);
});

it("returns null from findDeviceById for an id that was never enrolled", async () => {
  expect(await findDeviceById(db, "no-such-device")).toBeNull();
});

it("finds an enrolled device by its id", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  const device = await enrollDevice(db, account.id, "device-a-key", "approved", 1000);

  const found = await findDeviceById(db, device.id);

  expect(found).not.toBeNull();
  expect(found?.id).toBe(device.id);
  expect(found?.publicKey).toBe("device-a-key");
});

it("lists every device for an account and none for another", async () => {
  const accountA = await getOrCreateAccount(
    db,
    "a@example.com",
    "google-sub-a",
    1000,
    "whimbrel-sync-aaa",
    "https://whimbrel-sync-aaa-test-org.turso.io",
  );
  const accountB = await getOrCreateAccount(
    db,
    "b@example.com",
    "google-sub-b",
    1000,
    "whimbrel-sync-bbb",
    "https://whimbrel-sync-bbb-test-org.turso.io",
  );
  await enrollDevice(db, accountA.id, "device-a1-key", "approved", 1000);
  await enrollDevice(db, accountA.id, "device-a2-key", "pending", 2000);
  await enrollDevice(db, accountB.id, "device-b1-key", "approved", 1000);

  const devicesForA = await listDevicesForAccount(db, accountA.id);
  const devicesForB = await listDevicesForAccount(db, accountB.id);

  expect(devicesForA).toHaveLength(2);
  expect(devicesForA.map((d) => d.publicKey).sort()).toEqual(["device-a1-key", "device-a2-key"]);
  expect(devicesForB).toHaveLength(1);
  expect(devicesForB[0].publicKey).toBe("device-b1-key");
});

it("returns an empty list for an account with no devices", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  expect(await listDevicesForAccount(db, account.id)).toEqual([]);
});

it("flips a pending device's status to approved", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  const device = await enrollDevice(db, account.id, "device-a-key", "pending", 1000);

  await approveDevice(db, device.id);

  const found = await findDeviceById(db, device.id);
  expect(found?.status).toBe("approved");
});

it("marks a device revoked and records when", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  const device = await enrollDevice(db, account.id, "device-a-key", "approved", 1000);

  await revokeDevice(db, device.id, 2000);

  const found = await findDeviceById(db, device.id);
  expect(found?.status).toBe("revoked");
  expect(found?.revokedAtUnixMs).toBe(2000);
});

it("sets a device's name", async () => {
  const account = await getOrCreateAccount(
    db,
    "user@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-abc123",
    "https://kestrel-sync-abc123-test-org.turso.io",
  );
  const device = await enrollDevice(db, account.id, "device-a-key", "approved", 1000);

  await setDeviceName(db, device.id, "MacBook Pro");

  const found = await findDeviceById(db, device.id);
  expect(found?.name).toBe("MacBook Pro");
});
