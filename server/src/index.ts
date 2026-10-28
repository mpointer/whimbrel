import type { KVNamespace } from "@cloudflare/workers-types";
import { connect } from "@tursodatabase/serverless";
import { isEmailAllowed } from "./allowlist";
import { allowedAudiences, googleJwks, verifyGoogleIdToken } from "./googleAuth";
import { mintDeviceToken, createAccountDatabase, rotateAllTokens } from "./turso";
import { storeWrappedKeyForHandoff, fetchAndConsumeWrappedKey } from "./handoff";
import {
  findAccountByGoogleSub,
  getOrCreateAccount,
  findDeviceByPublicKey,
  findDeviceById,
  countActiveDevicesForAccount,
  enrollDevice,
  listDevicesForAccount,
  approveDevice,
  revokeDevice,
  setDeviceName,
  type AccountRecord,
  type DeviceRecord,
  type SqlExecutor,
  type SqlRow,
} from "./accounts";

export interface Env {
  GOOGLE_CLIENT_ID: string;
  /** iOS app OAuth client ID; empty/unset = iOS tokens rejected. */
  GOOGLE_IOS_CLIENT_ID?: string;
  TURSO_DATABASE_URL: string;
  TURSO_DATABASE_AUTH_TOKEN: string;
  TURSO_PLATFORM_API_TOKEN: string;
  TURSO_ORG_SLUG: string;
  TURSO_DB_GROUP: string;
  ALLOWED_GOOGLE_EMAILS: string;
  ENROLLMENT_HANDOFF: KVNamespace;
}

/** Wraps the real Turso SQL connection behind the SqlExecutor interface
 * accounts.ts is written against. `TURSO_DATABASE_URL`/`TURSO_DATABASE_AUTH_TOKEN`
 * point at the control-plane database (account/device registry only —
 * never sync payload data). Not unit-tested directly — there is no way to
 * exercise a real Turso connection without a live database; this function
 * is exercised by an actual deploy, not this task's test suite. Note: this
 * function (and therefore `connect()`) is evaluated as part of each
 * exported handler's default `db` parameter, which runs BEFORE that
 * handler's own try block — a synchronous throw here (e.g. a missing
 * `TURSO_DATABASE_URL`) is caught by the top-level `fetch` handler's
 * try/catch instead, not by any individual route handler's. */
function tursoSqlExecutor(env: Env): SqlExecutor {
  const conn = connect({ url: env.TURSO_DATABASE_URL, authToken: env.TURSO_DATABASE_AUTH_TOKEN });
  return {
    async execute(sql: string, params: unknown[]): Promise<SqlRow[]> {
      const stmt = await conn.prepare(sql);
      if (stmt.reader) {
        return (await stmt.all(params)) as SqlRow[];
      }
      await stmt.run(params);
      return [];
    },
  };
}

const DEVICE_TOKEN_EXPIRATION = "1d";

/** `db` defaults to a real Turso connection built from `env`; tests pass a
 * fake `SqlExecutor` directly so the route's real business logic
 * (first-device auto-approval, revocation, pending status, per-account
 * database provisioning) can be exercised against real `accounts.ts`
 * functions instead of mocking them all away. */
