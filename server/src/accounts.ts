export interface SqlRow {
  [column: string]: unknown;
}

export interface SqlExecutor {
  execute(sql: string, params: unknown[]): Promise<SqlRow[]>;
}

/** Not auto-run by this module. Run once against the real Turso
 * control-plane database as a manual setup step (e.g. via `turso db
 * shell`), not on every Worker cold start — repeated DDL execution on
 * every request is unnecessary overhead for a schema that essentially
 * never changes. This schema is for the CONTROL-PLANE database only
 * (account/device registry) — it holds no sync payload data; each
 * account's actual encrypted rows live in that account's own data-plane
 * database, provisioned separately (see `turso.ts`'s
 * `createAccountDatabase`). */
export const SCHEMA_SQL = `
CREATE TABLE IF NOT EXISTS account (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL,
  google_sub TEXT NOT NULL UNIQUE,
  data_db_name TEXT NOT NULL,
  data_db_url TEXT NOT NULL,
  created_at_unix_ms INTEGER NOT NULL,
  last_seen_unix_ms INTEGER,
  warned_at_unix_ms INTEGER,
  unlimited INTEGER NOT NULL DEFAULT 0,
  reset_notice_pending_unix_ms INTEGER,
  deleting_since_unix_ms INTEGER
);

CREATE TABLE IF NOT EXISTS device (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES account(id),
  public_key TEXT NOT NULL,
  name TEXT,
  platform TEXT,
  status TEXT NOT NULL,
  enrolled_at_unix_ms INTEGER NOT NULL,
  revoked_at_unix_ms INTEGER,
  last_seen_unix_ms INTEGER,
  replaces_device_id TEXT,
  resetting_since_unix_ms INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS device_account_public_key_idx ON device(account_id, public_key);

CREATE TABLE IF NOT EXISTS waitlist (
  account_id TEXT NOT NULL,
  wants TEXT NOT NULL,
  platform TEXT,
  created_at_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS retired_database (
  name TEXT NOT NULL,
  account_id TEXT NOT NULL,
  retired_at_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS account_creation (
  ip_bucket TEXT NOT NULL,
  created_at_unix_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS account_creation_ip ON account_creation(ip_bucket, created_at_unix_ms);
`;

export type DevicePlatform = "mac" | "windows" | "ios";

/** An iOS Google audience always wins; otherwise only "mac" or "windows"
 * from the body is trusted. Old builds that send nothing count as a Mac. */
export function platformFor(bodyPlatform: unknown, isIos: boolean): DevicePlatform {
  if (isIos) return "ios";
  return bodyPlatform === "windows" ? "windows" : "mac";
}

export interface AccountRecord {
  id: string;
  email: string;
  googleSub: string;
  dataDbName: string;
  dataDbUrl: string;
  /** Set while the account is being deleted; every route refuses it then. */
  deletingSinceUnixMs?: number | null;
}

/** Thrown when a route touches an account that is claimed for deletion. */
export class AccountDeletingError extends Error {}

const ACCOUNT_COLUMNS = "id, email, google_sub, data_db_name, data_db_url, deleting_since_unix_ms";

/** "resetting" is a pending swap device partway through a fresh-start
 * reset: it is treated like "pending" everywhere until the reset finishes. */
export type DeviceStatus = "approved" | "pending" | "resetting" | "revoked";

export interface DeviceRecord {
  id: string;
  accountId: string;
  publicKey: string;
  name: string | null;
  platform: string | null;
  status: DeviceStatus;
  enrolledAtUnixMs: number;
  revokedAtUnixMs: number | null;
  /** Set on a pending swap: the device this one revokes when approved. */
  replacesDeviceId?: string | null;
}

const DEVICE_COLUMNS =
  "id, account_id, public_key, name, platform, status, enrolled_at_unix_ms, revoked_at_unix_ms, replaces_device_id";

/** Returns the account for `googleSub` if it has signed in before, or
 * `null` if it hasn't. Read-only — never creates anything. Callers use
 * this to decide whether a new data-plane database needs provisioning
 * before calling `getOrCreateAccount`. */
