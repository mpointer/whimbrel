import { defineConfig } from "@playwright/test";

// Screenshots for docs/ui-redesign (not a test run, and not part of CI). The built UI is drawn in a browser with
// the Tauri side faked (e2e/fakeBridge.ts), so these show the React screens only: no title bar, tray or Mica.
// Run: npm run build && npx playwright test -c playwright.audit.config.ts
const port = Number(process.env.AUDIT_PORT) || 4190;

export default defineConfig({
  testDir: "e2e/audit",
  testMatch: "*.shots.ts",
  timeout: 60_000,
  workers: 4,
  reporter: [["list"]],
  use: { channel: process.env.AUDIT_CHANNEL || "msedge", baseURL: `http://localhost:${port}`, reducedMotion: "reduce", timezoneId: "UTC", locale: "en-US" },
  webServer: { command: `npx vite preview --port ${port} --strictPort`, url: `http://localhost:${port}`, reuseExistingServer: true, timeout: 60_000 },
});