export async function handleGoogleAuth(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
): Promise<Response> {
  let body: { idToken?: string; devicePublicKey?: string };
  try {
    body = (await request.json()) as { idToken?: string; devicePublicKey?: string };
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }
  if (
    typeof body.idToken !== "string" ||
    body.idToken.length === 0 ||
    typeof body.devicePublicKey !== "string" ||
    body.devicePublicKey.length === 0
  ) {
    return Response.json({ error: "idToken and devicePublicKey are required" }, { status: 400 });
  }

  let identity;
  try {
    identity = await verifyGoogleIdToken(body.idToken, allowedAudiences(env), googleJwks());
  } catch {
    return Response.json({ error: "invalid Google ID token" }, { status: 401 });
  }

  if (!isEmailAllowed(identity.email, env.ALLOWED_GOOGLE_EMAILS)) {
    return Response.json({ error: "email not allowed" }, { status: 403 });
  }

  try {
    const now = Date.now();

    const existingAccount = await findAccountByGoogleSub(db, identity.googleSub);
    // A phone must never become the account's first device (it can't create
    // the group key). Blank GOOGLE_IOS_CLIENT_ID never matches (reject-by-default).
    const isIos = !!env.GOOGLE_IOS_CLIENT_ID && identity.audience === env.GOOGLE_IOS_CLIENT_ID;
    const desktopRequired = () => Response.json({ error: "desktop_required" }, { status: 409 });
    if (isIos && !existingAccount) return desktopRequired(); // before createAccountDatabase
    let dataDbName: string;
    let dataDbUrl: string;
    if (existingAccount) {
      dataDbName = existingAccount.dataDbName;
      dataDbUrl = existingAccount.dataDbUrl;
    } else {
      const candidateDbName = `kestrel-sync-${crypto.randomUUID()}`;
      const createdDb = await createAccountDatabase(
        {
          platformApiToken: env.TURSO_PLATFORM_API_TOKEN,
          organizationSlug: env.TURSO_ORG_SLUG,
          group: env.TURSO_DB_GROUP,
        },
        candidateDbName,
      );
      dataDbName = createdDb.name;
      dataDbUrl = createdDb.url;
    }
    const account = await getOrCreateAccount(db, identity.email, identity.googleSub, now, dataDbName, dataDbUrl);

    let device = await findDeviceByPublicKey(db, account.id, body.devicePublicKey);
    if (device === null) {
      const activeDeviceCount = await countActiveDevicesForAccount(db, account.id);
      if (isIos && activeDeviceCount === 0) return desktopRequired();
      const status = activeDeviceCount === 0 ? "approved" : "pending";
      device = await enrollDevice(db, account.id, body.devicePublicKey, status, now);
    }

    if (device.status === "revoked") {
      return Response.json({ error: "device revoked" }, { status: 403 });
    }
    if (device.status === "pending") {
      return Response.json({ status: "pending" });
    }

    const tursoToken = await mintDeviceToken(
      {
        platformApiToken: env.TURSO_PLATFORM_API_TOKEN,
        organizationSlug: env.TURSO_ORG_SLUG,
        databaseName: account.dataDbName,
      },
      DEVICE_TOKEN_EXPIRATION,
    );
    const wrappedGroupKey = await fetchAndConsumeWrappedKey(env.ENROLLMENT_HANDOFF, device.id);

    return Response.json({
      status: "approved",
      tursoToken,
      tursoDatabaseUrl: account.dataDbUrl,
      wrappedGroupKey,
    });
  } catch (err) {
    console.error("handleGoogleAuth failed", err);
    return Response.json({ error: "sync backend unavailable" }, { status: 502 });
  }
}

interface AuthenticatedApprovedDeviceRequestBody {
  idToken?: string;
  devicePublicKey?: string;
}

/** Shared auth gate for routes that require the caller to already be an
 * approved device on some account (used by `handleListDevices` and
 * `handleApproveDevice` — NOT by `handleGoogleAuth`, which has its own
 * distinct pending/first-device enrollment logic). Returns either the
 * authenticated `{account, device}` pair, or a `Response` the caller
 * should return immediately. */
async function authenticateApprovedDevice(
  body: AuthenticatedApprovedDeviceRequestBody,
  env: Env,
  db: SqlExecutor,
  allowPending = false,
): Promise<{ account: AccountRecord; device: DeviceRecord } | Response> {
  if (
    typeof body.idToken !== "string" ||
    body.idToken.length === 0 ||
    typeof body.devicePublicKey !== "string" ||
    body.devicePublicKey.length === 0
  ) {
    return Response.json({ error: "idToken and devicePublicKey are required" }, { status: 400 });
  }
  let identity;
  try {
    identity = await verifyGoogleIdToken(body.idToken, allowedAudiences(env), googleJwks());
  } catch {
    return Response.json({ error: "invalid Google ID token" }, { status: 401 });
  }
  if (!isEmailAllowed(identity.email, env.ALLOWED_GOOGLE_EMAILS)) {
    return Response.json({ error: "email not allowed" }, { status: 403 });
  }
  const account = await findAccountByGoogleSub(db, identity.googleSub);
  if (!account) {
    return Response.json({ error: "device not approved" }, { status: 403 });
  }
  const device = await findDeviceByPublicKey(db, account.id, body.devicePublicKey);
  if (!device || (device.status !== "approved" && !(allowPending && device.status === "pending"))) {
    return Response.json({ error: "device not approved" }, { status: 403 });
  }
  return { account, device };
}

