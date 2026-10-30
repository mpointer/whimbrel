import { it, expect, vi, beforeEach } from "vitest";
import * as googleAuth from "../src/googleAuth";
import * as turso from "../src/turso";
import * as accounts from "../src/accounts";
import worker, {
  handleGoogleAuth,
  handleListDevices,
  handleApproveDevice,
  handleRevokeDevice,
  handleSetDeviceName,
  handleSwapDevice,
  handleWaitlist,
  handleResetAccount,
  handleDeleteAccount,
} from "../src/index";
import type { Env } from "../src/index";
import { FakeSqlExecutor } from "./testHelpers/fakeSqlExecutor";
import { deleteAccount, runInactivity } from "../src/inactivity";
import * as handoff from "../src/handoff";
import { fetchAndConsumeWrappedKey } from "../src/handoff";
import { env } from "cloudflare:test";

function makeEnv(overrides: Partial<Env> = {}): Env {
  return {
    GOOGLE_CLIENT_ID: "test-client-id",
    TURSO_DATABASE_URL: "https://test-control-plane.example.turso.io",
    TURSO_DATABASE_AUTH_TOKEN: "test-database-token",
    TURSO_PLATFORM_API_TOKEN: "test-platform-token",
    TURSO_ORG_SLUG: "test-org",
    TURSO_DB_GROUP: "test-group",
    ALLOWED_GOOGLE_EMAILS: "owner@example.com",
    ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF,
    ...overrides,
  };
}

function postAuth(body: unknown): Request {
  return new Request("https://example.com/v1/auth/google", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

function postDevices(body: unknown): Request {
  return new Request("https://example.com/v1/devices", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

beforeEach(() => {
  vi.restoreAllMocks();
  // Older tests mock accounts.* and hit no real DB; last-seen bookkeeping is covered by its own test.
  vi.spyOn(accounts, "touchLastSeen").mockResolvedValue(undefined);
});

it("returns 404 for an unrouted path", async () => {
  const response = await worker.fetch(new Request("https://example.com/nope"), makeEnv());
  expect(response.status).toBe(404);
});

it("returns 502 instead of an unhandled exception when the real Turso connection can't be constructed", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });

  const response = await worker.fetch(
    postAuth({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv({ TURSO_DATABASE_URL: "" }),
  );

  expect(response.status).toBe(502);
});

it("returns 400 when idToken is missing", async () => {
  const response = await worker.fetch(postAuth({ devicePublicKey: "key" }), makeEnv());
  expect(response.status).toBe(400);
});

it("returns 400 when devicePublicKey is missing", async () => {
  const response = await worker.fetch(postAuth({ idToken: "token" }), makeEnv());
  expect(response.status).toBe(400);
});

it("returns 400 when idToken is an empty string", async () => {
  const response = await worker.fetch(postAuth({ idToken: "", devicePublicKey: "key" }), makeEnv());
  expect(response.status).toBe(400);
});

it("returns 400 when devicePublicKey is an empty string", async () => {
  const response = await worker.fetch(postAuth({ idToken: "token", devicePublicKey: "" }), makeEnv());
  expect(response.status).toBe(400);
});

it("returns 400 from /v1/devices when devicePublicKey is an empty string", async () => {
  const response = await worker.fetch(postDevices({ idToken: "token", devicePublicKey: "" }), makeEnv());
  expect(response.status).toBe(400);
});

it("returns 400 for a malformed JSON body instead of a 500", async () => {
  const request = new Request("https://example.com/v1/auth/google", {
    method: "POST",
    body: "not valid json{{{",
  });
  const response = await worker.fetch(request, makeEnv());
  expect(response.status).toBe(400);
});

it("returns 401 when Google ID token verification fails", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockRejectedValue(new Error("bad token"));

  const response = await worker.fetch(postAuth({ idToken: "bad", devicePublicKey: "key" }), makeEnv());

  expect(response.status).toBe(401);
});

it("returns 403 when the verified email is not on the allowlist", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "not-allowed@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });

  const response = await worker.fetch(
    postAuth({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(403);
  expect(await response.json()).toEqual({ error: "email not allowed" });
});

it("returns 403 for every email when the allowlist is empty", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });

  const response = await worker.fetch(
    postAuth({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv({ ALLOWED_GOOGLE_EMAILS: "" }),
  );

  expect(response.status).toBe(403);
  expect(await response.json()).toEqual({ error: "email not allowed" });
});

it("returns 502 when the account/token backend throws", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockRejectedValue(new Error("turso unreachable"));

  const response = await worker.fetch(
    postAuth({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(502);
});

it("provisions a new data-plane database only for a brand new account, then mints a token scoped to it", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue(null);
  const createDbSpy = vi.spyOn(turso, "createAccountDatabase").mockResolvedValue({
    name: "kestrel-sync-abc123",
    url: "https://kestrel-sync-abc123-test-org.turso.io",
  });
  vi.spyOn(accounts, "getOrCreateAccount").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-abc123",
    dataDbUrl: "https://kestrel-sync-abc123-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue(null);
  vi.spyOn(accounts, "countActiveDevicesForAccount").mockResolvedValue(0);
  vi.spyOn(accounts, "listDevicesForAccount").mockResolvedValue([]);
  vi.spyOn(accounts, "enrollDevice").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  const mintSpy = vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("minted-turso-jwt");

  const response = await worker.fetch(
    postAuth({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF }),
  );

  expect(response.status).toBe(200);
  const body = (await response.json()) as { status: string; tursoToken: string; tursoDatabaseUrl: string; wrappedGroupKey: string | null };
  expect(body).toEqual({
    status: "approved",
    tursoToken: "minted-turso-jwt",
    tursoDatabaseUrl: "https://kestrel-sync-abc123-test-org.turso.io",
    wrappedGroupKey: null,
  });
  expect(createDbSpy).toHaveBeenCalledOnce();
  expect(mintSpy).toHaveBeenCalledWith(
    expect.objectContaining({ organizationSlug: "test-org", databaseName: "kestrel-sync-abc123" }),
    expect.any(String),
  );
});

it("does not provision a new database for an account that already exists", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  const createDbSpy = vi.spyOn(turso, "createAccountDatabase");
  vi.spyOn(accounts, "getOrCreateAccount").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("refreshed-turso-jwt");

  const response = await worker.fetch(
    postAuth({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF }),
  );

  expect(response.status).toBe(200);
  const body = (await response.json()) as { status: string; tursoToken: string; tursoDatabaseUrl: string; wrappedGroupKey: string | null };
  expect(body).toEqual({
    status: "approved",
    tursoToken: "refreshed-turso-jwt",
    tursoDatabaseUrl: "https://kestrel-sync-existing-test-org.turso.io",
    wrappedGroupKey: null,
  });
  expect(createDbSpy).not.toHaveBeenCalled();
});

it("marks a second device pending instead of minting a token", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "getOrCreateAccount").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue(null);
  vi.spyOn(accounts, "countActiveDevicesForAccount").mockResolvedValue(1);
  vi.spyOn(accounts, "enrollDevice").mockResolvedValue({
    id: "device-2",
    accountId: "account-1",
    publicKey: "device-b-key",
    name: null,
    platform: null,
    status: "pending",
    enrolledAtUnixMs: 2000,
    revokedAtUnixMs: null,
  });
  const mintSpy = vi.spyOn(turso, "mintDeviceToken");

  const response = await worker.fetch(
    postAuth({ idToken: "good-token", devicePublicKey: "device-b-key" }),
    makeEnv({ UNLIMITED_EMAILS: "owner@example.com" }), // predates the device limit
  );

  expect(response.status).toBe(200);
  const body = (await response.json()) as { status: string };
  expect(body).toEqual({ status: "pending" });
  expect(mintSpy).not.toHaveBeenCalled();
});

it("returns 403 for a previously revoked device", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "getOrCreateAccount").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "revoked",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: 1500,
  });

  const response = await worker.fetch(
    postAuth({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(403);
});

it("returns pending status for a device that already exists and is still pending, without minting a token", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "getOrCreateAccount").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-2",
    accountId: "account-1",
    publicKey: "device-b-key",
    name: null,
    platform: null,
    status: "pending",
    enrolledAtUnixMs: 2000,
    revokedAtUnixMs: null,
  });
  const mintSpy = vi.spyOn(turso, "mintDeviceToken");

  const response = await worker.fetch(
    postAuth({ idToken: "good-token", devicePublicKey: "device-b-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(200);
  const body = (await response.json()) as { status: string };
  expect(body).toEqual({ status: "pending" });
  expect(mintSpy).not.toHaveBeenCalled();
});

it("integration: real accounts.ts logic auto-approves the first device, provisions one database, and defers a second device to pending", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  const createDbSpy = vi.spyOn(turso, "createAccountDatabase").mockResolvedValue({
    name: "kestrel-sync-abc123",
    url: "https://kestrel-sync-abc123-test-org.turso.io",
  });
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("minted-turso-jwt");
  const fakeDb = new FakeSqlExecutor();

  const first = await handleGoogleAuth(
    postAuth({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF }),
    fakeDb,
  );
  expect(first.status).toBe(200);
  expect(await first.json()).toEqual({
    status: "approved",
    tursoToken: "minted-turso-jwt",
    tursoDatabaseUrl: "https://kestrel-sync-abc123-test-org.turso.io",
    wrappedGroupKey: null,
  });

  const second = await handleGoogleAuth(
    postAuth({ idToken: "good-token", devicePublicKey: "device-b-key" }),
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF, UNLIMITED_EMAILS: "owner@example.com" }),
    fakeDb,
  );
  expect(second.status).toBe(200);
  expect(await second.json()).toEqual({ status: "pending" });

  const firstAgain = await handleGoogleAuth(
    postAuth({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF }),
    fakeDb,
  );
  expect(firstAgain.status).toBe(200);
  expect(await firstAgain.json()).toEqual({
    status: "approved",
    tursoToken: "minted-turso-jwt",
    tursoDatabaseUrl: "https://kestrel-sync-abc123-test-org.turso.io",
    wrappedGroupKey: null,
  });

  // The database was provisioned exactly once — the second call (an
  // already-existing account signing in a second device) must not
  // provision a second database.
  expect(createDbSpy).toHaveBeenCalledOnce();
});

