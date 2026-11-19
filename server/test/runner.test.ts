import { it, expect, vi, beforeEach } from "vitest";
import * as googleAuth from "../src/googleAuth";
import * as turso from "../src/turso";
import * as accounts from "../src/accounts";
import { handleRegisterRunner, handleRevokeDevice } from "../src/index";
import type { Env } from "../src/index";
import { handleRunnerAuth, hashRunnerToken, canRunCloudRoutines } from "../src/runner";
import { limitCheck } from "../src/limits";
import type { DeviceRecord } from "../src/accounts";
import { FakeSqlExecutor } from "./testHelpers/fakeSqlExecutor";
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
    RUNNER_EMAILS: "owner@example.com",
    ENROLLMENT_HANDOFF: env.ENROLLMENT_HANDOFF,
    ...overrides,
  };
}

const RUNNER_TOKEN = "runner-secret-token-0123456789abcdef";
const RUNNER_KEY = "runner-public-key";

function post(path: string, body: unknown): Request {
  return new Request(`https://example.com${path}`, { method: "POST", body: JSON.stringify(body) });
}

const mintFetcher = vi.fn(async () => Response.json({ jwt: "minted-turso-jwt" })) as unknown as typeof fetch;

async function setup() {
  const db = new FakeSqlExecutor();
  const account = await accounts.getOrCreateAccount(db, "owner@example.com", "google-sub-1", 1, "db-name", "libsql://db-name.example.turso.io");
  const approver = await accounts.enrollDevice(db, account.id, "approver-key", "approved", 1, "mac");
  vi.spyOn(googleAuth, "verifyGoogleIdToken").mockResolvedValue({
    email: "owner@example.com",
    googleSub: "google-sub-1",
    audience: "test-client-id",
  });
  return { db, account, approver };
}

async function register(db: FakeSqlExecutor, overrides: Partial<Env> = {}, body: Record<string, unknown> = {}) {
  return handleRegisterRunner(
    post("/v1/devices/runner", {
      idToken: "good-token",
      devicePublicKey: "approver-key",
      runnerPublicKey: RUNNER_KEY,
      name: "Whimbrel Cloud",
      runnerTokenHash: await hashRunnerToken(RUNNER_TOKEN),
      ...body,
    }),
    makeEnv(overrides),
    db,
  );
}

function runnerAuth(db: FakeSqlExecutor, body: Record<string, unknown>, overrides: Partial<Env> = {}) {
  return handleRunnerAuth(post("/v1/runner/auth", body), makeEnv(overrides), db, mintFetcher);
}

beforeEach(() => {
  vi.restoreAllMocks();
});

it("runner auth returns a turso token for the right credential", async () => {
  const { db } = await setup();
  expect((await register(db)).status).toBe(200);
  const res = await runnerAuth(db, { devicePublicKey: RUNNER_KEY, runnerToken: RUNNER_TOKEN });
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ tursoToken: "minted-turso-jwt", tursoDatabaseUrl: "libsql://db-name.example.turso.io" });
});

it("wrong token is 401", async () => {
  const { db } = await setup();
  await register(db);
  const res = await runnerAuth(db, { devicePublicKey: RUNNER_KEY, runnerToken: "not-the-token" });
  expect(res.status).toBe(401);
  const unknown = await runnerAuth(db, { devicePublicKey: "other-key", runnerToken: RUNNER_TOKEN });
  expect(unknown.status).toBe(401);
});

it("revoked runner device is 401 even with the right token", async () => {
  const { db } = await setup();
  await register(db);
  const runner = await accounts.findDeviceByPublicKey(db, (await accounts.findAccountByGoogleSub(db, "google-sub-1"))!.id, RUNNER_KEY);
  vi.spyOn(turso, "rotateAllTokens").mockResolvedValue(undefined);
  const revoked = await handleRevokeDevice(
    post(`/v1/devices/${runner!.id}/revoke`, { idToken: "good-token", devicePublicKey: "approver-key" }),
    makeEnv(),
    runner!.id,
    db,
  );
  expect(revoked.status).toBe(200);
  const res = await runnerAuth(db, { devicePublicKey: RUNNER_KEY, runnerToken: RUNNER_TOKEN });
  expect(res.status).toBe(401);
});

