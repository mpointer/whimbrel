import { defineConfig } from "@playwright/test";

// The no-overrun matrix (e2e/matrix.spec.ts) drives the built UI (`npm run build`) in the Edge that is
// already on every Windows machine, so no browser is downloaded. CI picks a free MATRIX_PORT, since two
// runner instances on one PC can run this at the same time.
const port = Number(process.env.MATRIX_PORT) || 4180;

export default defineConfig({
  testDir: "e2e",
  testMatch: "*.spec.ts",
  timeout: 60_000,
  workers: 4,
  reporter: [["list"]],
  // Headless, and with no accessibility tree on CI: the Windows runner runs in a person's desktop session, and each test page
  // (titled "Whimbrel") taking focus had Narrator say "Whimbrel" over and over on that PC. The tests query the DOM, not the
  // browser's accessibility tree, so they don't need it.
  use: {
    channel: process.env.MATRIX_CHANNEL || "msedge",
    baseURL: `http://localhost:${port}`,
    reducedMotion: "reduce",
    headless: true,
    launchOptions: { args: process.env.CI ? ["--disable-renderer-accessibility"] : [] },
  },
  webServer: { command: `npx vite preview --port ${port} --strictPort`, url: `http://localhost:${port}`, reuseExistingServer: !process.env.CI, timeout: 60_000 },
});