it("returns the wrapped group key when one is waiting in the handoff store", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("refreshed-turso-jwt");

  const fakeDb = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(
    fakeDb,
    "owner@example.com",
    "google-sub-1",
    1000,
    "kestrel-sync-existing",
    "https://kestrel-sync-existing-test-org.turso.io",
  );
  const device = await accounts.enrollDevice(fakeDb, account.id, "device-b-key", "approved", 2000);
  await env.ENROLLMENT_HANDOFF.put(`handoff:${device.id}`, "the-wrapped-key-blob");

  const response = await handleGoogleAuth(
    postAuth({ idToken: "good-token", devicePublicKey: "device-b-key" }),
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF }),
    fakeDb,
  );

  expect(response.status).toBe(200);
  const body = (await response.json()) as {
    status: string;
    tursoToken: string;
    tursoDatabaseUrl: string;
    wrappedGroupKey: string | null;
  };
  expect(body.wrappedGroupKey).toBe("the-wrapped-key-blob");

  const secondResponse = await handleGoogleAuth(
    postAuth({ idToken: "good-token", devicePublicKey: "device-b-key" }),
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF }),
    fakeDb,
  );
  const secondBody = (await secondResponse.json()) as { wrappedGroupKey: string | null };
  expect(secondBody.wrappedGroupKey).toBeNull();
});

it("integration: two different accounts get tokens scoped to two different data-plane databases", async () => {
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("some-turso-jwt");
  vi.spyOn(turso, "createAccountDatabase").mockImplementation(async (_config, name) => ({
    name,
    url: `https://${name}-test-org.turso.io`,
  }));
  vi.spyOn(googleAuth, "verifyGoogleIdToken")
    .mockResolvedValueOnce({ email: "owner@example.com", googleSub: "google-sub-account-a", audience: "test-client-id" })
    .mockResolvedValueOnce({ email: "owner@example.com", googleSub: "google-sub-account-b", audience: "test-client-id" });
  const mintSpy = vi.spyOn(turso, "mintDeviceToken");
  const fakeDb = new FakeSqlExecutor();

  const firstResponse = await handleGoogleAuth(
    postAuth({ idToken: "token-a", devicePublicKey: "device-a-key" }),
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF }),
    fakeDb,
  );
  expect(firstResponse.status).toBe(200);

  const secondResponse = await handleGoogleAuth(
    postAuth({ idToken: "token-b", devicePublicKey: "device-b-key" }),
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF }),
    fakeDb,
  );
  expect(secondResponse.status).toBe(200);

  expect(mintSpy).toHaveBeenCalledTimes(2);
  const firstDbName = (mintSpy.mock.calls[0][0] as { databaseName: string }).databaseName;
  const secondDbName = (mintSpy.mock.calls[1][0] as { databaseName: string }).databaseName;
  expect(firstDbName).not.toBe(secondDbName);
});

it("returns 400 from /v1/devices when idToken is missing", async () => {
  const response = await worker.fetch(postDevices({ devicePublicKey: "key" }), makeEnv());
  expect(response.status).toBe(400);
});