it("runner registration refuses an account not in RUNNER_EMAILS", async () => {
  const { db } = await setup();
  expect((await register(db, { RUNNER_EMAILS: "someone-else@example.com" })).status).toBe(403);
  expect((await register(db, { RUNNER_EMAILS: undefined })).status).toBe(403);
  const account = (await accounts.findAccountByGoogleSub(db, "google-sub-1"))!;
  expect(await accounts.findDeviceByPublicKey(db, account.id, RUNNER_KEY)).toBeNull();
  expect(canRunCloudRoutines(account, makeEnv(), Date.now())).toBe(true);
  expect(canRunCloudRoutines(account, makeEnv({ RUNNER_EMAILS: "" }), Date.now())).toBe(false);
});

it("runner registration needs an approved Google-authed device and a well-formed hash", async () => {
  const { db } = await setup();
  expect((await register(db, {}, { devicePublicKey: "unknown-key" })).status).toBe(403);
  expect((await register(db, {}, { runnerTokenHash: "abc" })).status).toBe(400);
  expect((await register(db, {}, { runnerPublicKey: "" })).status).toBe(400);
  expect((await register(db)).status).toBe(200);
  expect((await register(db)).status).toBe(409);
});

it("a cloud device does not count toward the free computer cap", () => {
  const cloud: DeviceRecord = {
    id: "c1", accountId: "a", publicKey: "k", name: "Whimbrel Cloud", platform: "cloud",
    status: "approved", enrolledAtUnixMs: 1, revokedAtUnixMs: null, replacesDeviceId: null,
  };
  expect(limitCheck([cloud], "mac", false)).toEqual({ ok: true });
  expect(limitCheck([cloud], "ios", false)).toEqual({ ok: true });
  expect(limitCheck([], "cloud", false)).toEqual({ ok: true });
  expect(limitCheck([{ ...cloud, id: "m", platform: "mac" }, cloud], "windows", false)).toMatchObject({ ok: false, holders: [{ id: "m" }] });
});

it("a cloud device is not counted when deciding the first device", async () => {
  const { db, account } = await setup();
  await register(db);
  expect(await accounts.countActiveDevicesForAccount(db, account.id)).toBe(1);
});

it("the credential hash, not the secret, is stored", async () => {
  const { db, account } = await setup();
  await register(db);
  const runner = (await accounts.findDeviceByPublicKey(db, account.id, RUNNER_KEY))!;
  expect(runner.platform).toBe("cloud");
  expect(runner.status).toBe("approved");
  expect(runner.name).toBe("Whimbrel Cloud");
  const raw = db.rawDevice(runner.id)!;
  expect(raw.runner_token_hash).toBe(await hashRunnerToken(RUNNER_TOKEN));
  expect(raw.runner_token_hash).toMatch(/^[0-9a-f]{64}$/);
  expect(JSON.stringify(raw)).not.toContain(RUNNER_TOKEN);
});

it("runner auth never returns a group key", async () => {
  const { db } = await setup();
  await register(db);
  const res = await runnerAuth(db, { devicePublicKey: RUNNER_KEY, runnerToken: RUNNER_TOKEN });
  const text = await res.text();
  expect(Object.keys(JSON.parse(text)).sort()).toEqual(["tursoDatabaseUrl", "tursoToken"]);
  expect(text.toLowerCase()).not.toContain("groupkey");
});

it("runner auth refuses a missing credential and a non-cloud device", async () => {
  const { db } = await setup();
  expect((await runnerAuth(db, { devicePublicKey: RUNNER_KEY })).status).toBe(400);
  // A normal computer has no runner hash, so even an empty-ish token cannot match.
  expect((await runnerAuth(db, { devicePublicKey: "approver-key", runnerToken: RUNNER_TOKEN })).status).toBe(401);
});