export async function findAccountByGoogleSub(db: SqlExecutor, googleSub: string): Promise<AccountRecord | null> {
  const rows = await db.execute(`SELECT ${ACCOUNT_COLUMNS} FROM account WHERE google_sub = ?`, [googleSub]);
  if (rows.length === 0) return null;
  return rowToAccount(rows[0]);
}

/** Returns the existing account if `googleSub` has signed in before
 * (ignoring `dataDbNameForNewAccount`/`dataDbUrlForNewAccount` in that case
 * — the account's original data-plane database is never changed),
 * otherwise creates a new account with those values and returns it. Uses
 * INSERT OR IGNORE plus a re-select rather than a bare INSERT: two
 * concurrent first sign-ins for the same `googleSub` would otherwise race
 * on the UNIQUE constraint — with IGNORE, exactly one insert wins and
 * both callers converge on the same account (including the same
 * `dataDbName`/`dataDbUrl` — the winner's, not necessarily the caller's
 * own candidate). */
export async function getOrCreateAccount(
  db: SqlExecutor,
  email: string,
  googleSub: string,
  nowUnixMs: number,
  dataDbNameForNewAccount: string,
  dataDbUrlForNewAccount: string,
): Promise<AccountRecord> {
  const existing = await findAccountByGoogleSub(db, googleSub);
  if (existing) {
    return existing;
  }
  const id = crypto.randomUUID();
  await db.execute(
    "INSERT OR IGNORE INTO account (id, email, google_sub, data_db_name, data_db_url, created_at_unix_ms) VALUES (?, ?, ?, ?, ?, ?)",
    [id, email, googleSub, dataDbNameForNewAccount, dataDbUrlForNewAccount, nowUnixMs],
  );
  const created = await findAccountByGoogleSub(db, googleSub);
  if (!created) {
    throw new Error("account insert did not materialize");
  }
  return created;
}

export async function findDeviceByPublicKey(
  db: SqlExecutor,
  accountId: string,
  publicKey: string,
): Promise<DeviceRecord | null> {
  const rows = await db.execute(
    `SELECT ${DEVICE_COLUMNS} FROM device WHERE account_id = ? AND public_key = ?`,
    [accountId, publicKey],
  );
  if (rows.length === 0) return null;
  return rowToDevice(rows[0]);
}

/** Finds a device by its own row id, regardless of which account it
 * belongs to — callers (the device-approval route) are responsible for
 * checking `accountId` against the caller's own account before trusting
 * this result for anything account-scoped. */
export async function findDeviceById(db: SqlExecutor, deviceId: string): Promise<DeviceRecord | null> {
  const rows = await db.execute(
    `SELECT ${DEVICE_COLUMNS} FROM device WHERE id = ?`,
    [deviceId],
  );
  if (rows.length === 0) return null;
  return rowToDevice(rows[0]);
}

/** Lists every device on the account in insertion order (SQLite rowid),
 * including revoked ones — callers filter by `status` themselves if they
 * only want pending or active devices. Insertion order is what the
 * device-limit race check relies on: the database serialises inserts, so
 * unlike a client clock it can't disagree with who actually came first. */
export async function listDevicesForAccount(db: SqlExecutor, accountId: string): Promise<DeviceRecord[]> {
  const rows = await db.execute(
    `SELECT ${DEVICE_COLUMNS} FROM device WHERE account_id = ? ORDER BY rowid`,
    [accountId],
  );
  return rows.map(rowToDevice);
}

/** Counts every device row for the account, including revoked ones. */
export async function countDevicesForAccount(db: SqlExecutor, accountId: string): Promise<number> {
  const rows = await db.execute("SELECT COUNT(*) as count FROM device WHERE account_id = ?", [accountId]);
  return Number(rows[0].count);
}

/** Counts only non-revoked device rows for the account. This is the count
 * that decides whether a newly-enrolling device is the account's "first"
 * device (auto-approved) — using `countDevicesForAccount` (which includes
 * revoked rows) here would permanently lock an account out once its only
 * device is revoked, since the count would never return to zero. */