it("returns 401 from /v1/devices when Google ID token verification fails", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockRejectedValue(new Error("bad token"));

  const response = await worker.fetch(
    postDevices({ idToken: "bad", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(401);
});

it("returns 403 from /v1/devices when the verified email is not on the allowlist", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "not-allowed@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });

  const response = await worker.fetch(
    postDevices({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(403);
});

it("returns 403 from /v1/devices when the caller's account doesn't exist", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue(null);

  const response = await worker.fetch(
    postDevices({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(403);
});

it("returns 502 from /v1/devices when the account/token backend throws", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockRejectedValue(new Error("turso unreachable"));

  const response = await worker.fetch(
    postDevices({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(502);
});

it("returns 403 from /v1/devices when the caller's own device isn't approved", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-2",
    accountId: "account-1",
    publicKey: "device-b-key",
    name: null,
    platform: null,
    status: "pending",
    enrolledAtUnixMs: 2000,
    revokedAtUnixMs: null,
  });

  const response = await worker.fetch(
    postDevices({ idToken: "good-token", devicePublicKey: "device-b-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(403);
});

it("lists an account's devices for an approved caller", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  const devices = [
    {
      id: "device-1",
      accountId: "account-1",
      publicKey: "device-a-key",
      name: null,
      platform: null,
      status: "approved" as const,
      enrolledAtUnixMs: 1000,
      revokedAtUnixMs: null,
    },
    {
      id: "device-2",
      accountId: "account-1",
      publicKey: "device-b-key",
      name: null,
      platform: null,
      status: "pending" as const,
      enrolledAtUnixMs: 2000,
      revokedAtUnixMs: null,
    },
  ];
  vi.spyOn(accounts, "listDevicesForAccount").mockResolvedValue(devices);

  const response = await worker.fetch(
    postDevices({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(200);
  expect(await response.json()).toEqual({ devices });
});

it("integration: lists real devices via real accounts.ts logic", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  const fakeDb = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(
    fakeDb,
    "owner@example.com",
    "google-sub-1",
    1000,
    "whimbrel-sync-abc",
    "https://whimbrel-sync-abc-test-org.turso.io",
  );
  await accounts.enrollDevice(fakeDb, account.id, "device-a-key", "approved", 1000);
  await accounts.enrollDevice(fakeDb, account.id, "device-b-key", "pending", 2000);

  const response = await handleListDevices(
    postDevices({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
    fakeDb,
  );

  expect(response.status).toBe(200);
  const body = (await response.json()) as { devices: Array<{ publicKey: string; status: string }> };
  expect(body.devices).toHaveLength(2);
  expect(body.devices.map((d) => d.publicKey).sort()).toEqual(["device-a-key", "device-b-key"]);
});

function postApprove(deviceId: string, body: unknown): Request {
  return new Request(`https://example.com/v1/devices/${deviceId}/approve`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

it("returns 400 from the approve route when wrappedGroupKey is missing", async () => {
  const response = await worker.fetch(
    postApprove("device-2", { idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );
  expect(response.status).toBe(400);
});

it("returns 502 from the approve route when the account/token backend throws during auth", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockRejectedValue(new Error("turso unreachable"));

  const response = await worker.fetch(
    postApprove("device-2", {
      idToken: "good-token",
      devicePublicKey: "device-a-key",
      wrappedGroupKey: "wrapped-blob",
    }),
    makeEnv(),
  );

  expect(response.status).toBe(502);
});

it("returns 403 from the approve route when the caller isn't an approved device", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue(null);

  const response = await worker.fetch(
    postApprove("device-2", {
      idToken: "good-token",
      devicePublicKey: "device-a-key",
      wrappedGroupKey: "wrapped-blob",
    }),
    makeEnv(),
  );

  expect(response.status).toBe(403);
});

it("returns 404 from the approve route for a target device that doesn't exist", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  vi.spyOn(accounts, "findDeviceById").mockResolvedValue(null);

  const response = await worker.fetch(
    postApprove("no-such-device", {
      idToken: "good-token",
      devicePublicKey: "device-a-key",
      wrappedGroupKey: "wrapped-blob",
    }),
    makeEnv(),
  );

  expect(response.status).toBe(404);
  expect(await response.json()).toEqual({ error: "device not found" });
});

it("returns 404 (not 403) from the approve route for a target device on a different account", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  vi.spyOn(accounts, "findDeviceById").mockResolvedValue({
    id: "device-other-account",
    accountId: "account-DIFFERENT",
    publicKey: "device-x-key",
    name: null,
    platform: null,
    status: "pending",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });

  const response = await worker.fetch(
    postApprove("device-other-account", {
      idToken: "good-token",
      devicePublicKey: "device-a-key",
      wrappedGroupKey: "wrapped-blob",
    }),
    makeEnv(),
  );

  expect(response.status).toBe(404);
  expect(await response.json()).toEqual({ error: "device not found" });
});

it("returns 400 from the approve route when the target device is revoked", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  vi.spyOn(accounts, "findDeviceById").mockResolvedValue({
    id: "device-2",
    accountId: "account-1",
    publicKey: "device-b-key",
    name: null,
    platform: null,
    status: "revoked",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: 1500,
  });

  const response = await worker.fetch(
    postApprove("device-2", {
      idToken: "good-token",
      devicePublicKey: "device-a-key",
      wrappedGroupKey: "wrapped-blob",
    }),
    makeEnv(),
  );

  expect(response.status).toBe(400);
});

it("allows re-approving an already-approved device (recovers from an expired handoff TTL)", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  vi.spyOn(accounts, "findDeviceById").mockResolvedValue({
    id: "device-2",
    accountId: "account-1",
    publicKey: "device-b-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  vi.spyOn(accounts, "approveDevice").mockResolvedValue(true);

  const testEnv = makeEnv();
  const response = await worker.fetch(
    postApprove("device-2", {
      idToken: "good-token",
      devicePublicKey: "device-a-key",
      wrappedGroupKey: "wrapped-blob",
    }),
    testEnv,
  );

  expect(response.status).toBe(200);
  expect(await response.json()).toEqual({ status: "approved" });
  await expect(fetchAndConsumeWrappedKey(testEnv.ENROLLMENT_HANDOFF, "device-2")).resolves.toBe("wrapped-blob");
});

it("integration: approving a pending device stores its wrapped key in the real KV handoff store", async () => {
  const fakeDb = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(
    fakeDb,
    "owner@example.com",
    "google-sub-1",
    1000,
    "whimbrel-sync-abc",
    "https://whimbrel-sync-abc-test-org.turso.io",
  );
  await accounts.enrollDevice(fakeDb, account.id, "device-a-key", "approved", 1000);
  const pendingDevice = await accounts.enrollDevice(fakeDb, account.id, "device-b-key", "pending", 2000);

  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });

  const response = await handleApproveDevice(
    postApprove(pendingDevice.id, {
      idToken: "good-token",
      devicePublicKey: "device-a-key",
      wrappedGroupKey: "wrapped-blob-data",
    }),
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF, UNLIMITED_EMAILS: "owner@example.com" }), // two Macs
    pendingDevice.id,
    fakeDb,
  );

  expect(response.status).toBe(200);
  expect(await response.json()).toEqual({ status: "approved" });

  const stored = await fetchAndConsumeWrappedKey(env.ENROLLMENT_HANDOFF, pendingDevice.id);
  expect(stored).toBe("wrapped-blob-data");

  const updatedDevice = await accounts.findDeviceById(fakeDb, pendingDevice.id);
  expect(updatedDevice?.status).toBe("approved");
});

function postRevoke(deviceId: string, body: unknown): Request {
  return new Request(`https://example.com/v1/devices/${deviceId}/revoke`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}

it("returns 403 from the revoke route when the caller isn't an approved device", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue(null);

  const response = await worker.fetch(
    postRevoke("device-2", { idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(403);
});

it("returns 404 from the revoke route for a target device that doesn't exist", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  vi.spyOn(accounts, "findDeviceById").mockResolvedValue(null);

  const response = await worker.fetch(
    postRevoke("no-such-device", { idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(404);
  expect(await response.json()).toEqual({ error: "device not found" });
});

it("returns 404 (not 403) from the revoke route for a target device on a different account", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  vi.spyOn(accounts, "findDeviceById").mockResolvedValue({
    id: "device-other-account",
    accountId: "account-DIFFERENT",
    publicKey: "device-x-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });

  const response = await worker.fetch(
    postRevoke("device-other-account", { idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(404);
  expect(await response.json()).toEqual({ error: "device not found" });
});

it("returns 400 from the revoke route when the target device is already revoked", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  vi.spyOn(accounts, "findDeviceById").mockResolvedValue({
    id: "device-2",
    accountId: "account-1",
    publicKey: "device-b-key",
    name: null,
    platform: null,
    status: "revoked",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: 1500,
  });

  const response = await worker.fetch(
    postRevoke("device-2", { idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(400);
});

it("revokes an approved device and rotates all tokens for the account's database", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  vi.spyOn(accounts, "findDeviceById").mockResolvedValue({
    id: "device-2",
    accountId: "account-1",
    publicKey: "device-b-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  const revokeSpy = vi.spyOn(accounts, "revokeDevice").mockResolvedValue(undefined);
  const rotateSpy = vi.spyOn(turso, "rotateAllTokens").mockResolvedValue(undefined);

  const response = await worker.fetch(
    postRevoke("device-2", { idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );

  expect(response.status).toBe(200);
  expect(await response.json()).toEqual({ status: "revoked" });
  expect(revokeSpy).toHaveBeenCalledWith(expect.anything(), "device-2", expect.any(Number));
  expect(rotateSpy).toHaveBeenCalledWith({
    platformApiToken: "test-platform-token",
    organizationSlug: "test-org",
    databaseName: "kestrel-sync-existing",
  });
});

it("integration: revoking a device via real accounts.ts logic updates its status", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(turso, "rotateAllTokens").mockResolvedValue(undefined);
  const fakeDb = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(
    fakeDb,
    "owner@example.com",
    "google-sub-1",
    1000,
    "whimbrel-sync-abc",
    "https://whimbrel-sync-abc-test-org.turso.io",
  );
  await accounts.enrollDevice(fakeDb, account.id, "device-a-key", "approved", 1000);
  const target = await accounts.enrollDevice(fakeDb, account.id, "device-b-key", "approved", 2000);

  const response = await handleRevokeDevice(
    postRevoke(target.id, { idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
    target.id,
    fakeDb,
  );

  expect(response.status).toBe(200);
  const updated = await accounts.findDeviceById(fakeDb, target.id);
  expect(updated?.status).toBe("revoked");
});

function postSetName(body: unknown): Request {
  return new Request("https://example.com/v1/devices/name", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

it("returns 400 from the set-name route when name is missing", async () => {
  const response = await worker.fetch(
    postSetName({ idToken: "good-token", devicePublicKey: "device-a-key" }),
    makeEnv(),
  );
  expect(response.status).toBe(400);
});

it("returns 400 from the set-name route when name is an empty string", async () => {
  const response = await worker.fetch(
    postSetName({ idToken: "good-token", devicePublicKey: "device-a-key", name: "" }),
    makeEnv(),
  );
  expect(response.status).toBe(400);
});

it("returns 403 from the set-name route when the caller isn't an approved device", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue(null);

  const response = await worker.fetch(
    postSetName({ idToken: "good-token", devicePublicKey: "device-a-key", name: "MacBook Pro" }),
    makeEnv(),
  );

  expect(response.status).toBe(403);
});

it("sets the calling device's own name", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  vi.spyOn(accounts, "findAccountByGoogleSub").mockResolvedValue({
    id: "account-1",
    email: "owner@example.com",
    googleSub: "google-sub-1",
    dataDbName: "kestrel-sync-existing",
    dataDbUrl: "https://kestrel-sync-existing-test-org.turso.io",
  });
  vi.spyOn(accounts, "findDeviceByPublicKey").mockResolvedValue({
    id: "device-1",
    accountId: "account-1",
    publicKey: "device-a-key",
    name: null,
    platform: null,
    status: "approved",
    enrolledAtUnixMs: 1000,
    revokedAtUnixMs: null,
  });
  const setNameSpy = vi.spyOn(accounts, "setDeviceName").mockResolvedValue(undefined);

  const response = await worker.fetch(
    postSetName({ idToken: "good-token", devicePublicKey: "device-a-key", name: "MacBook Pro" }),
    makeEnv(),
  );

  expect(response.status).toBe(200);
  expect(await response.json()).toEqual({ status: "ok" });
  expect(setNameSpy).toHaveBeenCalledWith(expect.anything(), "device-1", "MacBook Pro");
});

it("integration: setting a device's name via real accounts.ts logic persists it", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  const fakeDb = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(
    fakeDb,
    "owner@example.com",
    "google-sub-1",
    1000,
    "whimbrel-sync-abc",
    "https://whimbrel-sync-abc-test-org.turso.io",
  );
  const device = await accounts.enrollDevice(fakeDb, account.id, "device-a-key", "approved", 1000);

  const response = await handleSetDeviceName(
    postSetName({ idToken: "good-token", devicePublicKey: "device-a-key", name: "MacBook Pro" }),
    makeEnv(),
    fakeDb,
  );

  expect(response.status).toBe(200);
  const updated = await accounts.findDeviceById(fakeDb, device.id);
  expect(updated?.name).toBe("MacBook Pro");
});

async function nameRouteWithDeviceStatus(status: "pending" | "revoked", name = "iPhone") {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  const fakeDb = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(fakeDb, "owner@example.com", "google-sub-1", 1000, "whimbrel-sync-abc", "https://whimbrel-sync-abc-test-org.turso.io");
  const device = await accounts.enrollDevice(fakeDb, account.id, "device-a-key", status, 1000);
  const response = await handleSetDeviceName(
    postSetName({ idToken: "good-token", devicePublicKey: "device-a-key", name }),
    makeEnv(),
    fakeDb,
  );
  return { response, device: await accounts.findDeviceById(fakeDb, device.id) };
}

it("lets a pending device name itself, so the approve prompt can show it", async () => {
  const { response, device } = await nameRouteWithDeviceStatus("pending", "Mike's iPhone");
  expect(response.status).toBe(200);
  expect(device?.name).toBe("Mike's iPhone");
});

it("strips control and bidi characters and cuts by code point", async () => {
  const { device } = await nameRouteWithDeviceStatus("pending", "\u202EMike\u0007 \u200Bphone" + "😀".repeat(200));
  expect(device?.name).toBe("Mike phone" + "😀".repeat(90));
});

it("collapses whitespace and bounds work on a huge name", async () => {
  const { device } = await nameRouteWithDeviceStatus("pending", "a \u2028  \u00a0 b" + "z".repeat(1_000_000));
  expect(device?.name?.startsWith("a b")).toBe(true);
  expect([...(device?.name ?? "")].length).toBeLessThanOrEqual(100);
});

it("refuses a name that is empty once control characters are stripped", async () => {
  const { response, device } = await nameRouteWithDeviceStatus("pending", "\u202E\u200B");
  expect(response.status).toBe(400);
  expect(device?.name).toBeNull();
});

it("refuses the set-name route for a revoked device", async () => {
  const { response, device } = await nameRouteWithDeviceStatus("revoked");
  expect(response.status).toBe(403);
  expect(device?.name).toBeNull();
});

it("truncates an over-long device name to 100 characters", async () => {
  const { response, device } = await nameRouteWithDeviceStatus("pending", "x".repeat(500));
  expect(response.status).toBe(200);
  expect(device?.name).toBe("x".repeat(100));
});

const IOS = "test-ios-client-id";
function mockAud(audience: string) {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience,
  });
  const createDb = vi.spyOn(turso, "createAccountDatabase").mockResolvedValue({
    name: "kestrel-sync-abc123",
    url: "https://kestrel-sync-abc123-test-org.turso.io",
  });
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("minted-turso-jwt");
  return createDb;
}
const authAs = (db: FakeSqlExecutor, e: Partial<Env>) =>
  handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "phone-key" }), makeEnv(e), db);

it("409 desktop_required for an iOS-audience first device, with no database or device row created", async () => {
  const createDb = mockAud(IOS);
  const db = new FakeSqlExecutor();
  const res = await authAs(db, { GOOGLE_IOS_CLIENT_ID: IOS });
  expect(res.status).toBe(409);
  expect(await res.json()).toEqual({ error: "desktop_required" });
  expect(createDb).not.toHaveBeenCalled();
  expect(await accounts.findAccountByGoogleSub(db, "google-sub-1")).toBeNull();
});

it("409 desktop_required when the account exists but has no active device, and enrolls nothing", async () => {
  mockAud(IOS);
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", "google-sub-1", 1, "n", "u");
  const old = await accounts.enrollDevice(db, account.id, "old-key", "approved", 2);
  await accounts.revokeDevice(db, old.id, 3);
  const res = await authAs(db, { GOOGLE_IOS_CLIENT_ID: IOS });
  expect(res.status).toBe(409);
  expect(await accounts.countDevicesForAccount(db, account.id)).toBe(1);
});

it("an iOS-audience token still enrolls pending when the account has an active device", async () => {
  mockAud(IOS);
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", "google-sub-1", 1, "n", "u");
  await accounts.enrollDevice(db, account.id, "mac-key", "approved", 2);
  const res = await authAs(db, { GOOGLE_IOS_CLIENT_ID: IOS });
  expect(await res.json()).toEqual({ status: "pending" });
});

it("a desktop-audience token as first device is auto-approved", async () => {
  mockAud("test-client-id");
  const res = await authAs(new FakeSqlExecutor(), { GOOGLE_IOS_CLIENT_ID: IOS });
  expect(((await res.json()) as { status: string }).status).toBe("approved");
});

it.each([undefined, ""])("never refuses when GOOGLE_IOS_CLIENT_ID is %j", async (ios) => {
  mockAud("");
  const res = await authAs(new FakeSqlExecutor(), { GOOGLE_IOS_CLIENT_ID: ios });
  expect(res.status).toBe(200);
});

// ---- open sign-in, platform, last-seen ----
function authBody(extra: Record<string, unknown> = {}) {
  return postAuth({ idToken: "t", devicePublicKey: "k1", ...extra });
}
async function firstDevicePlatform(extra: Record<string, unknown>, aud = "test-client-id") {
  mockAud(aud);
  const db = new FakeSqlExecutor();
  // Seed an account so an iOS audience is allowed to enrol (as a pending device).
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", "google-sub-1", 1, "n", "u");
  await accounts.enrollDevice(db, account.id, "mac-key", "approved", 2, "mac");
  await handleGoogleAuth(authBody(extra), makeEnv({ GOOGLE_IOS_CLIENT_ID: IOS, UNLIMITED_EMAILS: "owner@example.com" }), db);
  return (await accounts.findDeviceByPublicKey(db, account.id, "k1"))?.platform;
}

it("open sign-in admits any verified email when OPEN_SIGN_IN is \"true\"", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "stranger@example.com",
    googleSub: "google-sub-9",
    audience: "test-client-id",
  });
  vi.spyOn(turso, "createAccountDatabase").mockResolvedValue({ name: "n", url: "u" });
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("jwt");
  const res = await handleGoogleAuth(authBody(), makeEnv({ OPEN_SIGN_IN: "true" }), new FakeSqlExecutor());
  expect(res.status).toBe(200);
});

it("allowlist still gates when OPEN_SIGN_IN is unset", async () => {
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "stranger@example.com",
    googleSub: "google-sub-9",
    audience: "test-client-id",
  });
  const res = await handleGoogleAuth(authBody(), makeEnv(), new FakeSqlExecutor());
  expect(res.status).toBe(403);
});

it("auth stores the platform sent by a Mac", async () => {
  expect(await firstDevicePlatform({ platform: "windows" })).toBe("windows");
});

it("an iOS audience is stored as ios regardless of body platform", async () => {
  expect(await firstDevicePlatform({ platform: "windows" }, IOS)).toBe("ios");
});

it("a body without platform is stored as mac", async () => {
  expect(await firstDevicePlatform({})).toBe("mac");
});

it("every authenticated route touches last_seen and clears the warning", async () => {
  vi.mocked(accounts.touchLastSeen).mockRestore();
  mockAud("test-client-id");
  vi.spyOn(turso, "rotateAllTokens").mockResolvedValue(undefined);
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", "google-sub-1", 1, "n", "u");
  const a = await accounts.enrollDevice(db, account.id, "k1", "approved", 2, "mac");
  const b = await accounts.enrollDevice(db, account.id, "kb", "pending", 3, "mac");
  const body = { idToken: "t", devicePublicKey: "k1" };
  const post = (path: string, extra: object = {}) =>
    new Request(`https://example.com${path}`, { method: "POST", body: JSON.stringify({ ...body, ...extra }) });
  const calls: [string, () => Promise<Response>][] = [
    ["auth", () => handleGoogleAuth(post("/v1/auth/google"), makeEnv(), db)],
    ["list", () => handleListDevices(post("/v1/devices"), makeEnv(), db)],
    ["name", () => handleSetDeviceName(post("/v1/devices/name", { name: "Desk" }), makeEnv(), db)],
    ["approve", () => handleApproveDevice(post("/x", { wrappedGroupKey: "w" }), makeEnv({ UNLIMITED_EMAILS: "owner@example.com" }), b.id, db)],
    ["revoke", () => handleRevokeDevice(post("/x"), makeEnv(), b.id, db)],
  ];
  for (const [label, call] of calls) {
    db.markWarned(account.id, 5);
    await db.execute("UPDATE device SET last_seen_unix_ms = ? WHERE id = ?", [null, a.id]);
    const res = await call();
    expect(res.status, label).toBe(200);
    expect(db.rawDevice(a.id)?.last_seen_unix_ms, label).toBeGreaterThan(0);
    expect(db.rawAccount(account.id)?.lastSeenUnixMs, label).toBeGreaterThan(0);
    expect(db.rawAccount(account.id)?.warnedAtUnixMs, label).toBeNull();
  }
});

// ---- free device limit, swap, waitlist, reset ----
/** The hosted relay: OPEN_SIGN_IN on, so the free device limit applies. */
const hostedEnv = (overrides: Partial<Env> = {}) => makeEnv({ OPEN_SIGN_IN: "true", ...overrides });
const post = (path: string, body: unknown) =>
  new Request(`https://example.com${path}`, { method: "POST", body: JSON.stringify(body) });
async function seeded() {
  mockAud("test-client-id");
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", "google-sub-1", 1, "old-db", "https://old-db");
  const mac = await accounts.enrollDevice(db, account.id, "mac-key", "approved", 2, "mac");
  await accounts.setDeviceName(db, mac.id, "Office Mac");
  return { db, account, mac };
}
const swapBody = (replaceDeviceId: string, extra: object = {}) => ({
  idToken: "t",
  devicePublicKey: "new-key",
  platform: "windows",
  replaceDeviceId,
  ...extra,
});

it("auth for a second computer returns device_limit with the holder's name and enrols nothing", async () => {
  const { db, account, mac } = await seeded();
  const res = await handleGoogleAuth(authBody({ platform: "windows" }), hostedEnv(), db);
  expect(res.status).toBe(409);
  expect(await res.json()).toEqual({
    error: "device_limit",
    kind: "computer",
    devices: [{ id: mac.id, name: "Office Mac", platform: "mac" }],
  });
  expect(await accounts.countDevicesForAccount(db, account.id)).toBe(1);
});

it("an unlimited email enrols a second computer as pending", async () => {
  const { db } = await seeded();
  const res = await handleGoogleAuth(authBody(), hostedEnv({ UNLIMITED_EMAILS: "Owner@Example.com" }), db);
  expect(await res.json()).toEqual({ status: "pending" });
});

it("two racing computers: only one is enrolled", async () => {
  mockAud("test-client-id");
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", "google-sub-1", 1, "n", "u");
  const [a, b] = await Promise.all([
    handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "r1" }), hostedEnv(), db),
    handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "r2" }), hostedEnv(), db),
  ]);
  expect([a.status, b.status].sort()).toEqual([200, 409]);
  expect(await accounts.countDevicesForAccount(db, account.id)).toBe(1);
});

it("swap enrols pending and approval revokes the replaced device", async () => {
  const { db, account, mac } = await seeded();
  const rotate = vi.spyOn(turso, "rotateAllTokens").mockResolvedValue(undefined);
  const res = await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id)), hostedEnv(), db);
  expect(await res.json()).toEqual({ status: "pending" });
  const fresh = await accounts.findDeviceByPublicKey(db, account.id, "new-key");
  expect(fresh).toMatchObject({ status: "pending", platform: "windows", replacesDeviceId: mac.id });
  expect((await accounts.findDeviceById(db, mac.id))?.status).toBe("approved");

  // While the swap is pending, a third computer is still refused.
  const third = await handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "third" }), hostedEnv(), db);
  expect(third.status).toBe(409);

  const approve = await handleApproveDevice(
    post("/x", { idToken: "t", devicePublicKey: "mac-key", wrappedGroupKey: "w" }),
    hostedEnv(),
    fresh!.id,
    db,
  );
  expect(approve.status).toBe(200);
  expect((await accounts.findDeviceById(db, fresh!.id))?.status).toBe("approved");
  expect((await accounts.findDeviceById(db, mac.id))?.status).toBe("revoked");
  expect(rotate).toHaveBeenCalledOnce();
  expect((await accounts.findDeviceById(db, fresh!.id))?.replacesDeviceId).toBeNull();
});