it("a running runner does not block account reset", async () => {
  const { db, account, approver } = await setup();
  await register(db);
  const now = Date.now();
  await runnerAuth(db, { devicePublicKey: RUNNER_KEY, runnerToken: RUNNER_TOKEN }).catch(() => undefined);
  const runner = (await accounts.findDeviceByPublicKey(db, account.id, RUNNER_KEY))!;
  await db.execute("UPDATE device SET last_seen_unix_ms = ? WHERE id = ?", [now, runner.id]);
  expect(await accounts.recentlySeenApprovedDevices(db, account.id, approver.id, now - 7 * 86400000)).toEqual([]);
});

it("runner auth is 401 when the only approved device is the runner, 200 once a real one exists", async () => {
  const { db, account, approver } = await setup();
  await register(db);
  await accounts.revokeDevice(db, approver.id, Date.now());
  const refused = await runnerAuth(db, { devicePublicKey: RUNNER_KEY, runnerToken: RUNNER_TOKEN });
  expect(refused.status).toBe(401);
  expect(await refused.json()).toEqual({ error: "unauthorized" });
  await accounts.enrollDevice(db, account.id, "new-mac", "approved", 5, "mac");
  expect((await runnerAuth(db, { devicePublicKey: RUNNER_KEY, runnerToken: RUNNER_TOKEN })).status).toBe(200);
});

function runnerAuthFrom(ip: string, db: FakeSqlExecutor, body: Record<string, unknown>) {
  const request = new Request("https://example.com/v1/runner/auth", {
    method: "POST",
    headers: { "CF-Connecting-IP": ip },
    body: JSON.stringify(body),
  });
  return handleRunnerAuth(request, makeEnv(), db, mintFetcher);
}

it("runner auth is rate limited per client IP: the 31st try in the window is 429", async () => {
  // Mid-window, so the tries never straddle a window boundary.
  vi.spyOn(Date, "now").mockReturnValue(1_800_000_300_000);
  const { db } = await setup();
  expect((await register(db)).status).toBe(200);
  const wrong = { devicePublicKey: RUNNER_KEY, runnerToken: "not-the-token" };
  for (let i = 0; i < 30; i++) {
    expect((await runnerAuthFrom("203.0.113.7", db, wrong)).status).toBe(401);
  }
  const limited = await runnerAuthFrom("203.0.113.7", db, { devicePublicKey: RUNNER_KEY, runnerToken: RUNNER_TOKEN });
  expect(limited.status).toBe(429);
  expect(await limited.json()).toEqual({ error: "rate_limited" });
  // Another address (another bucket) is not affected, and still signs in.
  const other = await runnerAuthFrom("198.51.100.9", db, { devicePublicKey: RUNNER_KEY, runnerToken: RUNNER_TOKEN });
  expect(other.status).toBe(200);
});

it("runner auth from one IPv6 /64 shares one bucket", async () => {
  // Mid-window, so the tries never straddle a window boundary.
  vi.spyOn(Date, "now").mockReturnValue(1_800_000_300_000);
  const { db } = await setup();
  expect((await register(db)).status).toBe(200);
  const wrong = { devicePublicKey: RUNNER_KEY, runnerToken: "not-the-token" };
  for (let i = 0; i < 30; i++) {
    expect((await runnerAuthFrom(`2001:db8:1:2::${i + 1}`, db, wrong)).status).toBe(401);
  }
  expect((await runnerAuthFrom("2001:db8:1:2::ffff", db, wrong)).status).toBe(429);
  expect((await runnerAuthFrom("2001:db8:1:3::1", db, wrong)).status).toBe(401);
});

it("a normal runner sign-in with the client IP is not limited", async () => {
  const { db } = await setup();
  expect((await register(db)).status).toBe(200);
  for (let i = 0; i < 3; i++) {
    const res = await runnerAuthFrom("192.0.2.44", db, { devicePublicKey: RUNNER_KEY, runnerToken: RUNNER_TOKEN });
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ tursoToken: "minted-turso-jwt", tursoDatabaseUrl: "libsql://db-name.example.turso.io" });
  }
});
