import type { ExecutionContext, KVNamespace, ScheduledController } from "@cloudflare/workers-types";
import { connect } from "@tursodatabase/serverless";
import { hasDeviceLimit, isSignInAllowed } from "./allowlist";
import { allowedAudiences, googleJwks, verifyGoogleIdToken } from "./googleAuth";
import { mintDeviceToken, createAccountDatabase, deleteAccountDatabase, rotateAllTokens } from "./turso";
import { isCloud, kindOf, limitCheck } from "./limits";
import { storeWrappedKeyForHandoff, fetchAndConsumeWrappedKey } from "./handoff";
import { ipBucket, rateLimit, rateLimited } from "./rateLimit";
import { deleteAccount, runInactivity } from "./inactivity";
import { aiBalance, creditPurchase, lastPurchaseMicros, purchaseByStripeRef, refundPurchase, settleStaleHolds } from "./aiCredit";
import { billingConfigured, isAiCreditEvent, planFromSubscription, stripeRequest, subscriptionIdOf, verifyStripeSignature } from "./stripe";
import { handleAiMessages, offeredAiModels } from "./aiProxy";
import { EMAIL_FROM, EMAIL_REPLY_TO, sendEmail } from "./email";
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
  findAccountById,
  setStripeCustomer,
  applyPlan,
  stripeEventSeen,
  recordStripeEvent,
  AccountDeletingError,
  type AccountRecord,
  type DevicePlatform,
  type PhoneAudience,
  type DeviceRecord,
  type SqlExecutor,
  type SqlRow,
  type SqlStatement,
} from "./accounts";