it("swap of a device on another account is 404", async () => {
  const { db, account } = await seeded();
  const other = await accounts.getOrCreateAccount(db, "x@example.com", "google-sub-2", 1, "n2", "u2");
  const theirs = await accounts.enrollDevice(db, other.id, "their-key", "approved", 2, "mac");
  const res = await handleSwapDevice(post("/v1/devices/swap", swapBody(theirs.id)), makeEnv(), db);
  expect(res.status).toBe(404);
  expect(await accounts.findDeviceByPublicKey(db, account.id, "new-key")).toBeNull();
});

it("swap of a computer for a phone is swap_kind_mismatch", async () => {
  const { db, account } = await seeded();
  const phone = await accounts.enrollDevice(db, account.id, "phone-key", "approved", 3, "ios");
  const bad = await handleSwapDevice(post("/v1/devices/swap", swapBody(phone.id)), makeEnv(), db);
  expect(bad.status).toBe(400);
  expect(await bad.json()).toEqual({ error: "swap_kind_mismatch" });
  expect(await accounts.findDeviceByPublicKey(db, account.id, "new-key")).toBeNull();
});

it("waitlist stores only allowed wants", async () => {
  const { db, account } = await seeded();
  const res = await handleWaitlist(
    post("/v1/waitlist", {
      idToken: "t",
      devicePublicKey: "new-key",
      platform: "windows",
      wants: ["backups", "free_pony", "more_devices", "backups", 7],
    }),
    makeEnv(),
    db,
  );
  expect(await res.json()).toEqual({ status: "ok" });
  expect(db.rawWaitlist()).toEqual([
    { account_id: account.id, wants: '["more_devices","backups"]', platform: "windows", created_at_unix_ms: expect.any(Number) },
  ]);
  const none = await handleWaitlist(post("/v1/waitlist", { idToken: "t", devicePublicKey: "k", wants: ["nope"] }), makeEnv(), db);
  expect(none.status).toBe(400);
  expect(await none.json()).toEqual({ error: "wants_required" });
});