export async function countActiveDevicesForAccount(db: SqlExecutor, accountId: string): Promise<number> {
  const rows = await db.execute(
    "SELECT COUNT(*) as count FROM device WHERE account_id = ? AND status != 'revoked'",
    [accountId],
  );
  return Number(rows[0].count);
}

/** Enrolls a new device at the given status. Callers decide the status:
 * the account's first device is "approved" (it generates the group key
 * itself), every later device starts "pending" until an already-approved
 * device hands it the group key. */
export async function enrollDevice(
  db: SqlExecutor,
  accountId: string,
  publicKey: string,
  status: DeviceStatus,
  nowUnixMs: number,
  platform: DevicePlatform = "mac",
): Promise<DeviceRecord> {
  const id = crypto.randomUUID();
  await db.execute(
    "INSERT INTO device (id, account_id, public_key, status, enrolled_at_unix_ms, platform) VALUES (?, ?, ?, ?, ?, ?)",
    [id, accountId, publicKey, status, nowUnixMs, platform],
  );
  return {
    id,
    accountId,
    publicKey,
    name: null,
    platform,
    status,
    enrolledAtUnixMs: nowUnixMs,
    revokedAtUnixMs: null,
    replacesDeviceId: null,
  };
}

/** Flips a pending device's status to approved. Does not check the
 * device's current status or account ownership — callers (the Worker
 * route) are responsible for verifying the device is pending and belongs
 * to the approving device's account before calling this. */
export async function approveDevice(db: SqlExecutor, deviceId: string): Promise<boolean> {
  // Only pending (or already approved: "Re-send Key") rows; a device that was
  // revoked or claimed for a reset in the meantime is left alone.
  const rows = await db.execute(
    "UPDATE device SET status = 'approved' WHERE id = ? AND status IN ('pending', 'approved') RETURNING id",
    [deviceId],
  );
  return rows.length === 1;
}

/** Marks a device revoked and records when. Does not check the device's
 * current status or account ownership — callers (the Worker route) are
 * responsible for verifying the target belongs to the revoking device's
 * account, and isn't already revoked, before calling this. */
export async function revokeDevice(db: SqlExecutor, deviceId: string, nowUnixMs: number): Promise<void> {
  await db.execute("UPDATE device SET status = 'revoked', revoked_at_unix_ms = ? WHERE id = ?", [nowUnixMs, deviceId]);
}

/** Sets a device's name. Does not check account ownership — callers (the
 * Worker route) are responsible for only ever calling this with the
 * authenticated caller's own device id, never an arbitrary target. */
export async function setDeviceName(db: SqlExecutor, deviceId: string, name: string): Promise<void> {
  await db.execute("UPDATE device SET name = ? WHERE id = ?", [name, deviceId]);
}

/** Marks a pending device as a swap for `replacesDeviceId` (or clears it with null). */
export async function setReplacesDevice(db: SqlExecutor, deviceId: string, replacesDeviceId: string | null): Promise<void> {
  await db.execute("UPDATE device SET replaces_device_id = ? WHERE id = ?", [replacesDeviceId, deviceId]);
}

/** Removes a device row outright. Only used to undo an enrolment that lost the device-limit race. */
export async function deleteDevice(db: SqlExecutor, deviceId: string): Promise<void> {
  await db.execute("DELETE FROM device WHERE id = ?", [deviceId]);
}

/** Points the account at a new data-plane database after a fresh-start
 * reset, and flags the reset notice email for the cron to send. Returns
 * false, changing nothing, when the account is claimed for deletion (or gone). */
export async function setAccountDatabase(
  db: SqlExecutor,
  accountId: string,
  name: string,
  url: string,
  nowUnixMs: number,
): Promise<boolean> {
  const rows = await db.execute(
    "UPDATE account SET data_db_name = ?, data_db_url = ?, reset_notice_pending_unix_ms = ? WHERE id = ? AND deleting_since_unix_ms IS NULL RETURNING id",
    [name, url, nowUnixMs, accountId],
  );
  return rows.length === 1;
}