/** `db` defaults to a real Turso connection built from `env`, same
 * pattern as `handleGoogleAuth`. */
export async function handleListDevices(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
): Promise<Response> {
  let body: AuthenticatedApprovedDeviceRequestBody;
  try {
    body = (await request.json()) as AuthenticatedApprovedDeviceRequestBody;
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }

  try {
    const authResult = await authenticateApprovedDevice(body, env, db);
    if (authResult instanceof Response) return authResult;

    const devices = await listDevicesForAccount(db, authResult.account.id);
    return Response.json({ devices });
  } catch (err) {
    console.error("handleListDevices failed", err);
    return Response.json({ error: "sync backend unavailable" }, { status: 502 });
  }
}

interface ApproveDeviceRequestBody extends AuthenticatedApprovedDeviceRequestBody {
  wrappedGroupKey?: string;
}

/** `db` defaults to a real Turso connection built from `env`, same
 * pattern as `handleGoogleAuth`/`handleListDevices`. `targetDeviceId` is
 * the pending device's row id, taken from the route's `:id` path segment.
 * Only a `revoked` target device is rejected — an already-`approved`
 * device can be re-pushed a new wrapped key (e.g. after the handoff
 * store's 5-minute TTL expired before it polled), matching the design
 * spec's stated recovery path. This grants nothing an attacker doesn't
 * already have: the blob is wrapped to that specific device's own public
 * key, and the caller must already be an approved device on the same
 * account to reach this point at all. */
export async function handleApproveDevice(
  request: Request,
  env: Env,
  targetDeviceId: string,
  db: SqlExecutor = tursoSqlExecutor(env),
): Promise<Response> {
  let body: ApproveDeviceRequestBody;
  try {
    body = (await request.json()) as ApproveDeviceRequestBody;
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }
  if (typeof body.wrappedGroupKey !== "string" || body.wrappedGroupKey.length === 0) {
    return Response.json({ error: "wrappedGroupKey is required" }, { status: 400 });
  }

  try {
    const authResult = await authenticateApprovedDevice(body, env, db);
    if (authResult instanceof Response) return authResult;

    const targetDevice = await findDeviceById(db, targetDeviceId);
    if (!targetDevice || targetDevice.accountId !== authResult.account.id) {
      // Same 404 for "doesn't exist" and "belongs to a different account"
      // — never let a caller distinguish the two, which would leak
      // whether a device id exists on an account that isn't theirs.
      return Response.json({ error: "device not found" }, { status: 404 });
    }
    if (targetDevice.status === "revoked") {
      return Response.json({ error: "device is revoked" }, { status: 400 });
    }

    await storeWrappedKeyForHandoff(env.ENROLLMENT_HANDOFF, targetDevice.id, body.wrappedGroupKey);
    await approveDevice(db, targetDevice.id);

    return Response.json({ status: "approved" });
  } catch (err) {
    console.error("handleApproveDevice failed", err);
    return Response.json({ error: "sync backend unavailable" }, { status: 502 });
  }
}

/** `db` defaults to a real Turso connection built from `env`, same pattern
 * as `handleApproveDevice`. `targetDeviceId` is the device to revoke,
 * taken from the route's `:id` path segment. Rejects an already-revoked
 * target with 400, matching `handleApproveDevice`'s revoked-target
 * handling shape, so a caller can tell "just revoked it" apart from
 * "already was revoked" rather than getting a silent 200 either way.
 * After updating the device's own status, rotates every token for the
 * account's data-plane database — the only lever Turso's API exposes for
 * actually cutting off access rather than just blocking future
 * re-approval (see docs/superpowers/specs/2026-07-31-device-revocation-and-naming-design.md). */