it("reset without confirm is refused", async () => {
  const res = await handleResetAccount(post("/v1/account/reset", { idToken: "t", devicePublicKey: "k" }), makeEnv(), new FakeSqlExecutor());
  expect(res.status).toBe(400);
  expect(await res.json()).toEqual({ error: "reset_confirm_required" });
});

function resetSpies(db: FakeSqlExecutor, env: Partial<Env> = {}) {
  const del = vi.spyOn(turso, "deleteAccountDatabase").mockResolvedValue(undefined);
  const create = vi.spyOn(turso, "createAccountDatabase").mockResolvedValue({ name: "new-db", url: "https://new-db" });
  const mint = vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("fresh-jwt");
  // Keeps an existing implementation (a test watching the call order sets its own first).
  if (!vi.isMockFunction(turso.rotateAllTokens)) vi.spyOn(turso, "rotateAllTokens").mockResolvedValue(undefined);
  const call = () =>
    handleResetAccount(post("/v1/account/reset", { idToken: "t", devicePublicKey: "new-key", confirm: true }), makeEnv(env), db);
  return { del, create, mint, call };
}
async function resetFrom(db: FakeSqlExecutor, env: Partial<Env> = {}) {
  const spies = resetSpies(db, env);
  return { ...spies, res: await spies.call() };
}

it("reset from a swap-pending device revokes others, recreates the database and approves the caller", async () => {
  const { db, account, mac } = await seeded();
  const phone = await accounts.enrollDevice(db, account.id, "phone-key", "approved", 3, "ios");
  await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id)), makeEnv(), db);
  const { res, create, mint } = await resetFrom(db);

  expect(await res.json()).toEqual({
    status: "approved",
    tursoToken: "fresh-jwt",
    tursoDatabaseUrl: "https://new-db",
    wrappedGroupKey: null,
  });
  expect(create).toHaveBeenCalledOnce();
  expect(mint.mock.calls[0][0].databaseName).toBe("new-db");
  expect((await accounts.findDeviceById(db, mac.id))?.status).toBe("revoked");
  expect((await accounts.findDeviceById(db, phone.id))?.status).toBe("revoked");
  expect(await accounts.findDeviceByPublicKey(db, account.id, "new-key")).toMatchObject({
    status: "approved",
    replacesDeviceId: null,
  });
  expect(await accounts.findAccountByGoogleSub(db, "google-sub-1")).toMatchObject({
    dataDbName: "new-db",
    dataDbUrl: "https://new-db",
  });
});

it("reset from an ordinary pending device is 403", async () => {
  const { db, account, mac } = await seeded();
  await accounts.enrollDevice(db, account.id, "new-key", "pending", 3, "mac");
  const del = vi.spyOn(turso, "deleteAccountDatabase").mockResolvedValue(undefined);
  const res = await handleResetAccount(
    post("/v1/account/reset", { idToken: "t", devicePublicKey: "new-key", confirm: true }),
    makeEnv(),
    db,
  );
  expect(res.status).toBe(403);
  expect(del).not.toHaveBeenCalled();
  expect((await accounts.findDeviceById(db, mac.id))?.status).toBe("approved");
});

// ---- fix round 1 ----
const DAY = 24 * 60 * 60 * 1000;

it("reset is refused while another approved device was seen in the last 7 days", async () => {
  const { db, mac } = await seeded();
  await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id)), makeEnv(), db);
  await db.execute("UPDATE device SET last_seen_unix_ms = ? WHERE id = ?", [Date.now() - 6 * DAY, mac.id]);
  const { res, create } = await resetFrom(db);
  expect(res.status).toBe(403);
  expect(await res.json()).toEqual({
    error: "other_device_active",
    devices: [{ id: mac.id, name: "Office Mac", platform: "mac" }],
  });
  expect(create).not.toHaveBeenCalled();
  expect((await accounts.findDeviceById(db, mac.id))?.status).toBe("approved");

  await db.execute("UPDATE device SET last_seen_unix_ms = ? WHERE id = ?", [Date.now() - 8 * DAY, mac.id]);
  expect((await resetFrom(db)).res.status).toBe(200);
});

it("reset retires the old database instead of deleting it", async () => {
  const { db, account, mac } = await seeded();
  await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id)), makeEnv(), db);
  const { res, del } = await resetFrom(db);
  expect(res.status).toBe(200);
  expect(del).not.toHaveBeenCalled();
  expect(db.rawRetired()).toEqual([{ name: "old-db", account_id: account.id, retired_at_unix_ms: expect.any(Number) }]);
});

it("reset records a pending notice", async () => {
  const { db, account, mac } = await seeded();
  await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id)), makeEnv(), db);
  expect(db.rawAccount(account.id)?.resetNoticePendingUnixMs ?? null).toBeNull();
  await resetFrom(db);
  expect(db.rawAccount(account.id)?.resetNoticePendingUnixMs).toBeGreaterThan(0);
});

it("reset from a phone swap is 403", async () => {
  const { db, account } = await seeded();
  const phone = await accounts.enrollDevice(db, account.id, "phone-key", "approved", 3, "ios");
  mockAud(IOS);
  const ios = { GOOGLE_IOS_CLIENT_ID: IOS };
  const swap = await handleSwapDevice(post("/v1/devices/swap", swapBody(phone.id, { platform: undefined })), makeEnv(ios), db);
  expect(await swap.json()).toEqual({ status: "pending" });
  const { res, create } = await resetFrom(db, ios);
  expect(res.status).toBe(403);
  expect(await res.json()).toEqual({ error: "reset_requires_computer" });
  expect(create).not.toHaveBeenCalled();
  expect((await accounts.findDeviceById(db, phone.id))?.status).toBe("approved");
});

it("a newer swap supersedes an abandoned swap of the same device", async () => {
  const { db, account, mac } = await seeded();
  vi.spyOn(turso, "rotateAllTokens").mockResolvedValue(undefined);
  await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id, { devicePublicKey: "s1" })), hostedEnv(), db);
  const s1 = (await accounts.findDeviceByPublicKey(db, account.id, "s1"))!;

  // A new computer is told about the abandoned swap, and swaps that out.
  const refused = await handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "c" }), hostedEnv(), db);
  expect(((await refused.json()) as { devices: { id: string }[] }).devices.map((d) => d.id)).toEqual([s1.id]);
  const viaS1 = await handleSwapDevice(post("/v1/devices/swap", swapBody(s1.id, { devicePublicKey: "c" })), hostedEnv(), db);
  expect(await viaS1.json()).toEqual({ status: "pending" });
  expect(await accounts.findDeviceById(db, s1.id)).toBeNull();
  const c = (await accounts.findDeviceByPublicKey(db, account.id, "c"))!;
  expect(c.replacesDeviceId).toBe(mac.id);

  // Targeting the old computer directly also replaces the older swap.
  const direct = await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id, { devicePublicKey: "c2" })), hostedEnv(), db);
  expect(await direct.json()).toEqual({ status: "pending" });
  expect(await accounts.findDeviceById(db, c.id)).toBeNull();
  const c2 = (await accounts.findDeviceByPublicKey(db, account.id, "c2"))!;
  await handleApproveDevice(post("/x", { idToken: "t", devicePublicKey: "mac-key", wrappedGroupKey: "w" }), hostedEnv(), c2.id, db);
  const active = (await accounts.listDevicesForAccount(db, account.id)).filter((d) => d.status !== "revoked");
  expect(active.map((d) => d.id)).toEqual([c2.id]);
});

it("approving a non-swap pending device over the limit is refused", async () => {
  const { db, account, mac } = await seeded();
  const legacy = await accounts.enrollDevice(db, account.id, "legacy-key", "pending", 3, "mac");
  const res = await handleApproveDevice(
    post("/x", { idToken: "t", devicePublicKey: "mac-key", wrappedGroupKey: "w" }),
    hostedEnv(),
    legacy.id,
    db,
  );
  expect(res.status).toBe(409);
  expect(await res.json()).toEqual({
    error: "device_limit",
    kind: "computer",
    devices: [{ id: mac.id, name: "Office Mac", platform: "mac" }],
  });
  expect((await accounts.findDeviceById(db, legacy.id))?.status).toBe("pending");
});

it("a failed limit check removes the new row", async () => {
  const { db, account } = await seeded();
  vi.spyOn(accounts, "listDevicesForAccount").mockRejectedValueOnce(new Error("turso blip"));
  const res = await handleGoogleAuth(authBody(), hostedEnv(), db);
  expect(res.status).toBe(502);
  expect(await accounts.countDevicesForAccount(db, account.id)).toBe(1);
});

it("a second concurrent reset from the same device does not create a second database", async () => {
  const { db, mac } = await seeded();
  await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id)), makeEnv(), db);
  const { create, call } = resetSpies(db);
  const [a, b] = await Promise.all([call(), call()]);
  // The loser is refused either at auth (403, already approved) or at the claim (409), depending on timing.
  expect([a.status, b.status].filter((st) => st === 200)).toHaveLength(1);
  expect(create).toHaveBeenCalledOnce();
  expect(db.rawRetired()).toHaveLength(1);

  // The claim itself is single-use.
  const other = await accounts.enrollDevice(db, mac.accountId, "k2", "pending", 9, "mac");
  await accounts.setReplacesDevice(db, other.id, mac.id);
  expect(await accounts.claimSwapForReset(db, other.id)).toBe(true);
  expect(await accounts.claimSwapForReset(db, other.id)).toBe(false);
});