export interface Env {
  GOOGLE_CLIENT_ID: string;
  /** iOS app OAuth client ID; empty/unset = iOS tokens rejected. */
  GOOGLE_IOS_CLIENT_ID?: string;
  /** Android app Web OAuth client ID; empty/unset = Android tokens rejected. */
  GOOGLE_ANDROID_CLIENT_ID?: string;
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
  STRIPE_SECRET_KEY?: string;
  STRIPE_WEBHOOK_SECRET?: string;
  STRIPE_PRICE_MONTHLY?: string;
  STRIPE_PRICE_YEARLY?: string;
  /** Optional Billing Portal configuration (`bpc_…`) for Whimbrel; the Stripe
   * account is shared, so the default portal could offer another product. */
  STRIPE_PORTAL_CONFIGURATION?: string;
  /** Whimbrel's own Anthropic key for the AI credit proxy; unset = 503 ai_unavailable. */
  ANTHROPIC_API_KEY?: string;
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
    async batch(stmts: SqlStatement[]): Promise<SqlRow[][]> {
      // With a mode, the client sends one Hrana request: BEGIN IMMEDIATE,
      // each statement gated on the one before succeeding, COMMIT, and a
      // ROLLBACK if anything failed. It throws on any statement error.
      const results = (await conn.batch(
        stmts.map((s) => ({ sql: s.sql, args: s.params as unknown[] })),
        "immediate",
      )) as { rows: SqlRow[] }[];
      return results.map((r) => r.rows);
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
    // the group key). Blank GOOGLE_IOS_CLIENT_ID / GOOGLE_ANDROID_CLIENT_ID never match (reject-by-default).
    const phone = phoneAudience(env, identity.audience);
    if (phone && !existingAccount) return desktopRequired(); // before createAccountDatabase
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
      const enrolled = await enrolNewDevice(db, env, account, body.devicePublicKey, platformFor(body.platform, phone), phone, now);
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

function phoneAudience(env: Env, audience: string): PhoneAudience | null {
  if (env.GOOGLE_IOS_CLIENT_ID && audience === env.GOOGLE_IOS_CLIENT_ID) return "ios";
  if (env.GOOGLE_ANDROID_CLIENT_ID && audience === env.GOOGLE_ANDROID_CLIENT_ID) return "android";
  return null;
}

/** Validates the body's credentials and the Google token, and applies the
 * sign-in gate. Does not look at the account or device. */
async function verifyCaller(
  body: AuthenticatedApprovedDeviceRequestBody,
  env: Env,
): Promise<{ email: string; googleSub: string; phone: PhoneAudience | null; devicePublicKey: string } | Response> {
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
    phone: phoneAudience(env, identity.audience),
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
  account: AccountRecord,
  publicKey: string,
  platform: DevicePlatform,
  phone: PhoneAudience | null,
  now: number,
): Promise<DeviceRecord | Response> {
  const activeDeviceCount = await countActiveDevicesForAccount(db, account.id);
  if (phone && activeDeviceCount === 0) return desktopRequired();
  const status = activeDeviceCount === 0 ? "approved" : "pending";
  const device = await enrollDevice(db, account.id, publicKey, status, now, platform);
  const limited = await deviceLimitAfterEnrol(db, env, account, device, () => deleteDevice(db, device.id), now);
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
  account: AccountRecord,
  device: DeviceRecord,
  undo: () => Promise<void>,
  now: number,
): Promise<Response | null> {
  if (!hasDeviceLimit(account.email, env, isCloud(account, now))) return null;
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
): Promise<{ account: AccountRecord; device: DeviceRecord; phone: PhoneAudience | null } | Response> {
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
  return { account, device, phone: caller.phone };
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
    const now = Date.now();
    if (
      targetDevice.status === "pending" &&
      !targetDevice.replacesDeviceId &&
      hasDeviceLimit(authResult.account.email, env, isCloud(authResult.account, now))
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
    const platform = existing ? existing.platform : platformFor(body.platform, caller.phone);
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
      existing ?? (await enrollDevice(db, account.id, caller.devicePublicKey, "pending", now, platformFor(body.platform, caller.phone)));
    await setReplacesDevice(db, device.id, target.id);
    const limited = await deviceLimitAfterEnrol(db, env, account, { ...device, replacesDeviceId: target.id }, () =>
      existing ? setReplacesDevice(db, device.id, existing.replacesDeviceId ?? null) : deleteDevice(db, device.id),
      now,
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
    if (!(await addToWaitlist(db, account.id, wants, platformFor(body.platform, caller.phone), now))) throw new AccountDeletingError();
    return Response.json({ status: "ok" });
  } catch (err) {
    return backendError("handleWaitlist", err);
  }
}

/** The auth recipe for the billing routes: like the waitlist, the caller's
 * device need not be enrolled (at the free limit it was just refused). The
 * Google token proves account ownership; nothing here grants sync access. */
async function billingAccount(
  body: AuthenticatedApprovedDeviceRequestBody,
  env: Env,
  db: SqlExecutor,
  now: number,
): Promise<{ account: AccountRecord; googleSub: string } | Response> {
  const caller = await verifyCaller(body, env);
  if (caller instanceof Response) return caller;
  const account = await liveAccount(db, caller.googleSub);
  if (!account) return Response.json({ error: "device not approved" }, { status: 403 });
  const device = await findDeviceByPublicKey(db, account.id, caller.devicePublicKey);
  if (device && device.status !== "revoked") await touchLastSeen(db, account.id, device.id, now);
  return { account, googleSub: caller.googleSub };
}

const billingUnavailable = () => Response.json({ error: "billing_unavailable" }, { status: 503 });
/** Stripe's own error body never reaches the client. */
const billingFailed = () => Response.json({ error: "billing_failed" }, { status: 502 });

async function readBody(request: Request): Promise<AuthenticatedApprovedDeviceRequestBody | null> {
  try {
    return (await request.json()) as AuthenticatedApprovedDeviceRequestBody;
  } catch {
    return null;
  }
}

/** The caller's plan, for the app's Cloud row. */
export async function handleAccountPlan(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
  now: number = Date.now(),
): Promise<Response> {
  const body = await readBody(request);
  if (!body) return Response.json({ error: "invalid JSON body" }, { status: 400 });
  try {
    const auth = await billingAccount(body, env, db, now);
    if (auth instanceof Response) return auth;
    const { account } = auth;
    const cloud = isCloud(account, now);
    return Response.json({
      plan: cloud ? "cloud" : "free",
      until: cloud ? account.planUntilUnixMs : null,
      interval: cloud ? account.planInterval : null,
      renews: cloud ? account.planRenews : null,
      billing: billingConfigured(env),
    });
  } catch (err) {
    return backendError("handleAccountPlan", err);
  }
}

/** Starts a Stripe Checkout session for Cloud and returns its URL. */
export async function handleBillingCheckout(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
  fetcher: typeof fetch = fetch,
  now: number = Date.now(),
): Promise<Response> {
  const body = (await readBody(request)) as (AuthenticatedApprovedDeviceRequestBody & { interval?: unknown }) | null;
  if (!body) return Response.json({ error: "invalid JSON body" }, { status: 400 });
  const interval = body.interval;
  if (interval !== "month" && interval !== "year") return Response.json({ error: "interval_required" }, { status: 400 });
  if (!billingConfigured(env)) return billingUnavailable();
  try {
    const auth = await billingAccount(body, env, db, now);
    if (auth instanceof Response) return auth;
    const { account, googleSub } = auth;
    if (!(await rateLimit(env.ENROLLMENT_HANDOFF, `billing:${googleSub}`, 20, 60 * 60))) return rateLimited();
    if (isCloud(account, now)) return Response.json({ error: "already_cloud" }, { status: 409 });
    const params: Record<string, string> = {
      mode: "subscription",
      "line_items[0][price]": interval === "year" ? env.STRIPE_PRICE_YEARLY! : env.STRIPE_PRICE_MONTHLY!,
      "line_items[0][quantity]": "1",
      client_reference_id: account.id,
      "subscription_data[metadata][account_id]": account.id,
      "automatic_tax[enabled]": "true",
      billing_address_collection: "required",
      success_url: "https://whimbrelapp.com/cloud/thanks/",
      cancel_url: "https://whimbrelapp.com/cloud/cancelled/",
    };
    if (account.stripeCustomerId) {
      params.customer = account.stripeCustomerId;
      params["customer_update[address]"] = "auto";
    } else {
      params.customer_email = account.email;
    }
    const res = await stripeRequest(env.STRIPE_SECRET_KEY!, "POST", "checkout/sessions", params, fetcher);
    if (!res.ok || typeof res.body?.url !== "string") return billingFailed();
    return Response.json({ url: res.body.url });
  } catch (err) {
    return backendError("handleBillingCheckout", err);
  }
}

/** The AI credit packs, in dollars, and what each costs in cents before tax. */
const AI_PACKS: Record<string, number> = { "10": 1000, "25": 2500 };

/** Starts a Stripe Checkout session (one-time payment) for a pack of AI
 * credit and returns its URL. The webhook credits it. */
export async function handleAiCheckout(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
  fetcher: typeof fetch = fetch,
  now: number = Date.now(),
): Promise<Response> {
  const body = (await readBody(request)) as (AuthenticatedApprovedDeviceRequestBody & { pack?: unknown }) | null;
  if (!body) return Response.json({ error: "invalid JSON body" }, { status: 400 });
  const cents = typeof body.pack === "string" && Object.hasOwn(AI_PACKS, body.pack) ? AI_PACKS[body.pack] : null;
  if (cents === null) return Response.json({ error: "pack_required" }, { status: 400 });
  if (!billingConfigured(env)) return billingUnavailable();
  try {
    const auth = await billingAccount(body, env, db, now);
    if (auth instanceof Response) return auth;
    const { account, googleSub } = auth;
    if (!(await rateLimit(env.ENROLLMENT_HANDOFF, `billing:${googleSub}`, 20, 60 * 60))) return rateLimited();
    // The Stripe account is shared: the webhook acts only on kind=ai_credit,
    // on the session (purchase) and on the PaymentIntent (refund).
    const params: Record<string, string> = {
      mode: "payment",
      "line_items[0][price_data][currency]": "usd",
      "line_items[0][price_data][unit_amount]": String(cents),
      "line_items[0][price_data][product_data][name]": "Whimbrel AI credit",
      "line_items[0][price_data][product_data][tax_code]": "txcd_10000000",
      "line_items[0][price_data][tax_behavior]": "exclusive",
      "line_items[0][quantity]": "1",
      "metadata[kind]": "ai_credit",
      "metadata[account_id]": account.id,
      "payment_intent_data[metadata][kind]": "ai_credit",
      "payment_intent_data[metadata][account_id]": account.id,
      "automatic_tax[enabled]": "true",
      // Charge in USD only, so the subtotal is always one of AI_PACKS.
      "adaptive_pricing[enabled]": "false",
      billing_address_collection: "required",
      success_url: "https://whimbrelapp.com/ai/thanks/",
      cancel_url: "https://whimbrelapp.com/ai/cancelled/",
    };
    if (account.stripeCustomerId) {
      params.customer = account.stripeCustomerId;
      params["customer_update[address]"] = "auto";
    } else {
      params.customer_email = account.email;
    }
    const res = await stripeRequest(env.STRIPE_SECRET_KEY!, "POST", "checkout/sessions", params, fetcher);
    if (!res.ok || typeof res.body?.url !== "string") return billingFailed();
    return Response.json({ url: res.body.url });
  } catch (err) {
    return backendError("handleAiCheckout", err);
  }
}

/** The caller's AI credit balance and the size of their newest purchase,
 * both in micro-dollars. */
export async function handleAiBalance(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
  now: number = Date.now(),
): Promise<Response> {
  const body = await readBody(request);
  if (!body) return Response.json({ error: "invalid JSON body" }, { status: 400 });
  try {
    const auth = await billingAccount(body, env, db, now);
    if (auth instanceof Response) return auth;
    // Its own bucket: the app polls this, and polling must never block checkout.
    if (!(await rateLimit(env.ENROLLMENT_HANDOFF, `ai-balance:${auth.googleSub}`, 120, 60 * 60))) return rateLimited();
    const id = auth.account.id;
    return Response.json({ balance_micros: await aiBalance(db, id), last_purchase_micros: await lastPurchaseMicros(db, id) });
  } catch (err) {
    return backendError("handleAiBalance", err);
  }
}

/** The models Whimbrel AI offers (`offeredAiModels`), newest first, for the
 * app's model picker. Same body and sign-in as the balance, with its own
 * rate-limit bucket. */
export async function handleAiModels(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
  now: number = Date.now(),
  fetcher: typeof fetch = fetch,
): Promise<Response> {
  const body = await readBody(request);
  if (!body) return Response.json({ error: "invalid JSON body" }, { status: 400 });
  try {
    const auth = await billingAccount(body, env, db, now);
    if (auth instanceof Response) return auth;
    if (!(await rateLimit(env.ENROLLMENT_HANDOFF, `ai-models:${auth.googleSub}`, 60, 60 * 60))) return rateLimited();
    return Response.json({ models: await offeredAiModels(env, fetcher) });
  } catch (err) {
    return backendError("handleAiModels", err);
  }
}

/** Opens the Stripe customer portal for an account that has paid before. */
export async function handleBillingPortal(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
  fetcher: typeof fetch = fetch,
): Promise<Response> {
  const body = await readBody(request);
  if (!body) return Response.json({ error: "invalid JSON body" }, { status: 400 });
  if (!billingConfigured(env)) return billingUnavailable();
  try {
    const auth = await billingAccount(body, env, db, Date.now());
    if (auth instanceof Response) return auth;
    if (!(await rateLimit(env.ENROLLMENT_HANDOFF, `billing:${auth.googleSub}`, 20, 60 * 60))) return rateLimited();
    const customer = auth.account.stripeCustomerId;
    if (!customer) return Response.json({ error: "no_subscription" }, { status: 409 });
    const params: Record<string, string> = { customer, return_url: "https://whimbrelapp.com/cloud/thanks/" };
    if (env.STRIPE_PORTAL_CONFIGURATION) params.configuration = env.STRIPE_PORTAL_CONFIGURATION;
    const res = await stripeRequest(env.STRIPE_SECRET_KEY!, "POST", "billing_portal/sessions", params, fetcher);
    if (!res.ok || typeof res.body?.url !== "string") return billingFailed();
    return Response.json({ url: res.body.url });
  } catch (err) {
    return backendError("handleBillingPortal", err);
  }
}

/** The Stripe account is shared with another product: a subscription is ours
 * to act on when it is on one of Whimbrel's two prices, or when its
 * `metadata.account_id` names `accountId`, a Whimbrel account that exists (so a
 * renewal on a retired price still lands). Pass null when no account matched. */
function isWhimbrelSubscription(sub: any, env: Env, accountId: string | null): boolean {
  if (accountId !== null && sub?.metadata?.account_id === accountId) return true;
  const items: any[] = Array.isArray(sub?.items?.data) ? sub.items.data : [];
  return items.some((i) => {
    const price = i?.price?.id;
    return typeof price === "string" && (price === env.STRIPE_PRICE_MONTHLY || price === env.STRIPE_PRICE_YEARLY);
  });
}

/** Not an error to the caller: Stripe retries any non-2xx for three days. */
const stripeRetryLater = () => Response.json({ error: "stripe_unavailable" }, { status: 500 });

/** Emails the alert address so Mike can refund a duplicate by hand. When no
 * email goes out, the log carries everything the refund needs (ids only,
 * never the person's email address). */
async function alertDuplicateSubscription(env: Env, accountId: string, kept: string, cancelled: string, customer: unknown, fetcher: typeof fetch) {
  console.error("billing: duplicate subscription cancelled", cancelled, "kept", kept);
  const sent =
    !!env.RESEND_API_KEY &&
    (await sendEmail(
    env.RESEND_API_KEY,
    {
      from: EMAIL_FROM,
      to: EMAIL_REPLY_TO,
      subject: "Refund a duplicate Whimbrel Cloud subscription",
      text:
        `Account ${accountId} started a second Whimbrel Cloud subscription while one was live. ` +
        `${cancelled} was cancelled at once; ${kept} is kept. ` +
        `Refund what ${cancelled} charged by hand in Stripe (customer ${typeof customer === "string" ? customer : "unknown"}).`,
    },
    fetcher,
  ));
  if (!sent) console.error("billing: duplicate alert not sent", "account", accountId, "cancelled", cancelled, "kept", kept, "customer", customer);
}

/** Emails the alert address after cancelling a subscription whose account
 * is deleted or being deleted (a Checkout completed late, say). Ids only. */
async function alertDeletedAccountSubscription(env: Env, accountId: string, subscriptionId: string, customer: unknown, fetcher: typeof fetch) {
  console.error("billing: deleted account's subscription cancelled", subscriptionId);
  const sent =
    !!env.RESEND_API_KEY &&
    (await sendEmail(
    env.RESEND_API_KEY,
    {
      from: EMAIL_FROM,
      to: EMAIL_REPLY_TO,
      subject: "Refund a Whimbrel Cloud subscription for a deleted account",
      text:
        `Subscription ${subscriptionId} was cancelled because the account was deleted (account ${accountId}). ` +
        `No refund was issued: refund by hand in Stripe what it charged (customer ${typeof customer === "string" ? customer : "unknown"}).`,
    },
    fetcher,
  ));
  if (!sent) console.error("billing: deleted account alert not sent", "account", accountId, "subscription", subscriptionId, "customer", customer);
}

/** Stripe amounts are cents; credit is micro-dollars. */
const MICROS_PER_CENT = 10_000;

/** Emails the alert address when a paid AI credit session could not be
 * credited (missing or deleting account, or an unusable payload), so Mike
 * can refund it by hand. Ids and the amount only. Returns true once the
 * email is sent; anything else asks Stripe to redeliver the event, so the
 * alert is tried again rather than lost. */
async function alertUncreditedAiPurchase(env: Env, session: any, accountId: unknown, reason: string, fetcher: typeof fetch): Promise<Response | null> {
  const cents = session?.amount_total;
  const currency = typeof session?.currency === "string" ? session.currency.toUpperCase() : "unknown currency";
  const amount = typeof cents === "number" ? `${(cents / 100).toFixed(2)} ${currency}` : "an unknown amount";
  console.error("ai credit: paid session not credited", session?.id, "account", accountId, reason);
  const sent =
    !!env.RESEND_API_KEY &&
    (await sendEmail(
      env.RESEND_API_KEY,
      {
        from: EMAIL_FROM,
        to: EMAIL_REPLY_TO,
        subject: "Refund a Whimbrel AI credit purchase that was not credited",
        text:
          `Checkout session ${session?.id ?? "unknown"} paid ${amount} for AI credit but was not credited (${reason}, account ${String(accountId)}). ` +
          `Refund it by hand in Stripe (payment ${typeof session?.payment_intent === "string" ? session.payment_intent : "unknown"}).`,
      },
      fetcher,
    ));
  if (sent) return null;
  console.error("ai credit: uncredited alert not sent", "session", session?.id, "account", accountId);
  return stripeRetryLater();
}

/** Credits a paid AI credit Checkout Session: its pre-tax subtotal (ruling
 * R2), once per PaymentIntent (ruling R6: the refund path finds the purchase
 * by it). Only what was sold is credited: USD, a subtotal that is exactly one
 * pack, no discount. A paid session that cannot be credited is alerted and
 * skipped; Stripe is not asked to retry what can never succeed, only an
 * alert that failed to send. Returns a Response only when Stripe must
 * retry. */
async function creditAiSession(env: Env, db: SqlExecutor, session: any, now: number, fetcher: typeof fetch): Promise<Response | null> {
  if (session?.payment_status !== "paid") {
    console.log("ai credit: session not paid", session?.id, session?.payment_status);
    return null;
  }
  const accountId = session.metadata?.account_id;
  const account = typeof accountId === "string" ? await findAccountById(db, accountId) : null;
  if (!account || account.deletingSinceUnixMs != null) {
    return alertUncreditedAiPurchase(env, session, accountId, account ? "account being deleted" : "account missing", fetcher);
  }
  const cents = session.amount_subtotal;
  const pi = session.payment_intent;
  const discount = session.total_details?.amount_discount;
  if (session.currency !== "usd" || !Object.values(AI_PACKS).includes(cents) || (discount != null && discount !== 0)) {
    return alertUncreditedAiPurchase(env, session, accountId, "not a USD pack at full price", fetcher);
  }
  if (typeof pi !== "string") {
    return alertUncreditedAiPurchase(env, session, accountId, "no payment intent", fetcher);
  }
  if (await creditPurchase(db, account.id, cents * MICROS_PER_CENT, pi, now)) return null;
  // Not credited: a repeat of a purchase already credited, or the account
  // was deleted (or claimed for deletion) after it was read above.
  if (await purchaseByStripeRef(db, pi)) return null;
  return alertUncreditedAiPurchase(env, session, accountId, "account deleted before the credit landed", fetcher);
}

/** Handles `charge.refunded`. The purchase is found by the charge's
 * PaymentIntent. Each refund on the charge is applied once, keyed on the
 * refund id (ruling R4), and removes its pre-tax share of what was credited:
 * round(refund / charge amount x credited) (ruling R6). Failed or cancelled
 * refunds are skipped. Returns a Response only when Stripe must retry. */
async function refundAiCharge(db: SqlExecutor, key: string, charge: any, now: number, fetcher: typeof fetch): Promise<Response | null> {
  const piField = charge?.payment_intent;
  const pi = typeof piField === "string" ? piField : piField?.id;
  const purchase = typeof pi === "string" ? await purchaseByStripeRef(db, pi) : null;
  if (!purchase) {
    // Another product's charge is recorded and ignored. An AI credit charge
    // with no purchase row may be a refund Stripe sent before the purchase
    // event: retry (event not recorded) until the purchase lands.
    let meta = charge?.metadata;
    if (meta?.kind !== "ai_credit" && typeof piField === "string") {
      const res = await stripeRequest(key, "GET", `payment_intents/${encodeURIComponent(piField)}`, {}, fetcher);
      if (!res.ok) return stripeRetryLater();
      meta = res.body?.metadata;
    } else if (meta?.kind !== "ai_credit") {
      meta = piField?.metadata;
    }
    if (meta?.kind !== "ai_credit") return null;
    console.error("ai credit: refund with no purchase row yet", charge?.id, "payment", pi);
    return stripeRetryLater();
  }
  const chargeCents = charge?.amount;
  if (!Number.isSafeInteger(chargeCents) || chargeCents <= 0) {
    console.error("ai credit: refunded charge without a whole amount", charge?.id);
    return null;
  }
  // Recent API versions leave `refunds` off the Charge: list them instead.
  let refunds: any[];
  if (Array.isArray(charge?.refunds?.data) && charge.refunds.has_more !== true) {
    refunds = charge.refunds.data;
  } else {
    if (typeof charge?.id !== "string") return null;
    const res = await stripeRequest(key, "GET", "refunds", { charge: charge.id, limit: "100" }, fetcher);
    if (!res.ok || !Array.isArray(res.body?.data)) return stripeRetryLater();
    refunds = res.body.data;
  }
  for (const r of refunds) {
    if (typeof r?.id !== "string" || r.status === "failed" || r.status === "canceled") continue;
    if (!Number.isSafeInteger(r.amount) || r.amount < 0) {
      console.error("ai credit: refund without a whole amount", r.id);
      continue;
    }
    const micros = Math.round((r.amount / chargeCents) * purchase.micros);
    await refundPurchase(db, purchase.accountId, micros, r.id, now);
  }
  return null;
}

/** Stripe's webhook. Verified on the raw body; each event id is handled
 * once. The plan comes from a fresh read of the subscription, never the
 * event payload, because Stripe does not order its events. Stripe failures
 * return 500 (event not recorded) so Stripe retries; events for unknown or
 * deleting accounts write nothing, but a live Cloud subscription of theirs is
 * cancelled and alerted. */
export async function handleBillingWebhook(
  request: Request,
  env: Env,
  db: SqlExecutor = tursoSqlExecutor(env),
  fetcher: typeof fetch = fetch,
  now: number = Date.now(),
): Promise<Response> {
  if (!billingConfigured(env)) return billingUnavailable();
  const raw = await request.text();
  if (!(await verifyStripeSignature(raw, request.headers.get("stripe-signature"), env.STRIPE_WEBHOOK_SECRET!, now))) {
    console.log("billing webhook: bad signature");
    return Response.json({ error: "bad_signature" }, { status: 400 });
  }
  let event: any;
  try {
    event = JSON.parse(raw);
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }
  if (typeof event?.id !== "string") return Response.json({ error: "invalid JSON body" }, { status: 400 });
  const received = async () => {
    await recordStripeEvent(db, event.id, now);
    return Response.json({ received: true });
  };
  try {
    if (await stripeEventSeen(db, event.id)) return Response.json({ received: true });
    // Whimbrel AI credit: a one-time payment, never a subscription.
    const aiSessionEvent = event.type === "checkout.session.completed" || event.type === "checkout.session.async_payment_succeeded";
    if (aiSessionEvent && event.data?.object?.mode === "payment") {
      const retry = isAiCreditEvent(event) ? await creditAiSession(env, db, event.data.object, now, fetcher) : null;
      return retry ?? (await received());
    }
    if (event.type === "charge.refunded") {
      const retry = await refundAiCharge(db, env.STRIPE_SECRET_KEY!, event.data?.object, now, fetcher);
      return retry ?? (await received());
    }
    const subId = subscriptionIdOf(event);
    if (typeof subId !== "string") return await received();
    const key = env.STRIPE_SECRET_KEY!;
    const res = await stripeRequest(key, "GET", `subscriptions/${encodeURIComponent(subId)}`, {}, fetcher);
    // A 404 too: it means a wrong or test/live-mismatched key, so recording
    // the event would drop a paid checkout; Stripe retries for days.
    if (!res.ok) return stripeRetryLater();
    const sub = res.body;
    const obj = event.data?.object;
    const metaId = sub?.metadata?.account_id;
    const accountId = typeof metaId === "string" ? metaId : obj?.client_reference_id;
    const account = typeof accountId === "string" ? await findAccountById(db, accountId) : null;
    // Another product's subscription, invoice or checkout: acknowledge, touch nothing.
    if (!isWhimbrelSubscription(sub, env, account?.id ?? null)) {
      console.log("billing: ignored subscription", sub?.id, sub?.status);
      return await received();
    }
    let state = planFromSubscription(sub, now);
    if (!account || account.deletingSinceUnixMs != null) {
      // A Checkout completed (or renewed) after the account was deleted would
      // charge forever: cancel it (no proration, no refund) and alert. The
      // account's own stored subscription is left to deleteAccount.
      if (typeof metaId === "string" && state?.plan === "cloud" && account?.stripeSubscriptionId !== subId) {
        const del = await stripeRequest(key, "DELETE", `subscriptions/${encodeURIComponent(subId)}`, {}, fetcher);
        if (!del.ok && del.status !== 404) {
          console.error("billing: deleted account cancel failed", del.status, subId);
          return stripeRetryLater();
        }
        await alertDeletedAccountSubscription(env, metaId, subId, sub?.customer, fetcher);
      }
      return await received();
    }
    let applyId = subId;
    let keptSub = sub;
    const stored = account.stripeSubscriptionId;
    // ponytail: read-then-cancel, so two concurrent first events for two new
    // subscriptions can both miss each other; Mike's refund-by-hand covers it.
    if (state?.plan === "cloud" && stored && stored !== subId) {
      const other = await stripeRequest(key, "GET", `subscriptions/${encodeURIComponent(stored)}`, {}, fetcher);
      if (!other.ok && other.status !== 404) return stripeRetryLater();
      // Only two Whimbrel subscriptions on one account are a duplicate.
      const otherState = other.ok && isWhimbrelSubscription(other.body, env, account.id) ? planFromSubscription(other.body, now) : null;
      if (otherState?.plan === "cloud") {
        // Two live subscriptions: keep the older, cancel the newer (no proration, no refund).
        const storedIsOlder = (other.body?.created ?? 0) <= (sub?.created ?? 0);
        const [kept, cancelled] = storedIsOlder ? [stored, subId] : [subId, stored];
        const del = await stripeRequest(key, "DELETE", `subscriptions/${encodeURIComponent(cancelled)}`, {}, fetcher);
        if (!del.ok && del.status !== 404) {
          console.error("billing: duplicate cancel failed", del.status, "cancelling", cancelled, "kept", kept);
          return stripeRetryLater();
        }
        await alertDuplicateSubscription(env, account.id, kept, cancelled, storedIsOlder ? sub?.customer : other.body?.customer, fetcher);
        applyId = kept;
        if (storedIsOlder) {
          state = otherState;
          keptSub = other.body;
        }
      }
    }
    // The customer comes from the kept subscription, so Manage never opens
    // a cancelled duplicate's portal.
    if (state && (await applyPlan(db, account.id, applyId, state)) && state.plan === "cloud") {
      const customer = keptSub?.customer;
      if (typeof customer === "string" && customer !== account.stripeCustomerId) await setStripeCustomer(db, account.id, customer);
    }
    return await received();
  } catch (err) {
    return backendError("handleBillingWebhook", err);
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
    const { account, device, phone } = authResult;
    if (device.status !== "pending" || !device.replacesDeviceId) {
      return Response.json({ error: "device not approved" }, { status: 403 });
    }
    if (phone || kindOf(device.platform) !== "computer") {
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
  fetcher: typeof fetch = fetch,
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
    await deleteAccount(db, env, authResult.account, fetcher);
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
    ctx.waitUntil(settleStaleHolds(tursoSqlExecutor(env), Date.now()).then((n) => console.log("ai stale holds settled", n)));
  },

  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    try {
      const url = new URL(request.url);
      if (url.pathname === "/ai/v1/messages" && request.method === "POST") {
        return await handleAiMessages(request, env, ctx, tursoSqlExecutor(env));
      }
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
      if (url.pathname === "/v1/account/plan" && request.method === "POST") {
        return await handleAccountPlan(request, env);
      }
      if (url.pathname === "/v1/billing/checkout" && request.method === "POST") {
        return await handleBillingCheckout(request, env);
      }
      if (url.pathname === "/v1/billing/portal" && request.method === "POST") {
        return await handleBillingPortal(request, env);
      }
      if (url.pathname === "/v1/ai/checkout" && request.method === "POST") {
        return await handleAiCheckout(request, env);
      }
      if (url.pathname === "/v1/ai/balance" && request.method === "POST") {
        return await handleAiBalance(request, env);
      }
      if (url.pathname === "/v1/ai/models" && request.method === "POST") {
        return await handleAiModels(request, env);
      }
      if (url.pathname === "/v1/billing/webhook" && request.method === "POST") {
        return await handleBillingWebhook(request, env);
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
