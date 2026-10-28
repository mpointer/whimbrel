import { defineConfig } from "@playwright/test";

// The no-overrun matrix (e2e/matrix.spec.ts) drives the built UI (`npm run build`) in the Edge that is
// already on every Windows machine, so no browser is downloaded.
export default defineConfig({
  testDir: "e2e",
  testMatch: "*.spec.ts",
  timeout: 60_000,
  workers: 4,
  reporter: [["list"]],
  use: { channel: "msedge", baseURL: "http://localhost:4180", reducedMotion: "reduce" },
  webServer: { command: "npx vite preview --port 4180 --strictPort", url: "http://localhost:4180", reuseExistingServer: !process.env.CI, timeout: 60_000 },
});