it("a failed reset leaves the device pending for a retry", async () => {
  const { db, account, mac } = await seeded();
  await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id)), makeEnv(), db);
  vi.spyOn(turso, "createAccountDatabase").mockRejectedValueOnce(new Error("turso down"));
  const res = await handleResetAccount(
    post("/v1/account/reset", { idToken: "t", devicePublicKey: "new-key", confirm: true }),
    makeEnv(),
    db,
  );
  expect(res.status).toBe(502);
  expect(await accounts.findDeviceByPublicKey(db, account.id, "new-key")).toMatchObject({
    status: "pending",
    replacesDeviceId: mac.id,
  });
  expect((await resetFrom(db)).res.status).toBe(200);
});

// ---- fix round 2 ----
async function swappedAndClaimed() {
  const s = await seeded();
  await handleSwapDevice(post("/v1/devices/swap", swapBody(s.mac.id)), makeEnv(), s.db);
  const me = (await accounts.findDeviceByPublicKey(s.db, s.account.id, "new-key"))!;
  return { ...s, me };
}

it("auth during a reset returns pending and mints nothing", async () => {
  const { db, me } = await swappedAndClaimed();
  expect(await accounts.claimSwapForReset(db, me.id)).toBe(true);
  const mint = vi.spyOn(turso, "mintDeviceToken");
  const res = await handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "new-key" }), makeEnv(), db);
  expect(await res.json()).toEqual({ status: "pending" });
  expect(mint).not.toHaveBeenCalled();
  const list = await handleListDevices(postDevices({ idToken: "t", devicePublicKey: "new-key" }), makeEnv(), db);
  expect(list.status).toBe(403);
});

it("reset rotates the old database's tokens before retiring it", async () => {
  const { db, account } = await swappedAndClaimed();
  const retiredAtRotate: number[] = [];
  const rotate = vi.spyOn(turso, "rotateAllTokens").mockImplementation(async () => {
    retiredAtRotate.push(db.rawRetired().length);
  });
  const { res } = await resetFrom(db);
  expect(res.status).toBe(200);
  expect(rotate).toHaveBeenCalledOnce();
  expect(rotate.mock.calls[0][0].databaseName).toBe("old-db");
  expect(retiredAtRotate).toEqual([0]);
  expect(db.rawRetired().map((r) => r.name)).toEqual(["old-db"]);
  expect(db.rawAccount(account.id)?.dataDbName).toBe("new-db");
});

it("a device revoked during a failing reset stays revoked", async () => {
  const { db, me } = await swappedAndClaimed();
  vi.spyOn(turso, "rotateAllTokens").mockResolvedValue(undefined);
  vi.spyOn(turso, "createAccountDatabase").mockImplementation(async () => {
    await accounts.revokeDevice(db, me.id, Date.now());
    throw new Error("turso down");
  });
  const res = await handleResetAccount(
    post("/v1/account/reset", { idToken: "t", devicePublicKey: "new-key", confirm: true }),
    makeEnv(),
    db,
  );
  expect(res.status).toBe(502);
  expect((await accounts.findDeviceById(db, me.id))?.status).toBe("revoked");
});

it("a reset that fails after retiring the old database un-retires it", async () => {
  const { db, account } = await swappedAndClaimed();
  vi.spyOn(turso, "rotateAllTokens").mockResolvedValue(undefined);
  vi.spyOn(turso, "createAccountDatabase").mockResolvedValue({ name: "new-db", url: "https://new-db" });
  const realExecute = db.execute.bind(db);
  vi.spyOn(db, "execute").mockImplementation(async (sql, params) => {
    if (sql.startsWith("UPDATE account SET data_db_name")) throw new Error("registry blip");
    return realExecute(sql, params);
  });
  const res = await handleResetAccount(
    post("/v1/account/reset", { idToken: "t", devicePublicKey: "new-key", confirm: true }),
    makeEnv(),
    db,
  );
  expect(res.status).toBe(502);
  // The live database is not queued for deletion; the unused new one is.
  expect(db.rawRetired().map((r) => r.name)).toEqual(["new-db"]);
  expect(db.rawAccount(account.id)?.dataDbName).toBe("old-db");
  expect((await accounts.findDeviceByPublicKey(db, account.id, "new-key"))?.status).toBe("pending");
});

it("approve refuses a device that is mid-reset", async () => {
  const { db, me } = await swappedAndClaimed();
  await accounts.claimSwapForReset(db, me.id);
  const res = await handleApproveDevice(
    post("/x", { idToken: "t", devicePublicKey: "mac-key", wrappedGroupKey: "w" }),
    makeEnv(),
    me.id,
    db,
  );
  expect(res.status).toBe(400);
  expect((await accounts.findDeviceById(db, me.id))?.status).toBe("resetting");
});

it("swap of an orphaned pending swap does not 404", async () => {
  const { db, account, mac } = await seeded();
  await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id, { devicePublicKey: "p" })), makeEnv(), db);
  const p = (await accounts.findDeviceByPublicKey(db, account.id, "p"))!;
  await accounts.revokeDevice(db, mac.id, 5);
  const res = await handleSwapDevice(post("/v1/devices/swap", swapBody(p.id, { devicePublicKey: "c" })), makeEnv(), db);
  expect(res.status).toBe(200);
  // The caller swaps out the orphaned swap itself, which stays until approval or reset.
  expect(await res.json()).toEqual({ status: "pending" });
  expect((await accounts.findDeviceById(db, p.id))?.status).toBe("pending");
  expect(await accounts.findDeviceByPublicKey(db, account.id, "c")).toMatchObject({ status: "pending", replacesDeviceId: p.id });
});

// ---- fix round 3 ----
it("orphan swap never auto-approves", async () => {
  const { db, account, mac } = await seeded();
  await handleSwapDevice(post("/v1/devices/swap", swapBody(mac.id, { devicePublicKey: "p" })), makeEnv(), db);
  const p = (await accounts.findDeviceByPublicKey(db, account.id, "p"))!;
  await accounts.deleteDevice(db, mac.id); // the replaced device is gone entirely
  const mint = vi.spyOn(turso, "mintDeviceToken");
  await handleSwapDevice(post("/v1/devices/swap", swapBody(p.id, { devicePublicKey: "c" })), makeEnv(), db);
  const auth = await handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "c" }), makeEnv(), db);
  expect(await auth.json()).toEqual({ status: "pending" });
  expect(mint).not.toHaveBeenCalled();
  const active = (await accounts.listDevicesForAccount(db, account.id)).filter((d) => d.status === "approved");
  expect(active).toEqual([]);
});

it("swap of a resetting device is reset_in_progress", async () => {
  const { db, me } = await swappedAndClaimed();
  expect(await accounts.claimSwapForReset(db, me.id)).toBe(true);
  const res = await handleSwapDevice(post("/v1/devices/swap", swapBody(me.id, { devicePublicKey: "c" })), makeEnv(), db);
  expect(res.status).toBe(409);
  expect(await res.json()).toEqual({ error: "reset_in_progress" });
});

it("un-retire removes only its own retired row", async () => {
  const db = new FakeSqlExecutor();
  await accounts.retireDatabase(db, "acct", "db-x", 100);
  await accounts.retireDatabase(db, "acct", "db-x", 200);
  await accounts.unretireDatabase(db, "acct", "db-x", 200);
  expect(db.rawRetired()).toEqual([{ name: "db-x", account_id: "acct", retired_at_unix_ms: 100 }]);
});

it("approve during a reset does not overwrite resetting", async () => {
  const { db, me } = await swappedAndClaimed();
  // The reset claims the device after approve has read it as pending.
  vi.spyOn(handoff, "storeWrappedKeyForHandoff").mockImplementation(async () => {
    await accounts.claimSwapForReset(db, me.id);
  });
  const res = await handleApproveDevice(
    post("/x", { idToken: "t", devicePublicKey: "mac-key", wrappedGroupKey: "w" }),
    makeEnv(),
    me.id,
    db,
  );
  expect(res.status).toBe(400);
  expect(await res.json()).toEqual({ error: "device is resetting" });
  expect((await accounts.findDeviceById(db, me.id))?.status).toBe("resetting");
});

it("re-approving an approved device still works (Re-send Key)", async () => {
  const { db, mac } = await seeded();
  const res = await handleApproveDevice(
    post("/x", { idToken: "t", devicePublicKey: "mac-key", wrappedGroupKey: "w" }),
    makeEnv(),
    mac.id,
    db,
  );
  expect(res.status).toBe(200);
});

// ---- account deletion, rate limits ----
it("delete removes the database and every row for the account", async () => {
  const { db, account } = await seeded();
  const phone = await accounts.enrollDevice(db, account.id, "phone-key", "approved", 3, "ios");
  await accounts.addToWaitlist(db, account.id, ["backups"], "mac", 4);
  await accounts.retireDatabase(db, account.id, "older-db", 5);
  const other = await accounts.getOrCreateAccount(db, "x@example.com", "google-sub-2", 1, "their-db", "u2");
  const theirs = await accounts.enrollDevice(db, other.id, "their-key", "approved", 2, "mac");
  const del = vi.spyOn(turso, "deleteAccountDatabase").mockResolvedValue(undefined);

  const direct = await handleDeleteAccount(
    post("/v1/account/delete", { idToken: "t", devicePublicKey: "mac-key", confirm: true }),
    makeEnv(),
    db,
  );
  expect(await direct.json()).toEqual({ status: "deleted" });
  expect(del.mock.calls.map((c) => c[1]).sort()).toEqual(["old-db", "older-db"]);
  expect(db.rawAccount(account.id)).toBeUndefined();
  expect(db.rawDevice(phone.id)).toBeUndefined();
  expect(db.rawWaitlist()).toEqual([]);
  expect(db.rawRetired()).toEqual([]);
  expect(db.rawAccount(other.id)).toBeDefined();
  expect(db.rawDevice(theirs.id)).toBeDefined();
});

