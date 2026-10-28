import { it, expect, vi, beforeEach } from "vitest";
import * as googleAuth from "../src/googleAuth";
import * as turso from "../src/turso";
import * as accounts from "../src/accounts";
import worker, { handleGoogleAuth, handleListDevices, handleApproveDevice, handleRevokeDevice, handleSetDeviceName } from "../src/index";
import type { Env } from "../src/index";
import { FakeSqlExecutor } from "./testHelpers/fakeSqlExecutor";
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
    makeEnv(),
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
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF }),
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
  vi.spyOn(accounts, "approveDevice").mockResolvedValue(undefined);

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
    makeEnv({ ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF }),
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
