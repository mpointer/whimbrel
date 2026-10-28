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
  created_at_unix_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS device (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES account(id),
  public_key TEXT NOT NULL,
  name TEXT,
  platform TEXT,
  status TEXT NOT NULL,
  enrolled_at_unix_ms INTEGER NOT NULL,
  revoked_at_unix_ms INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS device_account_public_key_idx ON device(account_id, public_key);
`;

export interface AccountRecord {
  id: string;
  email: string;
  googleSub: string;
  dataDbName: string;
  dataDbUrl: string;
}

export type DeviceStatus = "approved" | "pending" | "revoked";

export interface DeviceRecord {
  id: string;
  accountId: string;
  publicKey: string;
  name: string | null;
  platform: string | null;
  status: DeviceStatus;
  enrolledAtUnixMs: number;
  revokedAtUnixMs: number | null;
}

/** Returns the account for `googleSub` if it has signed in before, or
 * `null` if it hasn't. Read-only — never creates anything. Callers use
 * this to decide whether a new data-plane database needs provisioning
 * before calling `getOrCreateAccount`. */
export async function findAccountByGoogleSub(db: SqlExecutor, googleSub: string): Promise<AccountRecord | null> {
  const rows = await db.execute(
    "SELECT id, email, google_sub, data_db_name, data_db_url FROM account WHERE google_sub = ?",
    [googleSub],
  );
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
    "SELECT id, account_id, public_key, name, platform, status, enrolled_at_unix_ms, revoked_at_unix_ms FROM device WHERE account_id = ? AND public_key = ?",
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
    "SELECT id, account_id, public_key, name, platform, status, enrolled_at_unix_ms, revoked_at_unix_ms FROM device WHERE id = ?",
    [deviceId],
  );
  if (rows.length === 0) return null;
  return rowToDevice(rows[0]);
}

/** Lists every device on the account, in no particular order, including
 * revoked ones — callers filter by `status` themselves if they only want
 * pending or active devices. */
export async function listDevicesForAccount(db: SqlExecutor, accountId: string): Promise<DeviceRecord[]> {
  const rows = await db.execute(
    "SELECT id, account_id, public_key, name, platform, status, enrolled_at_unix_ms, revoked_at_unix_ms FROM device WHERE account_id = ?",
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
): Promise<DeviceRecord> {
  const id = crypto.randomUUID();
  await db.execute(
    "INSERT INTO device (id, account_id, public_key, status, enrolled_at_unix_ms) VALUES (?, ?, ?, ?, ?)",
    [id, accountId, publicKey, status, nowUnixMs],
  );
  return {
    id,
    accountId,
    publicKey,
    name: null,
    platform: null,
    status,
    enrolledAtUnixMs: nowUnixMs,
    revokedAtUnixMs: null,
  };
}

/** Flips a pending device's status to approved. Does not check the
 * device's current status or account ownership — callers (the Worker
 * route) are responsible for verifying the device is pending and belongs
 * to the approving device's account before calling this. */
export async function approveDevice(db: SqlExecutor, deviceId: string): Promise<void> {
  await db.execute("UPDATE device SET status = 'approved' WHERE id = ?", [deviceId]);
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

function rowToAccount(row: SqlRow): AccountRecord {
  return {
    id: row.id as string,
    email: row.email as string,
    googleSub: row.google_sub as string,
    dataDbName: row.data_db_name as string,
    dataDbUrl: row.data_db_url as string,
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
  };
}