it("delete without confirm is refused", async () => {
  const { db, account } = await seeded();
  const del = vi.spyOn(turso, "deleteAccountDatabase").mockResolvedValue(undefined);
  const res = await handleDeleteAccount(post("/v1/account/delete", { idToken: "t", devicePublicKey: "mac-key" }), makeEnv(), db);
  expect(res.status).toBe(400);
  expect(await res.json()).toEqual({ error: "reset_confirm_required" });
  expect(del).not.toHaveBeenCalled();
  expect(db.rawAccount(account.id)).toBeDefined();
  // Routed by the Worker (refused before any database work).
  const routed = await worker.fetch(post("/v1/account/delete", { idToken: "t", devicePublicKey: "k" }), makeEnv());
  expect(await routed.json()).toEqual({ error: "reset_confirm_required" });
});

it("delete from a pending device is refused", async () => {
  const { db, account } = await seeded();
  await accounts.enrollDevice(db, account.id, "new-key", "pending", 3, "mac");
  const del = vi.spyOn(turso, "deleteAccountDatabase").mockResolvedValue(undefined);
  const res = await handleDeleteAccount(
    post("/v1/account/delete", { idToken: "t", devicePublicKey: "new-key", confirm: true }),
    makeEnv(),
    db,
  );
  expect(res.status).toBe(403);
  expect(del).not.toHaveBeenCalled();
});

it("a fourth new account from one IP in a day is rate limited", async () => {
  const create = vi.spyOn(turso, "createAccountDatabase").mockResolvedValue({ name: "n", url: "u" });
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("jwt");
  const verify = vi.spyOn(googleAuth, "verifyGoogleIdToken");
  const ip = `198.51.100.${Math.floor(Math.random() * 250)}-${crypto.randomUUID()}`;
  const db = new FakeSqlExecutor();
  const statuses = [];
  for (let i = 0; i < 4; i++) {
    verify.mockResolvedValueOnce({ email: `u${i}@example.com`, googleSub: `ip-sub-${i}-${ip}`, audience: "test-client-id" });
    const req = new Request("https://example.com/v1/auth/google", {
      method: "POST",
      headers: { "CF-Connecting-IP": ip },
      body: JSON.stringify({ idToken: "t", devicePublicKey: "k" }),
    });
    const res = await handleGoogleAuth(req, makeEnv({ OPEN_SIGN_IN: "true" }), db);
    statuses.push(res.status);
    if (i === 3) expect(await res.json()).toEqual({ error: "rate_limited" });
  }
  expect(statuses).toEqual([200, 200, 200, 429]);
  expect(create).toHaveBeenCalledTimes(3);
});

it("a pending device polling every 3 s is never rate limited", async () => {
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("jwt");
  const sub = `poll-${crypto.randomUUID()}`;
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({ email: "owner@example.com", googleSub: sub, audience: "test-client-id" });
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", sub, 1, "n", "u");
  await accounts.enrollDevice(db, account.id, "mac", "approved", 2, "mac");
  await accounts.enrollDevice(db, account.id, "k1", "pending", 3, "ios");
  const statuses = new Set<number>();
  for (let i = 0; i < 40; i++) statuses.add((await handleGoogleAuth(authBody(), makeEnv(), db)).status);
  expect([...statuses]).toEqual([200]);
});

it("new enrolment attempts are limited to 30 per 10 minutes", async () => {
  const sub = `burst-${crypto.randomUUID()}`;
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({ email: "owner@example.com", googleSub: sub, audience: "test-client-id" });
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", sub, 1, "n", "u");
  await accounts.enrollDevice(db, account.id, "mac", "approved", 2, "mac");
  const env = makeEnv({ UNLIMITED_EMAILS: "owner@example.com" });
  const statuses = [];
  for (let i = 0; i < 31; i++) {
    statuses.push((await handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: `new-${i}` }), env, db)).status);
  }
  expect(statuses.slice(0, 30).every((s) => s === 200)).toBe(true);
  expect(statuses[30]).toBe(429);
  expect(await accounts.findDeviceByPublicKey(db, account.id, "new-30")).toBeNull();
});

function createFrom(ip: string, sub: string, env: Partial<Env> = {}, db = new FakeSqlExecutor()) {
  vi.mocked(googleAuth.verifyGoogleIdToken).mockResolvedValueOnce({ email: `${sub}@example.com`, googleSub: sub, audience: "test-client-id" });
  return handleGoogleAuth(
    new Request("https://example.com/v1/auth/google", {
      method: "POST",
      headers: { "CF-Connecting-IP": ip },
      body: JSON.stringify({ idToken: "t", devicePublicKey: "k" }),
    }),
    makeEnv({ OPEN_SIGN_IN: "true", ...env }),
    db,
  );
}

it("account creation fails closed when the creation reservation errors", async () => {
  const create = vi.spyOn(turso, "createAccountDatabase").mockResolvedValue({ name: "n", url: "u" });
  vi.spyOn(googleAuth, "verifyGoogleIdToken");
  const db = new FakeSqlExecutor();
  const realExecute = db.execute.bind(db);
  vi.spyOn(db, "execute").mockImplementation(async (sql, params) => {
    if (sql.startsWith("INSERT INTO account_creation")) throw new Error("registry blip");
    return realExecute(sql, params);
  });
  const res = await createFrom("192.0.2.1", "kv-sub", {}, db);
  expect(res.status).toBe(429);
  expect(await res.json()).toEqual({ error: "rate_limited" });
  expect(create).not.toHaveBeenCalled();
});

it("two IPv6 addresses in one /64 share a bucket", async () => {
  vi.spyOn(turso, "createAccountDatabase").mockResolvedValue({ name: "n", url: "u" });
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("jwt");
  vi.spyOn(googleAuth, "verifyGoogleIdToken");
  const hex = () => Math.floor(Math.random() * 0xffff).toString(16);
  const prefix = `2001:db8:${hex()}:${hex()}`;
  const db = new FakeSqlExecutor();
  const statuses = [];
  for (const [i, ip] of [`${prefix}::1`, `${prefix}:0:0:0:2`, `${prefix}::abcd`, `${prefix}:ffff::9`].entries()) {
    statuses.push((await createFrom(ip, `v6-${prefix}-${i}`, {}, db)).status);
  }
  expect(statuses).toEqual([200, 200, 200, 429]);
  // A different /64 has its own bucket.
  expect((await createFrom(`2001:db8:${hex()}:${hex()}::1`, `v6-${prefix}-other`, {}, db)).status).toBe(200);
});

// ---- deletion claim ----
it("auth refuses a claimed account", async () => {
  vi.mocked(accounts.touchLastSeen).mockRestore();
  const { db, account } = await seeded();
  const mint = vi.spyOn(turso, "mintDeviceToken");
  expect(await accounts.claimForDeletion(db, account.id, 5)).toBe(true);
  const before = db.rawAccount(account.id)?.lastSeenUnixMs ?? null;
  for (const res of [
    await handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "mac-key" }), makeEnv(), db),
    await handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "brand-new" }), makeEnv({ UNLIMITED_EMAILS: "owner@example.com" }), db),
    await handleListDevices(postDevices({ idToken: "t", devicePublicKey: "mac-key" }), makeEnv(), db),
    await handleWaitlist(post("/v1/waitlist", { idToken: "t", devicePublicKey: "mac-key", wants: ["backups"] }), makeEnv(), db),
  ]) {
    expect(res.status).toBe(503);
    expect(await res.json()).toEqual({ error: "account_deleting" });
  }
  expect(mint).not.toHaveBeenCalled();
  expect(await accounts.findDeviceByPublicKey(db, account.id, "brand-new")).toBeNull();
  expect(db.rawAccount(account.id)?.lastSeenUnixMs ?? null).toBe(before);
  expect(db.rawWaitlist()).toEqual([]);
});

it("touchLastSeen refuses an account claimed after it was read", async () => {
  vi.mocked(accounts.touchLastSeen).mockRestore();
  const { db, account, mac } = await seeded();
  const mint = vi.spyOn(turso, "mintDeviceToken");
  db.after("SELECT id, account_id, public_key", () => accounts.claimForDeletion(db, account.id, 5).then(() => {}));
  const res = await handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "mac-key" }), makeEnv(), db);
  expect(await res.json()).toEqual({ error: "account_deleting" });
  expect(mint).not.toHaveBeenCalled();
  expect(db.rawDevice(mac.id)?.last_seen_unix_ms).toBeNull();
});

it("sign-in racing a deletion does not recreate the account", async () => {
  const { db, account } = await seeded();
  const create = vi.spyOn(turso, "createAccountDatabase");
  const mint = vi.spyOn(turso, "mintDeviceToken");
  // The deletion finishes right after the sign-in first reads the account.
  db.after("SELECT id, email, google_sub", () => accounts.deleteAccountRows(db, account.id));
  const res = await handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "mac-key" }), makeEnv(), db);
  expect(res.status).toBe(503);
  expect(await res.json()).toEqual({ error: "account_deleting" });
  expect(await accounts.findAccountByGoogleSub(db, "google-sub-1")).toBeNull();
  expect(create).not.toHaveBeenCalled();
  expect(mint).not.toHaveBeenCalled();
});

it("delete of an account already being deleted is refused", async () => {
  const { db, account } = await seeded();
  const del = vi.spyOn(turso, "deleteAccountDatabase").mockResolvedValue(undefined);
  db.after("SELECT id, account_id, public_key", () => accounts.claimForDeletion(db, account.id, 5).then(() => {}));
  const res = await handleDeleteAccount(
    post("/v1/account/delete", { idToken: "t", devicePublicKey: "mac-key", confirm: true }),
    makeEnv(),
    db,
  );
  expect(res.status).toBe(503);
  expect(del).not.toHaveBeenCalled();
});

