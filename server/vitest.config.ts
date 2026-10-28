import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

export default defineConfig({
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
});
