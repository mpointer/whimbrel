import type { SqlExecutor, SqlRow } from "../../src/accounts";

/** Minimal in-memory fake covering exactly the queries accounts.ts issues
 * — not a general SQL engine, just enough to test real accounts.ts query
 * logic (and callers built on top of it) without a live Turso database.
 * Shared between accounts.test.ts and index.test.ts so there is one
 * source of truth for "what accounts.ts's SQL actually looks like". */
export class FakeSqlExecutor implements SqlExecutor {
  private accounts = new Map<
    string,
    { id: string; email: string; googleSub: string; dataDbName: string; dataDbUrl: string }
  >();
  private devices = new Map<string, SqlRow>();

  async execute(sql: string, params: unknown[]): Promise<SqlRow[]> {
    const q = sql.replace(/\s+/g, " ").trim();

    if (q.startsWith("SELECT id, email, google_sub, data_db_name, data_db_url FROM account WHERE google_sub = ?")) {
      const [googleSub] = params as [string];
      const found = [...this.accounts.values()].find((a) => a.googleSub === googleSub);
      return found
        ? [
            {
              id: found.id,
              email: found.email,
              google_sub: found.googleSub,
              data_db_name: found.dataDbName,
              data_db_url: found.dataDbUrl,
            },
          ]
        : [];
    }
    if (q.startsWith("INSERT OR IGNORE INTO account")) {
      const [id, email, googleSub, dataDbName, dataDbUrl] = params as [string, string, string, string, string];
      const alreadyExists = [...this.accounts.values()].some((a) => a.googleSub === googleSub);
      if (!alreadyExists) {
        this.accounts.set(id, { id, email, googleSub, dataDbName, dataDbUrl });
      }
      return [];
    }
    // NOTE: this check must come BEFORE the listDevicesForAccount check
    // below, since that query's text ("...WHERE account_id = ?") is a
    // string prefix of this one's ("...WHERE account_id = ? AND public_key = ?").
    if (
      q.startsWith(
        "SELECT id, account_id, public_key, name, platform, status, enrolled_at_unix_ms, revoked_at_unix_ms FROM device WHERE account_id = ? AND public_key = ?",
      )
    ) {
      const [accountId, publicKey] = params as [string, string];
      const found = [...this.devices.values()].find(
        (d) => d.account_id === accountId && d.public_key === publicKey,
      );
      return found ? [found] : [];
    }
    if (
      q.startsWith(
        "SELECT id, account_id, public_key, name, platform, status, enrolled_at_unix_ms, revoked_at_unix_ms FROM device WHERE id = ?",
      )
    ) {
      const [deviceId] = params as [string];
      const found = this.devices.get(deviceId);
      return found ? [found] : [];
    }
    if (
      q.startsWith(
        "SELECT id, account_id, public_key, name, platform, status, enrolled_at_unix_ms, revoked_at_unix_ms FROM device WHERE account_id = ?",
      )
    ) {
      const [accountId] = params as [string];
      return [...this.devices.values()].filter((d) => d.account_id === accountId);
    }
    // NOTE: this check must come BEFORE the plain count check below, since
    // the plain count query text is a string prefix of this one.
    if (q.startsWith("SELECT COUNT(*) as count FROM device WHERE account_id = ? AND status != 'revoked'")) {
      const [accountId] = params as [string];
      const count = [...this.devices.values()].filter(
        (d) => d.account_id === accountId && d.status !== "revoked",
      ).length;
      return [{ count }];
    }
    if (q.startsWith("SELECT COUNT(*) as count FROM device WHERE account_id = ?")) {
      const [accountId] = params as [string];
      const count = [...this.devices.values()].filter((d) => d.account_id === accountId).length;
      return [{ count }];
    }
    if (q.startsWith("INSERT INTO device")) {
      const [id, accountId, publicKey, status, enrolledAtUnixMs] = params as [
        string,
        string,
        string,
        string,
        number,
      ];
      this.devices.set(id, {
        id,
        account_id: accountId,
        public_key: publicKey,
        name: null,
        platform: null,
        status,
        enrolled_at_unix_ms: enrolledAtUnixMs,
        revoked_at_unix_ms: null,
      });
      return [];
    }
    if (q.startsWith("UPDATE device SET status = 'approved' WHERE id = ?")) {
      const [deviceId] = params as [string];
      const device = this.devices.get(deviceId);
      if (device) {
        this.devices.set(deviceId, { ...device, status: "approved" });
      }
      return [];
    }
    if (q.startsWith("UPDATE device SET status = 'revoked', revoked_at_unix_ms = ? WHERE id = ?")) {
      const [revokedAtUnixMs, deviceId] = params as [number, string];
      const device = this.devices.get(deviceId);
      if (device) {
        this.devices.set(deviceId, { ...device, status: "revoked", revoked_at_unix_ms: revokedAtUnixMs });
      }
      return [];
    }
    if (q.startsWith("UPDATE device SET name = ? WHERE id = ?")) {
      const [name, deviceId] = params as [string, string];
      const device = this.devices.get(deviceId);
      if (device) {
        this.devices.set(deviceId, { ...device, name });
      }
      return [];
    }
    throw new Error(`FakeSqlExecutor: unrecognized query: ${q}`);
  }
}
