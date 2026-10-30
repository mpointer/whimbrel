import type { ExecutionContext, KVNamespace, ScheduledController } from "@cloudflare/workers-types";
import { connect } from "@tursodatabase/serverless";
import { hasDeviceLimit, isSignInAllowed } from "./allowlist";
import { allowedAudiences, googleJwks, verifyGoogleIdToken } from "./googleAuth";
import { mintDeviceToken, createAccountDatabase, deleteAccountDatabase, rotateAllTokens } from "./turso";
import { kindOf, limitCheck } from "./limits";
import { storeWrappedKeyForHandoff, fetchAndConsumeWrappedKey } from "./handoff";
import { ipBucket, rateLimit, rateLimited } from "./rateLimit";
import { deleteAccount, runInactivity } from "./inactivity";
import {
  findAccountByGoogleSub,
  getOrCreateAccount,
  findDeviceByPublicKey,
  findDeviceById,
  countActiveDevicesForAccount,
  enrollDevice,
  platformFor,
  touchLastSeen,
  listDevicesForAccount,
  approveDevice,
  revokeDevice,
  setDeviceName,
  setReplacesDevice,
  deleteDevice,
  setAccountDatabase,
  retireDatabase,
  reserveAccountCreation,
  unretireDatabase,
  claimSwapForReset,
  unclaimSwapForReset,
  finishReset,
  recentlySeenApprovedDevices,
  addToWaitlist,
  claimForDeletion,
  AccountDeletingError,
  type AccountRecord,
  type DevicePlatform,
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
  /** Exactly "true" opens sign-in to any verified Google account. */
  OPEN_SIGN_IN?: string;
  /** Comma-separated emails exempt from the free device limit. */
  UNLIMITED_EMAILS?: string;
  RESEND_API_KEY?: string;
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

const accountDeleting = () => Response.json({ error: "account_deleting" }, { status: 503 });

/** A route's catch-all: an account claimed for deletion is a 503, anything else a 502. */
function backendError(label: string, err: unknown): Response {
  if (err instanceof AccountDeletingError) return accountDeleting();
  console.error(`${label} failed`, err);
  return Response.json({ error: "sync backend unavailable" }, { status: 502 });
}

/** `findAccountByGoogleSub`, but an account claimed for deletion throws
 * AccountDeletingError: no route may mint, enrol or touch it. */
async function liveAccount(db: SqlExecutor, googleSub: string): Promise<AccountRecord | null> {
  const account = await findAccountByGoogleSub(db, googleSub);
  if (account?.deletingSinceUnixMs != null) throw new AccountDeletingError();
  return account;
}

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
  let body: { idToken?: string; devicePublicKey?: string; platform?: unknown };
  try {
    body = (await request.json()) as { idToken?: string; devicePublicKey?: string; platform?: unknown };
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

  if (!isSignInAllowed(identity.email, env)) {
    return Response.json({ error: "email not allowed" }, { status: 403 });
  }

  try {
    const now = Date.now();

    const existingAccount = await liveAccount(db, identity.googleSub);
    // A phone must never become the account's first device (it can't create
    // the group key). Blank GOOGLE_IOS_CLIENT_ID never matches (reject-by-default).
    const isIos = isIosAudience(env, identity.audience);
    if (isIos && !existingAccount) return desktopRequired(); // before createAccountDatabase
    let dataDbName: string;
    let dataDbUrl: string;
    if (existingAccount) {
      dataDbName = existingAccount.dataDbName;
      dataDbUrl = existingAccount.dataDbUrl;
    } else {
      // Three a day per client IP (IPv6 by /64), reserved in the registry
      // before the database exists, so a burst can't overshoot. Skipped when
      // the header is absent (only outside Cloudflare). Fails closed: each
      // new account costs a database.
      const ip = request.headers.get("CF-Connecting-IP");
      if (ip) {
        const reserved = await reserveAccountCreation(db, ipBucket(ip), 3, now - 24 * 60 * 60 * 1000, now).catch((err) => {
          console.error("handleGoogleAuth: creation reservation failed, refusing", err);
          return false;
        });
        if (!reserved) return rateLimited();
      }
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
    // An existing account is only ever re-read, never re-inserted: if it was
    // deleted since the read above, inserting would recreate it on its
    // deleted database.
    const account = existingAccount
      ? await liveAccount(db, identity.googleSub)
      : await getOrCreateAccount(db, identity.email, identity.googleSub, now, dataDbName, dataDbUrl);
    if (!account) return accountDeleting();
    // Lost a simultaneous first sign-in: the winner's database is the
    // account's, so ours is unused. Delete it now; if that fails, the cron
    // retries it from the retired list.
    if (!existingAccount && account.dataDbName !== dataDbName) {
      const orphan = dataDbName;
      await deleteAccountDatabase(
        { platformApiToken: env.TURSO_PLATFORM_API_TOKEN, organizationSlug: env.TURSO_ORG_SLUG },
        orphan,
      ).catch(async (err) => {
        console.error("handleGoogleAuth: orphan delete failed, retiring it", err);
        await retireDatabase(db, account.id, orphan, now);
      });
    }

    let device = await findDeviceByPublicKey(db, account.id, body.devicePublicKey);
    if (device === null) {
      // Only new device keys count: a known device (a pending one polls every
      // few seconds) is never limited.
      if (!(await rateLimit(env.ENROLLMENT_HANDOFF, `auth:${identity.googleSub}`, 30, 10 * 60))) return rateLimited();
      const enrolled = await enrolNewDevice(db, env, identity.email, account.id, body.devicePublicKey, platformFor(body.platform, isIos), isIos, now);
      if (enrolled instanceof Response) return enrolled;
      device = enrolled;
    }

    if (device.status === "revoked") {
      return Response.json({ error: "device revoked" }, { status: 403 });
    }
    await touchLastSeen(db, account.id, device.id, now);
    // A device mid-reset gets no token until the reset has switched the
    // account to its new database.
    if (device.status === "pending" || device.status === "resetting") {
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
    return backendError("handleGoogleAuth", err);
  }
}

interface AuthenticatedApprovedDeviceRequestBody {
  idToken?: string;
  devicePublicKey?: string;
}

function isIosAudience(env: Env, audience: string): boolean {
  return !!env.GOOGLE_IOS_CLIENT_ID && audience === env.GOOGLE_IOS_CLIENT_ID;
}

/** Validates the body's credentials and the Google token, and applies the
 * sign-in gate. Does not look at the account or device. */
async function verifyCaller(
  body: AuthenticatedApprovedDeviceRequestBody,
  env: Env,
): Promise<{ email: string; googleSub: string; isIos: boolean; devicePublicKey: string } | Response> {
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
  if (!isSignInAllowed(identity.email, env)) {
    return Response.json({ error: "email not allowed" }, { status: 403 });
  }
  return {
    email: identity.email,
    googleSub: identity.googleSub,
    isIos: isIosAudience(env, identity.audience),
    devicePublicKey: body.devicePublicKey,
  };
}

const desktopRequired = () => Response.json({ error: "desktop_required" }, { status: 409 });

/** Enrols a device with no existing row: the account's first active device
 * is approved (and must not be a phone), any later one is pending, and the
 * free device limit is enforced after the insert. */
async function enrolNewDevice(
  db: SqlExecutor,
  env: Env,
  email: string,
  accountId: string,
  publicKey: string,
  platform: DevicePlatform,
  isIos: boolean,
  now: number,
): Promise<DeviceRecord | Response> {
  const activeDeviceCount = await countActiveDevicesForAccount(db, accountId);
  if (isIos && activeDeviceCount === 0) return desktopRequired();
  const status = activeDeviceCount === 0 ? "approved" : "pending";
  const device = await enrollDevice(db, accountId, publicKey, status, now, platform);
  const limited = await deviceLimitAfterEnrol(db, env, email, device, () => deleteDevice(db, device.id));
  return limited ?? device;
}

function deviceLimitResponse(kind: string, holders: DeviceRecord[]): Response {
  return Response.json(
    { error: "device_limit", kind, devices: holders.map(({ id, name, platform }) => ({ id, name, platform })) },
    { status: 409 },
  );
}

/** Runs after `device` is written. A device only counts rows inserted before
 * its own, so of two racing enrolments the later insert always yields:
 * either both see each other, or only the later one sees the earlier.
 * Calls `undo` to take back the write when over the limit, and also when
 * the check itself fails, so a transient error never leaves an unchecked
 * row behind. Returns the 409 to send, or null. */
async function deviceLimitAfterEnrol(
  db: SqlExecutor,
  env: Env,
  email: string,
  device: DeviceRecord,
  undo: () => Promise<void>,
): Promise<Response | null> {
  if (!hasDeviceLimit(email, env)) return null;
  try {
    const devices = await listDevicesForAccount(db, device.accountId);
    const own = devices.findIndex((d) => d.id === device.id);
    const earlier = (own < 0 ? devices : devices.slice(0, own)).filter((d) => d.id !== device.replacesDeviceId);
    const check = limitCheck(earlier, device.platform, false);
    if (check.ok) return null;
    await undo();
    return deviceLimitResponse(check.kind, check.holders);
  } catch (err) {
    await undo().catch((undoErr) => console.error("device limit undo failed", undoErr));
    throw err;
  }
}

/** Revokes a device and rotates every token on the account's database, so
 * the device's already-issued token stops working too. */
async function revokeAndRotate(db: SqlExecutor, env: Env, account: AccountRecord, deviceId: string): Promise<void> {
  await revokeDevice(db, deviceId, Date.now());
  await rotateAllTokens({
    platformApiToken: env.TURSO_PLATFORM_API_TOKEN,
    organizationSlug: env.TURSO_ORG_SLUG,
    databaseName: account.dataDbName,
  });
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
): Promise<{ account: AccountRecord; device: DeviceRecord; isIos: boolean } | Response> {
  const caller = await verifyCaller(body, env);
  if (caller instanceof Response) return caller;
  const account = await liveAccount(db, caller.googleSub);
  if (!account) {
    return Response.json({ error: "device not approved" }, { status: 403 });
  }
  const device = await findDeviceByPublicKey(db, account.id, caller.devicePublicKey);
  const waiting = device?.status === "pending" || device?.status === "resetting";
  if (!device || (device.status !== "approved" && !(allowPending && waiting))) {
    return Response.json({ error: "device not approved" }, { status: 403 });
  }
  await touchLastSeen(db, account.id, device.id, Date.now());
  return { account, device, isIos: caller.isIos };
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
    return backendError("handleListDevices", err);
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
    if (targetDevice.status === "resetting") {
      return Response.json({ error: "device is resetting" }, { status: 400 });
    }

    // A pending row that isn't a swap can still be over the limit (enrolled
    // before the limit existed, or left behind by a failed check), so check
    // again here rather than trust that it was checked at enrolment.
    if (
      targetDevice.status === "pending" &&
      !targetDevice.replacesDeviceId &&
      hasDeviceLimit(authResult.account.email, env)
    ) {
      const others = (await listDevicesForAccount(db, authResult.account.id)).filter((d) => d.id !== targetDevice.id);
      const check = limitCheck(others, targetDevice.platform, false);
      if (!check.ok) return deviceLimitResponse(check.kind, check.holders);
    }

    await storeWrappedKeyForHandoff(env.ENROLLMENT_HANDOFF, targetDevice.id, body.wrappedGroupKey);
    if (!(await approveDevice(db, targetDevice.id))) {
      // Revoked or claimed for a reset since it was read above. Drop the key
      // just stored, so a reset device never picks up the old group key.
      await fetchAndConsumeWrappedKey(env.ENROLLMENT_HANDOFF, targetDevice.id);
      const now = await findDeviceById(db, targetDevice.id);
      return Response.json(
        { error: now?.status === "resetting" ? "device is resetting" : "device is revoked" },
        { status: 400 },
      );
    }
    // Approving a swap retires the computer or phone it replaces. The link is
    // cleared only after the revoke, so a failed revoke is retried by
    // approving again.
    if (targetDevice.replacesDeviceId) {
      const replaced = await findDeviceById(db, targetDevice.replacesDeviceId);
      if (replaced && replaced.accountId === authResult.account.id && replaced.status !== "revoked") {
        await revokeAndRotate(db, env, authResult.account, replaced.id);
      }
      await setReplacesDevice(db, targetDevice.id, null);
    }

    return Response.json({ status: "approved" });
  } catch (err) {
    return backendError("handleApproveDevice", err);
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

    await revokeAndRotate(db, env, authResult.account, targetDevice.id);

    return Response.json({ status: "revoked" });
  } catch (err) {
    return backendError("handleRevokeDevice", err);
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
    return backendError("handleSetDeviceName", err);
  }
}

interface SwapDeviceRequestBody extends AuthenticatedApprovedDeviceRequestBody {
  platform?: unknown;
  replaceDeviceId?: string;
}

/** The "use this computer instead" path at the device limit. The caller is
 * not yet approved: it enrols (or stays) pending with `replaces_device_id`
 * set, and the replaced device keeps syncing until something approves the
 * caller, at which point `handleApproveDevice` revokes it. */
export async function handleSwapDevice(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
): Promise<Response> {
  let body: SwapDeviceRequestBody;
  try {
    body = (await request.json()) as SwapDeviceRequestBody;
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }

  try {
    const caller = await verifyCaller(body, env);
    if (caller instanceof Response) return caller;
    const account = await liveAccount(db, caller.googleSub);
    if (!account) {
      return Response.json({ error: "device not found" }, { status: 404 });
    }
    const existing = await findDeviceByPublicKey(db, account.id, caller.devicePublicKey);
    if (existing?.status === "revoked") {
      return Response.json({ error: "device revoked" }, { status: 403 });
    }
    if (existing?.status === "approved") {
      return Response.json({ error: "device already approved" }, { status: 400 });
    }
    if (existing?.status === "resetting") {
      return Response.json({ error: "reset_in_progress" }, { status: 409 });
    }
    const now = Date.now();
    let target = typeof body.replaceDeviceId === "string" ? await findDeviceById(db, body.replaceDeviceId) : null;
    // Swapping out another pending swap (the device the limit named) means
    // swapping out the device that swap was going to replace.
    if (target?.status === "pending" && target.replacesDeviceId && target.accountId === account.id && target.id !== existing?.id) {
      const orphan = target;
      target = await findDeviceById(db, orphan.replacesDeviceId!);
      // That device is gone, so the old swap can never complete. Swap out
      // the orphaned swap itself instead: the caller stays pending, so the
      // only ways forward are an approved device's approval or a reset (with
      // its quiet-period check and notice), never an automatic approval.
      if (!target || target.accountId !== account.id || target.status === "revoked") target = orphan;
    }
    if (target?.accountId === account.id && target.status === "resetting") {
      return Response.json({ error: "reset_in_progress" }, { status: 409 });
    }
    // Same 404 for missing, revoked, or on another account, as in approve/revoke.
    if (!target || target.accountId !== account.id || target.status === "revoked" || target.id === existing?.id) {
      return Response.json({ error: "device not found" }, { status: 404 });
    }
    const platform = existing ? existing.platform : platformFor(body.platform, caller.isIos);
    if (kindOf(platform) !== kindOf(target.platform)) {
      return Response.json({ error: "swap_kind_mismatch" }, { status: 400 });
    }

    // The newest swap of a device wins: older pending swaps of the same
    // target are abandoned attempts. Left in place, they would hold the slot
    // and, if later approved, leave two computers.
    for (const d of await listDevicesForAccount(db, account.id)) {
      if (d.id !== existing?.id && d.status === "pending" && d.replacesDeviceId === target.id) await deleteDevice(db, d.id);
    }

    const device =
      existing ?? (await enrollDevice(db, account.id, caller.devicePublicKey, "pending", now, platformFor(body.platform, caller.isIos)));
    await setReplacesDevice(db, device.id, target.id);
    const limited = await deviceLimitAfterEnrol(db, env, caller.email, { ...device, replacesDeviceId: target.id }, () =>
      existing ? setReplacesDevice(db, device.id, existing.replacesDeviceId ?? null) : deleteDevice(db, device.id),
    );
    if (limited) return limited;
    await touchLastSeen(db, account.id, device.id, now);
    return Response.json({ status: "pending" });
  } catch (err) {
    return backendError("handleSwapDevice", err);
  }
}

const WAITLIST_WANTS = ["more_devices", "routines_while_closed", "phone_notifications", "backups"];

interface WaitlistRequestBody extends AuthenticatedApprovedDeviceRequestBody {
  wants?: unknown;
  platform?: unknown;
}

/** "Join the waitlist" at the device limit. The caller's device need not be
 * enrolled (it was just refused), but the account must exist. */
export async function handleWaitlist(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
): Promise<Response> {
  let body: WaitlistRequestBody;
  try {
    body = (await request.json()) as WaitlistRequestBody;
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }
  const sent = Array.isArray(body.wants) ? body.wants : [];
  const wants = WAITLIST_WANTS.filter((w) => sent.includes(w));
  if (wants.length === 0) {
    return Response.json({ error: "wants_required" }, { status: 400 });
  }

  try {
    const caller = await verifyCaller(body, env);
    if (caller instanceof Response) return caller;
    if (!(await rateLimit(env.ENROLLMENT_HANDOFF, `waitlist:${caller.googleSub}`, 5, 24 * 60 * 60))) return rateLimited();
    const account = await liveAccount(db, caller.googleSub);
    if (!account) {
      return Response.json({ error: "device not approved" }, { status: 403 });
    }
    const now = Date.now();
    const device = await findDeviceByPublicKey(db, account.id, caller.devicePublicKey);
    if (device && device.status !== "revoked") await touchLastSeen(db, account.id, device.id, now);
    if (!(await addToWaitlist(db, account.id, wants, platformFor(body.platform, caller.isIos), now))) throw new AccountDeletingError();
    return Response.json({ status: "ok" });
  } catch (err) {
    return backendError("handleWaitlist", err);
  }
}

interface ResetAccountRequestBody extends AuthenticatedApprovedDeviceRequestBody {
  confirm?: unknown;
}

const RESET_QUIET_MS = 7 * 24 * 60 * 60 * 1000;

/** The fresh start, when nothing that holds the group key is left to
 * approve a swap. Only a pending swap device on a computer may call it (a
 * phone can't make a group key, so a phone reset would lock the account),
 * and only when no other approved device has been seen for 7 days, so a
 * stolen Google session can't use swap + reset to take over a live account.
 *
 * The device is claimed atomically by moving it to "resetting", so
 * concurrent resets can't make two databases. While resetting it is treated
 * like a pending device: auth mints no token, so nothing can write to the old
 * database through it. It becomes approved only after the account has
 * switched to the new database. Any failure hands the claim back (unless the
 * device was revoked meanwhile) so the client can retry.
 *
 * The old database is not deleted here: its tokens are rotated, it is
 * recorded in `retired_database` for the cron to delete after 30 days, and
 * the account is flagged for a reset notice email. */
export async function handleResetAccount(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
): Promise<Response> {
  let body: ResetAccountRequestBody;
  try {
    body = (await request.json()) as ResetAccountRequestBody;
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }
  if (body.confirm !== true) {
    return Response.json({ error: "reset_confirm_required" }, { status: 400 });
  }

  try {
    const authResult = await authenticateApprovedDevice(body, env, db, true);
    if (authResult instanceof Response) return authResult;
    const { account, device, isIos } = authResult;
    if (device.status !== "pending" || !device.replacesDeviceId) {
      return Response.json({ error: "device not approved" }, { status: 403 });
    }
    if (isIos || kindOf(device.platform) !== "computer") {
      return Response.json({ error: "reset_requires_computer" }, { status: 403 });
    }
    const now = Date.now();
    const active = await recentlySeenApprovedDevices(db, account.id, device.id, now - RESET_QUIET_MS);
    if (active.length > 0) {
      return Response.json({ error: "other_device_active", devices: active }, { status: 403 });
    }
    if (!(await claimSwapForReset(db, device.id))) {
      return Response.json({ error: "reset_in_progress" }, { status: 409 });
    }

    const dbConfig = {
      platformApiToken: env.TURSO_PLATFORM_API_TOKEN,
      organizationSlug: env.TURSO_ORG_SLUG,
      group: env.TURSO_DB_GROUP,
    };
    let created: { name: string; url: string } | undefined;
    let oldRetired = false;
    let switched = false;
    try {
      for (const other of await listDevicesForAccount(db, account.id)) {
        if (other.id !== device.id && other.status !== "revoked") await revokeDevice(db, other.id, now);
      }
      created = await createAccountDatabase(dbConfig, `kestrel-sync-${crypto.randomUUID()}`);
      // No token issued before now can write to the copy being retired.
      await rotateAllTokens({ ...dbConfig, databaseName: account.dataDbName });
      // Recorded before the switch, so it is never lost if a later step fails.
      await retireDatabase(db, account.id, account.dataDbName, now);
      oldRetired = true;
      // Fenced: if a deletion claimed the account meanwhile, abort through the
      // failure path below (the new database is retired, the device unclaimed).
      if (!(await setAccountDatabase(db, account.id, created.name, created.url, now))) throw new AccountDeletingError();
      switched = true;
      if (!(await finishReset(db, device.id))) {
        // Revoked while the reset ran: the account is reset, but this device gets nothing.
        return Response.json({ error: "device revoked" }, { status: 403 });
      }
    } catch (err) {
      try {
        if (!switched) {
          // A database made but never switched to is retired, not leaked;
          // the old one is still live, so it must not stay queued for deletion.
          if (created) await retireDatabase(db, account.id, created.name, now);
          if (oldRetired) await unretireDatabase(db, account.id, account.dataDbName, now);
        }
      } finally {
        // Back to pending so the client can retry (a retry after the switch
        // just makes and switches to another empty database).
        await unclaimSwapForReset(db, device.id);
      }
      throw err;
    }

    const tursoToken = await mintDeviceToken(
      {
        platformApiToken: env.TURSO_PLATFORM_API_TOKEN,
        organizationSlug: env.TURSO_ORG_SLUG,
        databaseName: created.name,
      },
      DEVICE_TOKEN_EXPIRATION,
    );
    return Response.json({ status: "approved", tursoToken, tursoDatabaseUrl: created.url, wrappedGroupKey: null });
  } catch (err) {
    return backendError("handleResetAccount", err);
  }
}

/** Self-serve deletion from an approved device: the data database (and any
 * retired ones), then every registry row for the account. Same confirm flag
 * and error code as the reset. */
export async function handleDeleteAccount(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
): Promise<Response> {
  let body: ResetAccountRequestBody;
  try {
    body = (await request.json()) as ResetAccountRequestBody;
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }
  if (body.confirm !== true) {
    return Response.json({ error: "reset_confirm_required" }, { status: 400 });
  }

  try {
    const authResult = await authenticateApprovedDevice(body, env, db);
    if (authResult instanceof Response) return authResult;
    // The claim shuts every other route out before anything is deleted.
    if (!(await claimForDeletion(db, authResult.account.id, Date.now()))) return accountDeleting();
    await deleteAccount(db, env, authResult.account);
    return Response.json({ status: "deleted" });
  } catch (err) {
    return backendError("handleDeleteAccount", err);
  }
}

export default {
  async scheduled(_controller: ScheduledController, env: Env, ctx: ExecutionContext): Promise<void> {
    ctx.waitUntil(
      runInactivity(tursoSqlExecutor(env), env, Date.now(), fetch).then((r) => console.log("inactivity", JSON.stringify(r))),
    );
  },

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
      if (url.pathname === "/v1/devices/swap" && request.method === "POST") {
        return await handleSwapDevice(request, env);
      }
      if (url.pathname === "/v1/waitlist" && request.method === "POST") {
        return await handleWaitlist(request, env);
      }
      if (url.pathname === "/v1/account/reset" && request.method === "POST") {
        return await handleResetAccount(request, env);
      }
      if (url.pathname === "/v1/account/delete" && request.method === "POST") {
        return await handleDeleteAccount(request, env);
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