/** Takes one of `limit` account-creation slots for an IP bucket in the
 * window after `sinceUnixMs`. A single statement, so concurrent first
 * sign-ins can't all see the last free slot (KV get-then-put could).
 * ponytail: one row per account ever created, never pruned; prune in the
 * cron if the table ever grows large enough to matter. */
export async function reserveAccountCreation(
  db: SqlExecutor,
  ipBucket: string,
  limit: number,
  sinceUnixMs: number,
  nowUnixMs: number,
): Promise<boolean> {
  const rows = await db.execute(
    "INSERT INTO account_creation (ip_bucket, created_at_unix_ms) SELECT ?, ? WHERE (SELECT COUNT(*) FROM account_creation WHERE ip_bucket = ? AND created_at_unix_ms > ?) < ? RETURNING ip_bucket",
    [ipBucket, nowUnixMs, ipBucket, sinceUnixMs, limit],
  );
  return rows.length === 1;
}

/** Queues a data-plane database for deletion by the cron (after 30 days). */
export async function retireDatabase(db: SqlExecutor, accountId: string, name: string, nowUnixMs: number): Promise<void> {
  await db.execute("INSERT INTO retired_database (name, account_id, retired_at_unix_ms) VALUES (?, ?, ?)", [
    name,
    accountId,
    nowUnixMs,
  ]);
}

/** Takes back `retireDatabase` for a database that is still in use (a reset
 * that failed before switching the account). Matches the exact row this
 * reset wrote, so another reset's row is never removed. */
export async function unretireDatabase(
  db: SqlExecutor,
  accountId: string,
  name: string,
  retiredAtUnixMs: number,
): Promise<void> {
  await db.execute("DELETE FROM retired_database WHERE account_id = ? AND name = ? AND retired_at_unix_ms = ?", [
    accountId,
    name,
    retiredAtUnixMs,
  ]);
}

/** Atomically claims a pending swap device for a reset by moving it to
 * "resetting". Returns false when another request already claimed it, or the
 * account is claimed for deletion (the deletion claim in turn refuses while a
 * device is resetting, so the two exclude each other). The claim time lets
 * the cron hand back a claim whose reset crashed. */
export async function claimSwapForReset(db: SqlExecutor, deviceId: string, nowUnixMs = Date.now()): Promise<boolean> {
  const rows = await db.execute(
    "UPDATE device SET status = 'resetting', resetting_since_unix_ms = ? WHERE id = ? AND status = 'pending' AND replaces_device_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM account WHERE account.id = device.account_id AND account.deleting_since_unix_ms IS NOT NULL) RETURNING id",
    [nowUnixMs, deviceId],
  );
  return rows.length === 1;
}

/** Undoes `claimSwapForReset` when a later reset step fails, so the client
 * can retry. Only a still-resetting device goes back: one revoked in the
 * meantime stays revoked. */
export async function unclaimSwapForReset(db: SqlExecutor, deviceId: string): Promise<void> {
  await db.execute(
    "UPDATE device SET status = 'pending', resetting_since_unix_ms = NULL WHERE id = ? AND status = 'resetting'",
    [deviceId],
  );
}

/** The cron's version of `unclaimSwapForReset`: only a claim still older
 * than `beforeUnixMs`, so a claim retaken since the device was listed stays. */
export async function recoverStuckReset(db: SqlExecutor, deviceId: string, beforeUnixMs: number): Promise<void> {
  await db.execute(
    "UPDATE device SET status = 'pending', resetting_since_unix_ms = NULL WHERE id = ? AND status = 'resetting' AND resetting_since_unix_ms < ?",
    [deviceId, beforeUnixMs],
  );
}

/** Finishes a reset: approves the still-resetting device and clears its swap
 * link. Returns false if the device was revoked while the reset ran. */