it("waitlist is limited to 5 per Google account per day", async () => {
  const sub = `wl-${crypto.randomUUID()}`;
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({ email: "owner@example.com", googleSub: sub, audience: "test-client-id" });
  const db = new FakeSqlExecutor();
  await accounts.getOrCreateAccount(db, "owner@example.com", sub, 1, "n", "u");
  const statuses = [];
  for (let i = 0; i < 6; i++) {
    statuses.push((await handleWaitlist(post("/v1/waitlist", { idToken: "t", devicePublicKey: "k", wants: ["backups"] }), makeEnv(), db)).status);
  }
  expect(statuses).toEqual([200, 200, 200, 200, 200, 429]);
  expect(db.rawWaitlist()).toHaveLength(5);
});

// ---- fix round 2: deletion vs reset, waitlist ----
const deleteReq = () => post("/v1/account/delete", { idToken: "t", devicePublicKey: "mac-key", confirm: true });

it("delete is refused while a reset is in progress", async () => {
  const { db, account, me } = await swappedAndClaimed();
  expect(await accounts.claimSwapForReset(db, me.id)).toBe(true);
  const del = vi.spyOn(turso, "deleteAccountDatabase").mockResolvedValue(undefined);
  const res = await handleDeleteAccount(deleteReq(), makeEnv(), db);
  expect(res.status).toBe(503);
  expect(await res.json()).toEqual({ error: "account_deleting" });
  expect(del).not.toHaveBeenCalled();
  expect(db.rawAccount(account.id)?.deletingSinceUnixMs ?? null).toBeNull();
});

it("a reset cannot claim an account claimed for deletion", async () => {
  const { db, account, me } = await swappedAndClaimed();
  expect(await accounts.claimForDeletion(db, account.id, 5)).toBe(true);
  expect(await accounts.claimSwapForReset(db, me.id)).toBe(false);
});

it("a reset aborts when the account was claimed for deletion", async () => {
  const { db, account, me } = await swappedAndClaimed();
  // A claim that slips in after the reset's own claim (the claims exclude
  // each other, so this forces it), right before the database switch.
  db.after("INSERT INTO retired_database", async () => {
    db.rawAccount(account.id)!.deletingSinceUnixMs = 5;
  });
  const { res, del } = await resetFrom(db);
  expect(res.status).toBe(503);
  expect(await res.json()).toEqual({ error: "account_deleting" });
  expect(db.rawAccount(account.id)?.dataDbName).toBe("old-db");
  expect(db.rawDevice(me.id)?.status).toBe("pending");
  expect(db.rawRetired().map((r) => r.name)).toEqual(["new-db"]);

  // The claimed deletion is finished by the cron and takes both databases.
  await runInactivity(db, makeEnv(), Date.now() + 24 * 60 * 60 * 1000, (async () => new Response("{}")) as unknown as typeof fetch);
  expect(del.mock.calls.map((c) => c[1]).sort()).toEqual(["new-db", "old-db"]);
  expect(db.rawAccount(account.id)).toBeUndefined();
  expect(db.rawRetired()).toEqual([]);
});

it("deleteAccount deletes the database current at claim time", async () => {
  const { db, account } = await seeded();
  const snapshot = { ...account };
  expect(await accounts.claimForDeletion(db, account.id, 5)).toBe(true);
  // The database changed after the caller's snapshot (as a reset would).
  await accounts.retireDatabase(db, account.id, "old-db", 6);
  db.rawAccount(account.id)!.dataDbName = "switched-db";
  const del = vi.spyOn(turso, "deleteAccountDatabase").mockResolvedValue(undefined);
  await deleteAccount(db, makeEnv(), snapshot);
  expect(del.mock.calls.map((c) => c[1]).sort()).toEqual(["old-db", "switched-db"]);
  expect(db.rawAccount(account.id)).toBeUndefined();
});

it("a waitlist entry racing a deletion claim is not stored", async () => {
  const { db, account } = await seeded();
  db.after("SELECT id, account_id, public_key", async () => {
    await accounts.claimForDeletion(db, account.id, 5);
  });
  const res = await handleWaitlist(post("/v1/waitlist", { idToken: "t", devicePublicKey: "new-key", wants: ["backups"] }), makeEnv(), db);
  expect(res.status).toBe(503);
  expect(db.rawWaitlist()).toEqual([]);
});

// ---- final review I1: orphaned databases ----

/** Holds every createAccountDatabase call until `n` have arrived, so `n`
 * first sign-ins are all past their "no account yet" read before any inserts. */
function createBarrier(n: number) {
  let waiting: (() => void)[] = [];
  return vi.spyOn(turso, "createAccountDatabase").mockImplementation(async (_config, name) => {
    await new Promise<void>((resolve) => {
      waiting.push(resolve);
      if (waiting.length === n) {
        waiting.forEach((r) => r());
        waiting = [];
      }
    });
    return { name, url: `https://${name}` };
  });
}

it("two simultaneous first sign-ins leave exactly one live database and delete the other", async () => {
  const create = createBarrier(2);
  const del = vi.spyOn(turso, "deleteAccountDatabase").mockResolvedValue(undefined);
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("jwt");
  const sub = `race-${crypto.randomUUID()}`;
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({ email: "owner@example.com", googleSub: sub, audience: "test-client-id" });
  const db = new FakeSqlExecutor();
  const env = makeEnv();
  await Promise.all([
    handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "k" }), env, db),
    handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "k" }), env, db),
  ]);
  const created = create.mock.calls.map((c) => c[1]);
  expect(created).toHaveLength(2);
  const live = (await accounts.findAccountByGoogleSub(db, sub))!.dataDbName;
  expect(created).toContain(live);
  expect(del.mock.calls.map((c) => c[1])).toEqual(created.filter((n) => n !== live));
});

it("a race loser whose delete fails is left on the retired list for the cron", async () => {
  const create = createBarrier(2);
  vi.spyOn(turso, "deleteAccountDatabase").mockRejectedValue(new Error("turso down"));
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("jwt");
  const sub = `race-${crypto.randomUUID()}`;
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({ email: "owner@example.com", googleSub: sub, audience: "test-client-id" });
  const db = new FakeSqlExecutor();
  const env = makeEnv();
  await Promise.all([
    handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "k" }), env, db),
    handleGoogleAuth(postAuth({ idToken: "t", devicePublicKey: "k" }), env, db),
  ]);
  const live = (await accounts.findAccountByGoogleSub(db, sub))!.dataDbName;
  const loser = create.mock.calls.map((c) => c[1]).filter((n) => n !== live);
  expect(db.rawRetired().map((r) => r.name)).toEqual(loser);
});

it("a burst of first sign-ins from one IP creates no more databases than the limit", async () => {
  const create = vi.spyOn(turso, "createAccountDatabase").mockImplementation(async (_c, name) => ({ name, url: "u" }));
  vi.spyOn(turso, "mintDeviceToken").mockResolvedValue("jwt");
  const verify = vi.spyOn(googleAuth, "verifyGoogleIdToken");
  const ip = `203.0.113.${Math.floor(Math.random() * 250)}`;
  const db = new FakeSqlExecutor();
  const env = makeEnv({ OPEN_SIGN_IN: "true" });
  const requests = Array.from({ length: 6 }, (_, i) => {
    verify.mockResolvedValueOnce({ email: `b${i}@example.com`, googleSub: `burst-${ip}-${i}-${crypto.randomUUID()}`, audience: "test-client-id" });
    return handleGoogleAuth(
      new Request("https://example.com/v1/auth/google", {
        method: "POST",
        headers: { "CF-Connecting-IP": ip },
        body: JSON.stringify({ idToken: "t", devicePublicKey: "k" }),
      }),
      env,
      db,
    );
  });
  const statuses = (await Promise.all(requests)).map((r) => r.status).sort();
  expect(statuses).toEqual([200, 200, 200, 429, 429, 429]);
  expect(create).toHaveBeenCalledTimes(3);
});

// ---- final review I2: the free limit is hosted-only ----

it("a self-hosted relay (OPEN_SIGN_IN unset) has no device limit at enrolment", async () => {
  const { db, account } = await seeded();
  const res = await handleGoogleAuth(authBody({ platform: "windows" }), makeEnv(), db);
  expect(await res.json()).toEqual({ status: "pending" });
  expect(await accounts.countDevicesForAccount(db, account.id)).toBe(2);
});

it("a self-hosted relay (OPEN_SIGN_IN false) lets a pending second computer be approved", async () => {
  const { db, account } = await seeded();
  const legacy = await accounts.enrollDevice(db, account.id, "legacy-key", "pending", 3, "mac");
  const res = await handleApproveDevice(
    post("/x", { idToken: "t", devicePublicKey: "mac-key", wrappedGroupKey: "w" }),
    makeEnv({ OPEN_SIGN_IN: "false" }),
    legacy.id,
    db,
  );
  expect(res.status).toBe(200);
  expect((await accounts.findDeviceById(db, legacy.id))?.status).toBe("approved");
});

it("the hosted relay (OPEN_SIGN_IN true) enforces the free limit at enrolment and approval", async () => {
  const { db, account, mac } = await seeded();
  const env = makeEnv({ OPEN_SIGN_IN: "true" });
  const res = await handleGoogleAuth(authBody({ platform: "windows" }), env, db);
  expect(res.status).toBe(409);
  expect(await accounts.countDevicesForAccount(db, account.id)).toBe(1);
  const legacy = await accounts.enrollDevice(db, account.id, "legacy-key", "pending", 3, "mac");
  const approve = await handleApproveDevice(
    post("/x", { idToken: "t", devicePublicKey: mac.publicKey, wrappedGroupKey: "w" }),
    env,
    legacy.id,
    db,
  );
  expect(approve.status).toBe(409);
});

it("on the hosted relay an unlimited email can approve a second computer", async () => {
  const { db, account } = await seeded();
  const legacy = await accounts.enrollDevice(db, account.id, "legacy-key", "pending", 3, "mac");
  const res = await handleApproveDevice(
    post("/x", { idToken: "t", devicePublicKey: "mac-key", wrappedGroupKey: "w" }),
    hostedEnv({ UNLIMITED_EMAILS: "owner@example.com" }),
    legacy.id,
    db,
  );
  expect(res.status).toBe(200);
});
