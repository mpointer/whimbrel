import { readdirSync } from "node:fs";
import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

// Tests that need real SQLite (node:sqlite) cannot run in the Workers pool, so
// they run in a plain-Node project: *.realsql.test.ts and realSql.smoke.test.ts.
const REAL_SQL = ["test/**/*.realsql.test.ts", "test/realSql.smoke.test.ts"];

// The globs above are case-sensitive. A real-SQLite test whose name is
// mis-cased (say "x.realSql.test.ts") would land in the Workers pool, so
// refuse to start instead.
const testDir = new URL("./test/", import.meta.url);
for (const file of readdirSync(testDir, { recursive: true }) as string[]) {
  const name = file.split(/[\\/]/).pop()!;
  const lower = name.toLowerCase();
  const isRealSql = name.endsWith(".realsql.test.ts") || file === "realSql.smoke.test.ts";
  if (!isRealSql && (lower.endsWith(".realsql.test.ts") || lower === "realsql.smoke.test.ts")) {
    throw new Error(`test/${file}: a real-SQLite test must be named *.realsql.test.ts (lower case) to run in the node project`);
  }
}

export default defineConfig({
  test: {
    projects: [
      {
        plugins: [
          cloudflareTest({
            wrangler: { configPath: "./wrangler.jsonc" },
            miniflare: {
              bindings: {
                TURSO_DATABASE_URL: "https://test-db.example.turso.io",
                TURSO_DATABASE_AUTH_TOKEN: "test-database-token",
                TURSO_PLATFORM_API_TOKEN: "test-platform-token",
              },
            },
          }),
        ],
        test: { name: "workers", exclude: ["**/node_modules/**", ...REAL_SQL] },
      },
      {
        test: { name: "node", environment: "node", include: REAL_SQL },
      },
    ],
  },
});