export async function finishReset(db: SqlExecutor, deviceId: string): Promise<boolean> {
  const rows = await db.execute(
    "UPDATE device SET status = 'approved', replaces_device_id = NULL, resetting_since_unix_ms = NULL WHERE id = ? AND status = 'resetting' RETURNING id",
    [deviceId],
  );
  return rows.length === 1;
}

/** Approved devices, other than `exceptDeviceId`, seen since `sinceUnixMs`. */
export async function recentlySeenApprovedDevices(
  db: SqlExecutor,
  accountId: string,
  exceptDeviceId: string,
  sinceUnixMs: number,
): Promise<{ id: string; name: string | null; platform: string | null }[]> {
  const rows = await db.execute(
    "SELECT id, name, platform FROM device WHERE account_id = ? AND status = 'approved' AND id != ? AND last_seen_unix_ms >= ?",
    [accountId, exceptDeviceId, sinceUnixMs],
  );
  return rows.map((r) => ({
    id: r.id as string,
    name: r.name == null ? null : (r.name as string),
    platform: r.platform == null ? null : (r.platform as string),
  }));
}

/** `wants` is stored as a JSON array of the allowed waitlist values. Returns
 * false, inserting nothing, when the account is claimed for deletion (or gone). */
export async function addToWaitlist(
  db: SqlExecutor,
  accountId: string,
  wants: string[],
  platform: DevicePlatform,
  nowUnixMs: number,
): Promise<boolean> {
  const rows = await db.execute(
    "INSERT INTO waitlist (account_id, wants, platform, created_at_unix_ms) SELECT ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM account WHERE id = ? AND deleting_since_unix_ms IS NULL) RETURNING account_id",
    [accountId, JSON.stringify(wants), platform, nowUnixMs, accountId],
  );
  return rows.length === 1;
}

/** Records an authenticated contact: stamps the account (clearing any
 * pending inactivity warning) and the calling device. Throws
 * AccountDeletingError for an account claimed for deletion, or gone. The
 * account row is written first: it serialises with the deletion claim, which
 * needs the warning this clears. */
export async function touchLastSeen(
  db: SqlExecutor,
  accountId: string,
  deviceId: string,
  nowUnixMs: number,
): Promise<void> {
  const rows = await db.execute(
    "UPDATE account SET last_seen_unix_ms = ?, warned_at_unix_ms = NULL WHERE id = ? AND deleting_since_unix_ms IS NULL RETURNING id",
    [nowUnixMs, accountId],
  );
  if (rows.length === 0) throw new AccountDeletingError();
  await db.execute("UPDATE device SET last_seen_unix_ms = ? WHERE id = ?", [nowUnixMs, deviceId]);
}

// ---- account deletion and the inactivity cron ----

/** Claims an inactive account for deletion, re-checking the inactivity
 * conditions in the same statement so a contact since it was read wins.
 * Both claims refuse while a device is resetting (a reset could otherwise
 * switch the account to a database the deletion never sees). */
export async function claimInactiveForDeletion(
  db: SqlExecutor,
  accountId: string,
  nowUnixMs: number,
  warnedBeforeUnixMs: number,
): Promise<boolean> {
  const rows = await db.execute(
    "UPDATE account SET deleting_since_unix_ms = ? WHERE id = ? AND deleting_since_unix_ms IS NULL AND warned_at_unix_ms < ? AND last_seen_unix_ms < warned_at_unix_ms AND NOT EXISTS (SELECT 1 FROM device WHERE device.account_id = account.id AND device.status = 'resetting') RETURNING id",
    [nowUnixMs, accountId, warnedBeforeUnixMs],
  );
  return rows.length === 1;
}

/** Claims an account for a self-serve deletion. */
export async function claimForDeletion(db: SqlExecutor, accountId: string, nowUnixMs: number): Promise<boolean> {
  const rows = await db.execute(
    "UPDATE account SET deleting_since_unix_ms = ? WHERE id = ? AND deleting_since_unix_ms IS NULL AND NOT EXISTS (SELECT 1 FROM device WHERE device.account_id = account.id AND device.status = 'resetting') RETURNING id",
    [nowUnixMs, accountId],
  );
  return rows.length === 1;
}