export async function handleRevokeDevice(
  request: Request,
  env: Env,
  targetDeviceId: string,
  db: SqlExecutor = tursoSqlExecutor(env),
): Promise<Response> {
  let body: AuthenticatedApprovedDeviceRequestBody;
  try {
    body = (await request.json()) as AuthenticatedApprovedDeviceRequestBody;
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }

  try {
    const authResult = await authenticateApprovedDevice(body, env, db);
    if (authResult instanceof Response) return authResult;

    const targetDevice = await findDeviceById(db, targetDeviceId);
    if (!targetDevice || targetDevice.accountId !== authResult.account.id) {
      return Response.json({ error: "device not found" }, { status: 404 });
    }
    if (targetDevice.status === "revoked") {
      return Response.json({ error: "device is already revoked" }, { status: 400 });
    }

    await revokeDevice(db, targetDevice.id, Date.now());
    await rotateAllTokens({
      platformApiToken: env.TURSO_PLATFORM_API_TOKEN,
      organizationSlug: env.TURSO_ORG_SLUG,
      databaseName: authResult.account.dataDbName,
    });

    return Response.json({ status: "revoked" });
  } catch (err) {
    console.error("handleRevokeDevice failed", err);
    return Response.json({ error: "sync backend unavailable" }, { status: 502 });
  }
}

interface SetDeviceNameRequestBody extends AuthenticatedApprovedDeviceRequestBody {
  name?: string;
}

/** `db` defaults to a real Turso connection built from `env`, same pattern
 * as the other device routes. Unlike `handleApproveDevice`/
 * `handleRevokeDevice`, there is no `:id` path segment — this always
 * operates on the calling device's own record, resolved by
 * `authenticateApprovedDevice` from the request's own `idToken`/
 * `devicePublicKey`, never an arbitrary target (see
 * docs/superpowers/specs/2026-07-31-device-revocation-and-naming-design.md). */
export async function handleSetDeviceName(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
): Promise<Response> {
  let body: SetDeviceNameRequestBody;
  try {
    body = (await request.json()) as SetDeviceNameRequestBody;
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }
  if (typeof body.name !== "string" || body.name.length === 0) {
    return Response.json({ error: "name is required" }, { status: 400 });
  }

  try {
    // Pending devices may name themselves so the approver sees what they are approving.
    // The name is unverified text: control/format/private/unassigned/line-separator characters are
    // stripped, spaces collapsed, and it is capped at 100 code points; the comparison code is what vouches for the device.
    // Bound the input first (cheap), then strip non-printing/layout characters, collapse spaces.
    const name = [...body.name.slice(0, 400).replace(/[\p{Cc}\p{Cf}\p{Cs}\p{Co}\p{Cn}\p{Zl}\p{Zp}]/gu, "").replace(/\s+/g, " ")]
      .slice(0, 100)
      .join("")
      .trim();
    if (name.length === 0) {
      return Response.json({ error: "name is required" }, { status: 400 });
    }
    const authResult = await authenticateApprovedDevice(body, env, db, true);
    if (authResult instanceof Response) return authResult;

    await setDeviceName(db, authResult.device.id, name);

    return Response.json({ status: "ok" });
  } catch (err) {
    console.error("handleSetDeviceName failed", err);
    return Response.json({ error: "sync backend unavailable" }, { status: 502 });
  }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    try {
      const url = new URL(request.url);
      if (url.pathname === "/v1/auth/google" && request.method === "POST") {
        return await handleGoogleAuth(request, env);
      }
      if (url.pathname === "/v1/devices" && request.method === "POST") {
        return await handleListDevices(request, env);
      }
      if (url.pathname === "/v1/devices/name" && request.method === "POST") {
        return await handleSetDeviceName(request, env);
      }
      const approveMatch = url.pathname.match(/^\/v1\/devices\/([^/]+)\/approve$/);
      if (approveMatch && request.method === "POST") {
        return await handleApproveDevice(request, env, approveMatch[1]);
      }
      const revokeMatch = url.pathname.match(/^\/v1\/devices\/([^/]+)\/revoke$/);
      if (revokeMatch && request.method === "POST") {
        return await handleRevokeDevice(request, env, revokeMatch[1]);
      }
      return new Response("not found", { status: 404 });
    } catch (err) {
      console.error("fetch handler failed", err);
      return Response.json({ error: "sync backend unavailable" }, { status: 502 });
    }
  },
};
