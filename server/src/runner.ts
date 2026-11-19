import type { AccountRecord, SqlExecutor } from "./accounts";
import { enrollRunnerDevice, findAccountById, hasApprovedNonCloudDevice, findDeviceByPublicKey, findRunnerDevicesByPublicKey, touchLastSeen } from "./accounts";
import { isEmailAllowed } from "./allowlist";
import { mintDeviceToken } from "./turso";
import { ipBucket, rateLimit, rateLimited } from "./rateLimit";
import type { KVNamespace } from "@cloudflare/workers-types";

const RUNNER_TOKEN_EXPIRATION = "1d";
/** Runner sign-ins per client IP bucket (IPv6 by /64) per window: the Google
 * sign-in route's 30 per 10 minutes. A runner signs in once per run (every 5
 * minutes in the runbook), so one host IP fits about 15 accounts with no
 * retries; raise it before one host serves more. */
const RUNNER_AUTH_LIMIT = 30;
const RUNNER_AUTH_WINDOW_SECONDS = 10 * 60;
const HASH_RE = /^[0-9a-f]{64}$/;

/** Entitlement hook for the cloud runner. Author-only phase: the account's
 * email must be in the RUNNER_EMAILS list. Billing entitlement is a later plan. */
export function canRunCloudRoutines(
  account: Pick<AccountRecord, "email">,
  env: { RUNNER_EMAILS?: string },
  _nowUnixMs: number,
): boolean {
  return isEmailAllowed(account.email, env.RUNNER_EMAILS);
}

/** Lowercase hex SHA-256 of a runner token. */
export async function hashRunnerToken(token: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** Compares two strings without returning early on the first differing byte. */
function constantTimeEqual(a: string, b: string): boolean {
  const x = new TextEncoder().encode(a);
  const y = new TextEncoder().encode(b);
  let diff = x.length ^ y.length;
  for (let i = 0; i < Math.max(x.length, y.length); i++) diff |= (x[i] ?? 0) ^ (y[i] ?? 0);
  return diff === 0;
}

interface RegisterBody {
  runnerPublicKey?: unknown;
  name?: unknown;
  runnerTokenHash?: unknown;
}

/** Creates the approved `cloud` device row for an already-authenticated
 * approver. Only the token's SHA-256 is accepted and stored. */
export async function registerRunner(
  db: SqlExecutor,
  env: { RUNNER_EMAILS?: string },
  account: AccountRecord,
  body: RegisterBody,
  now: number,
): Promise<Response> {
  if (!canRunCloudRoutines(account, env, now)) {
    return Response.json({ error: "cloud routines are not available for this account" }, { status: 403 });
  }
  if (typeof body.runnerPublicKey !== "string" || body.runnerPublicKey.length === 0 || body.runnerPublicKey.length > 512) {
    return Response.json({ error: "runnerPublicKey is required" }, { status: 400 });
  }
  if (typeof body.runnerTokenHash !== "string" || !HASH_RE.test(body.runnerTokenHash)) {
    return Response.json({ error: "runnerTokenHash must be a SHA-256 hex digest" }, { status: 400 });
  }
  if (typeof body.name !== "string") {
    return Response.json({ error: "name is required" }, { status: 400 });
  }
  const name = [...body.name.slice(0, 400).replace(/[\p{Cc}\p{Cf}\p{Cs}\p{Co}\p{Cn}\p{Zl}\p{Zp}]/gu, "").replace(/\s+/g, " ")]
    .slice(0, 100)
    .join("")
    .trim();
  if (name.length === 0) return Response.json({ error: "name is required" }, { status: 400 });

  if (await findDeviceByPublicKey(db, account.id, body.runnerPublicKey)) {
    return Response.json({ error: "device already exists" }, { status: 409 });
  }
  const device = await enrollRunnerDevice(db, account.id, body.runnerPublicKey, name, body.runnerTokenHash, now);
  return Response.json({ status: "approved", deviceId: device.id });
}

const unauthorized = () => Response.json({ error: "unauthorized" }, { status: 401 });

/** The runner has no Google sign-in. It proves itself with the token whose
 * SHA-256 was stored at registration. Returns a short-lived database token
 * only; no group key ever comes from the server. */
export async function handleRunnerAuth(
  request: Request,
  env: {
    RUNNER_EMAILS?: string;
    TURSO_PLATFORM_API_TOKEN: string;
    TURSO_ORG_SLUG: string;
    ENROLLMENT_HANDOFF: KVNamespace;
  },
  db: SqlExecutor,
  fetcher: typeof fetch = fetch,
): Promise<Response> {
  // Unauthenticated, so limited by client IP before any work. Skipped when the
  // header is absent (only outside Cloudflare), as account creation does.
  const ip = request.headers.get("CF-Connecting-IP");
  if (ip && !(await rateLimit(env.ENROLLMENT_HANDOFF, `runner-auth:${ipBucket(ip)}`, RUNNER_AUTH_LIMIT, RUNNER_AUTH_WINDOW_SECONDS))) {
    return rateLimited();
  }
  let body: { devicePublicKey?: unknown; runnerToken?: unknown };
  try {
    body = (await request.json()) as typeof body;
  } catch {
    return Response.json({ error: "invalid JSON body" }, { status: 400 });
  }
  if (
    typeof body.devicePublicKey !== "string" ||
    body.devicePublicKey.length === 0 ||
    typeof body.runnerToken !== "string" ||
    body.runnerToken.length === 0
  ) {
    return Response.json({ error: "devicePublicKey and runnerToken are required" }, { status: 400 });
  }
  try {
    const presented = await hashRunnerToken(body.runnerToken);
    const candidates = await findRunnerDevicesByPublicKey(db, body.devicePublicKey);
    // Check every candidate (and a dummy when there are none) so timing does not reveal which part failed.
    let match: (typeof candidates)[number] | null = null;
    for (const c of candidates) {
      if (constantTimeEqual(presented, c.runnerTokenHash ?? "") && c.runnerTokenHash !== null) match = c;
    }
    if (candidates.length === 0) constantTimeEqual(presented, "0".repeat(64));
    if (!match || match.device.platform !== "cloud" || match.device.status !== "approved") return unauthorized();

    const account = await findAccountById(db, match.device.accountId);
    if (!account) return unauthorized();
    if (account.deletingSinceUnixMs != null) return Response.json({ error: "account_deleting" }, { status: 503 });
    if (!canRunCloudRoutines(account, env, Date.now())) {
      return Response.json({ error: "cloud routines are not available for this account" }, { status: 403 });
    }
    // With no real device approved, a new first device would mint a new group
    // key and strand the runner on the old one; refuse until one exists.
    if (!(await hasApprovedNonCloudDevice(db, account.id))) return unauthorized();
    await touchLastSeen(db, account.id, match.device.id, Date.now());
    const tursoToken = await mintDeviceToken(
      { platformApiToken: env.TURSO_PLATFORM_API_TOKEN, organizationSlug: env.TURSO_ORG_SLUG, databaseName: account.dataDbName },
      RUNNER_TOKEN_EXPIRATION,
      fetcher,
    );
    return Response.json({ tursoToken, tursoDatabaseUrl: account.dataDbUrl });
  } catch (err) {
    console.error("handleRunnerAuth failed", err);
    return Response.json({ error: "sync backend unavailable" }, { status: 502 });
  }
}