export async function findAccountById(db: SqlExecutor, accountId: string): Promise<AccountRecord | null> {
  const rows = await db.execute(`SELECT ${ACCOUNT_COLUMNS} FROM account WHERE id = ?`, [accountId]);
  return rows.length === 0 ? null : rowToAccount(rows[0]);
}

/** Accounts claimed for deletion before `beforeUnixMs` whose deletion never finished. */
export async function staleDeletionClaims(db: SqlExecutor, beforeUnixMs: number): Promise<AccountRecord[]> {
  const rows = await db.execute(`SELECT ${ACCOUNT_COLUMNS} FROM account WHERE deleting_since_unix_ms < ?`, [beforeUnixMs]);
  return rows.map(rowToAccount);
}

/** True while some account uses `name` as its data database. */
export async function isCurrentDatabase(db: SqlExecutor, name: string): Promise<boolean> {
  return (await db.execute("SELECT 1 FROM account WHERE data_db_name = ?", [name])).length > 0;
}

/** Retired databases of one account that no account currently uses. */
export async function retiredDatabasesForAccount(db: SqlExecutor, accountId: string): Promise<string[]> {
  const rows = await db.execute(
    "SELECT name FROM retired_database WHERE account_id = ? AND name NOT IN (SELECT data_db_name FROM account)",
    [accountId],
  );
  return rows.map((r) => r.name as string);
}

/** Retired databases older than `beforeUnixMs` that no account currently uses. */
export async function expiredRetiredDatabases(db: SqlExecutor, beforeUnixMs: number): Promise<string[]> {
  const rows = await db.execute(
    "SELECT name FROM retired_database WHERE retired_at_unix_ms < ? AND name NOT IN (SELECT data_db_name FROM account)",
    [beforeUnixMs],
  );
  return rows.map((r) => r.name as string);
}

export async function forgetRetiredDatabase(db: SqlExecutor, name: string): Promise<void> {
  await db.execute("DELETE FROM retired_database WHERE name = ?", [name]);
}

/** Removes the account's registry rows. The account row goes first: if a
 * later delete fails, the leftovers are orphans nothing reads, whereas an
 * account left with no devices would auto-approve the next sign-in onto a
 * database that no longer exists. */
export async function deleteAccountRows(db: SqlExecutor, accountId: string): Promise<void> {
  await db.execute("DELETE FROM account WHERE id = ?", [accountId]);
  await db.execute("DELETE FROM device WHERE account_id = ?", [accountId]);
  await db.execute("DELETE FROM waitlist WHERE account_id = ?", [accountId]);
  await db.execute("DELETE FROM retired_database WHERE account_id = ?", [accountId]);
}

/** Accounts from before last-seen existed count as seen now, never as idle. */
export async function backfillLastSeen(db: SqlExecutor, nowUnixMs: number): Promise<void> {
  await db.execute("UPDATE account SET last_seen_unix_ms = ? WHERE last_seen_unix_ms IS NULL", [nowUnixMs]);
}

/** Devices claimed for a reset before `beforeUnixMs` (the reset crashed). */
export async function stuckResettingDevices(db: SqlExecutor, beforeUnixMs: number): Promise<string[]> {
  const rows = await db.execute("SELECT id FROM device WHERE status = 'resetting' AND resetting_since_unix_ms < ?", [
    beforeUnixMs,
  ]);
  return rows.map((r) => r.id as string);
}

export async function pendingResetNotices(
  db: SqlExecutor,
): Promise<{ id: string; email: string; resetAtUnixMs: number }[]> {
  const rows = await db.execute(
    "SELECT id, email, reset_notice_pending_unix_ms FROM account WHERE reset_notice_pending_unix_ms IS NOT NULL",
    [],
  );
  return rows.map((r) => ({ id: r.id as string, email: r.email as string, resetAtUnixMs: Number(r.reset_notice_pending_unix_ms) }));
}

