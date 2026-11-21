import { readFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { afterEach, expect, it } from "vitest";
import { SCHEMA_SQL } from "../src/accounts";
import { DAILY_CRON, YEARLY_GRANTS_CRON } from "../src/schedules";

// Ties SCHEMA_SQL to migrations 0006 and 0007: the schema before 0006
// (SCHEMA_SQL with the Plus columns and tables taken out) plus 0006 then
// 0007 must equal SCHEMA_SQL, table by table, column by column and index by
// index.

const MIGRATION = readFileSync(new URL("../migrations/0006_plus_allowance.sql", import.meta.url), "utf8");
const MIGRATION_0007 = readFileSync(new URL("../migrations/0007_plus_grant_line_bounds.sql", import.meta.url), "utf8");

/** Removes `part` from `sql`, failing loudly when it is not there, so a
 * change to SCHEMA_SQL cannot silently leave a Plus column behind. */
function cut(sql: string, part: string | RegExp, by = ""): string {
  const out = sql.replace(part, by);
  if (out === sql) throw new Error(`SCHEMA_SQL no longer contains ${String(part)}`);
  return out;
}

function schemaBefore0006(): string {
  let sql = SCHEMA_SQL;
  sql = cut(
    sql,
    "  ai_open_holds INTEGER NOT NULL DEFAULT 0,\n  plan_tier TEXT,\n  ai_allowance_micros INTEGER NOT NULL DEFAULT 0,\n  ai_allowance_expires_ms INTEGER\n",
    "  ai_open_holds INTEGER NOT NULL DEFAULT 0\n",
  );
  sql = cut(sql, "  shortfall_micros INTEGER,\n  from_allowance_micros INTEGER\n", "  shortfall_micros INTEGER\n");
  sql = cut(
    sql,
    "  created_at_unix_ms INTEGER NOT NULL,\n  from_allowance_micros INTEGER NOT NULL DEFAULT 0,\n  allowance_expires_ms INTEGER\n",
    "  created_at_unix_ms INTEGER NOT NULL\n",
  );
  sql = cut(sql, /CREATE TABLE IF NOT EXISTS ai_allowance_grant \([^;]*\);\n/);
  sql = cut(sql, /CREATE INDEX IF NOT EXISTS ai_allowance_grant_account [^;]*;\n/);
  sql = cut(sql, /CREATE TABLE IF NOT EXISTS server_setting \([^;]*\);\n/);
  return sql;
}

/** Every table's columns and every index's columns, by name. */
function shape(db: DatabaseSync) {
  const objects = db.prepare("SELECT type, name, tbl_name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name").all() as {
    type: string;
    name: string;
    tbl_name: string;
  }[];
  return objects.map((o) => ({
    ...o,
    columns:
      o.type === "table"
        ? db.prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?) ORDER BY cid").all(o.name)
        : db.prepare("SELECT seqno, name FROM pragma_index_info(?) ORDER BY seqno").all(o.name),
  }));
}

let dbs: DatabaseSync[] = [];
afterEach(() => {
  for (const db of dbs) db.close();
  dbs = [];
});
function fresh(): DatabaseSync {
  const db = new DatabaseSync(":memory:");
  dbs.push(db);
  return db;
}

it("migrations 0006 then 0007 applied to the old schema give exactly SCHEMA_SQL", () => {
  const migrated = fresh();
  migrated.exec(schemaBefore0006());
  const columns = (table: string) => (migrated.prepare("SELECT name FROM pragma_table_info(?)").all(table) as { name: string }[]).map((c) => c.name);
  // The old schema really lacks what 0006 adds.
  expect(columns("account")).not.toContain("plan_tier");
  expect(columns("ai_allowance_grant")).toEqual([]);
  migrated.exec(MIGRATION);

  for (const c of ["plan_tier", "ai_allowance_micros", "ai_allowance_expires_ms"]) expect(columns("account")).toContain(c);
  for (const c of ["from_allowance_micros", "allowance_expires_ms"]) expect(columns("ai_hold")).toContain(c);
  expect(columns("ai_ledger")).toContain("from_allowance_micros");
  expect(columns("ai_allowance_grant")).toEqual(["grant_key", "account_id", "micros", "period_end_unix_ms", "created_at_unix_ms"]);
  migrated.exec(MIGRATION_0007);
  expect(columns("ai_allowance_grant")).toEqual([
    "grant_key",
    "account_id",
    "micros",
    "period_end_unix_ms",
    "created_at_unix_ms",
    "line_start_unix_ms",
    "line_end_unix_ms",
    "subscription_id",
  ]);
  expect(columns("server_setting")).toEqual(["key", "value"]);
  expect(migrated.prepare("SELECT tbl_name FROM sqlite_master WHERE type = 'index' AND name = 'ai_allowance_grant_account'").all()).toEqual([
    { tbl_name: "ai_allowance_grant" },
  ]);

  const current = fresh();
  current.exec(SCHEMA_SQL);
  expect(shape(migrated)).toEqual(shape(current));
});

// Not schema, but the same kind of tie and it needs node:fs, so it lives
// here: wrangler.jsonc's cron triggers are exactly the strings `scheduled`
// dispatches on.
it("wrangler_crons_are_the_two_schedules_scheduled_dispatches_on", () => {
  const wrangler = readFileSync(new URL("../wrangler.jsonc", import.meta.url), "utf8");
  const crons = /"crons":\s*\[([^\]]*)\]/.exec(wrangler);
  expect(crons).not.toBeNull();
  expect(JSON.parse(`[${crons![1]}]`)).toEqual([DAILY_CRON, YEARLY_GRANTS_CRON]);
});
