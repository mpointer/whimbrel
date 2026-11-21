import { DatabaseSync } from "node:sqlite";
import { SCHEMA_SQL, type SqlExecutor, type SqlRow, type SqlStatement } from "../../src/accounts";

/** A SqlExecutor over a real in-memory SQLite (node:sqlite), created from
 * SCHEMA_SQL. Unlike FakeSqlExecutor it runs the SQL text that ships, so
 * money statements can be tested for real. `batch` is ONE transaction:
 * if any statement throws, every earlier statement is rolled back. */
export function realSqlExecutor(): SqlExecutor & { close(): void } {
  const db = new DatabaseSync(":memory:");
  db.exec(SCHEMA_SQL);
  const run = (sql: string, params: unknown[]): SqlRow[] => {
    const stmt = db.prepare(sql);
    // all() also executes writes (returning no rows), and changes() then
    // reports the previous statement as SQLite does. Production differs in
    // transport only: Turso runs a batch as one BEGIN IMMEDIATE transaction
    // through a Hrana batch; here it is a local BEGIN ... COMMIT.
    return stmt.all(...(params as never[])) as SqlRow[];
  };
  return {
    async execute(sql: string, params: unknown[]): Promise<SqlRow[]> {
      return run(sql, params);
    },
    async batch(stmts: SqlStatement[]): Promise<SqlRow[][]> {
      db.exec("BEGIN");
      try {
        const out = stmts.map((s) => run(s.sql, s.params));
        db.exec("COMMIT");
        return out;
      } catch (e) {
        // A failed ROLLBACK (say, SQLite already rolled back) must not hide
        // the statement error that caused it.
        try {
          db.exec("ROLLBACK");
        } catch {
          // ignored: the original error is what the caller needs
        }
        throw e;
      }
    },
    close() {
      db.close();
    },
  };
}