/** Clears the notice only if no newer reset has replaced it since it was read. */
export async function clearResetNotice(db: SqlExecutor, accountId: string, resetAtUnixMs: number): Promise<void> {
  await db.execute("UPDATE account SET reset_notice_pending_unix_ms = NULL WHERE id = ? AND reset_notice_pending_unix_ms = ?", [
    accountId,
    resetAtUnixMs,
  ]);
}

/** Unwarned accounts last seen before `seenBeforeUnixMs`. */
export async function accountsToWarn(db: SqlExecutor, seenBeforeUnixMs: number): Promise<{ id: string; email: string }[]> {
  const rows = await db.execute("SELECT id, email FROM account WHERE warned_at_unix_ms IS NULL AND last_seen_unix_ms < ?", [
    seenBeforeUnixMs,
  ]);
  return rows.map((r) => ({ id: r.id as string, email: r.email as string }));
}

/** Records a sent warning, unless a device made contact since the account was
 * read (which would leave a warning that can never lead to deletion). */
export async function markWarned(db: SqlExecutor, accountId: string, nowUnixMs: number, seenBeforeUnixMs: number): Promise<void> {
  await db.execute(
    "UPDATE account SET warned_at_unix_ms = ? WHERE id = ? AND warned_at_unix_ms IS NULL AND last_seen_unix_ms < ?",
    [nowUnixMs, accountId, seenBeforeUnixMs],
  );
}

/** Accounts warned before `warnedBeforeUnixMs` with no contact since the warning. */
export async function accountsToDelete(db: SqlExecutor, warnedBeforeUnixMs: number): Promise<AccountRecord[]> {
  const rows = await db.execute(
    `SELECT ${ACCOUNT_COLUMNS} FROM account WHERE warned_at_unix_ms < ? AND last_seen_unix_ms < warned_at_unix_ms`,
    [warnedBeforeUnixMs],
  );
  return rows.map(rowToAccount);
}

/** Every pending or resetting swap device, across all accounts. */
export async function swapDevices(
  db: SqlExecutor,
): Promise<{ id: string; status: DeviceStatus; replacesDeviceId: string; enrolledAtUnixMs: number }[]> {
  const rows = await db.execute(
    "SELECT id, status, replaces_device_id, enrolled_at_unix_ms FROM device WHERE status IN ('pending', 'resetting') AND replaces_device_id IS NOT NULL",
    [],
  );
  return rows.map((r) => ({
    id: r.id as string,
    status: r.status as DeviceStatus,
    replacesDeviceId: r.replaces_device_id as string,
    enrolledAtUnixMs: Number(r.enrolled_at_unix_ms),
  }));
}

/** Deletes a device only while it is still pending (not approved or claimed
 * meanwhile) and no live swap has come to point at it since it was listed. */
export async function deletePendingDevice(db: SqlExecutor, deviceId: string): Promise<void> {
  await db.execute(
    "DELETE FROM device WHERE id = ? AND status = 'pending' AND NOT EXISTS (SELECT 1 FROM device d2 WHERE d2.replaces_device_id = device.id AND d2.status IN ('pending', 'resetting'))",
    [deviceId],
  );
}

function rowToAccount(row: SqlRow): AccountRecord {
  return {
    id: row.id as string,
    email: row.email as string,
    googleSub: row.google_sub as string,
    dataDbName: row.data_db_name as string,
    dataDbUrl: row.data_db_url as string,
    deletingSinceUnixMs: row.deleting_since_unix_ms == null ? null : Number(row.deleting_since_unix_ms),
  };
}

function rowToDevice(row: SqlRow): DeviceRecord {
  return {
    id: row.id as string,
    accountId: row.account_id as string,
    publicKey: row.public_key as string,
    name: row.name == null ? null : (row.name as string),
    platform: row.platform == null ? null : (row.platform as string),
    status: row.status as DeviceStatus,
    enrolledAtUnixMs: Number(row.enrolled_at_unix_ms),
    revokedAtUnixMs: row.revoked_at_unix_ms == null ? null : Number(row.revoked_at_unix_ms),
    replacesDeviceId: row.replaces_device_id == null ? null : (row.replaces_device_id as string),
  };
}
