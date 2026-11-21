import { afterEach, describe, expect, it } from "vitest";
import { deleteAccountRows } from "../src/accounts";
import { realSqlExecutor as openRealSql } from "./testHelpers/realSqlExecutor";

let open: ReturnType<typeof openRealSql>[] = [];
afterEach(() => {
  for (const db of open) db.close();
  open = [];
});
/** A real-SQLite executor that is closed after the test. */
function realSqlExecutor() {
  const db = openRealSql();
  open.push(db);
  return db;
}

const ACCOUNT_INSERT =
  "INSERT INTO account (id, email, google_sub, data_db_name, data_db_url, created_at_unix_ms) VALUES (?, ?, ?, ?, ?, ?)";

describe("realSqlExecutor", () => {
  it("accepts SCHEMA_SQL, including the Plus allowance columns and tables", async () => {
    const db = realSqlExecutor();
    await db.execute(ACCOUNT_INSERT, ["a1", "a@x.test", "g1", "n", "u", 1]);
    const [row] = await db.execute(
      "SELECT plan_tier, ai_allowance_micros, ai_allowance_expires_ms FROM account WHERE id = ?",
      ["a1"],
    );
    expect(row).toEqual({ plan_tier: null, ai_allowance_micros: 0, ai_allowance_expires_ms: null });
    await db.execute("INSERT INTO server_setting (key, value) VALUES (?, ?)", ["plus_allowance_micros", "6000000"]);
    await db.execute("INSERT INTO ai_hold (id, account_id, micros, created_at_unix_ms) VALUES (?, ?, ?, ?)", ["h1", "a1", 5, 1]);
    const [hold] = await db.execute("SELECT from_allowance_micros, allowance_expires_ms FROM ai_hold WHERE id = ?", ["h1"]);
    expect(hold).toEqual({ from_allowance_micros: 0, allowance_expires_ms: null });
  });

  it("runs batch as one transaction: an error in the second statement rolls back the first", async () => {
    const db = realSqlExecutor();
    await expect(
      db.batch([
        { sql: ACCOUNT_INSERT, params: ["a1", "a@x.test", "g1", "n", "u", 1] },
        { sql: "INSERT INTO account (id) VALUES (?)", params: ["a2"] }, // NOT NULL violation
      ]),
    ).rejects.toThrow();
    expect(await db.execute("SELECT id FROM account", [])).toEqual([]);
    // And a good batch commits and returns each statement's rows.
    const out = await db.batch([
      { sql: ACCOUNT_INSERT, params: ["a1", "a@x.test", "g1", "n", "u", 1] },
      { sql: "SELECT id FROM account", params: [] },
    ]);
    expect(out[1]).toEqual([{ id: "a1" }]);
  });

  it("deleteAccountRows removes the account's allowance grant rows and only theirs", async () => {
    const db = realSqlExecutor();
    const grant = "INSERT INTO ai_allowance_grant (grant_key, account_id, micros, period_end_unix_ms, created_at_unix_ms) VALUES (?, ?, ?, ?, ?)";
    await db.execute(ACCOUNT_INSERT, ["a1", "a@x.test", "g1", "n", "u", 1]);
    await db.execute(grant, ["in_1", "a1", 6000000, 10, 1]);
    await db.execute(grant, ["in_2", "a2", 6000000, 10, 1]);
    await deleteAccountRows(db, "a1");
    expect(await db.execute("SELECT grant_key FROM ai_allowance_grant", [])).toEqual([{ grant_key: "in_2" }]);
    expect(await db.execute("SELECT id FROM account", [])).toEqual([]);
  });
});
