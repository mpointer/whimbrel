import { defineConfig } from "@playwright/test";

// Screenshots of the design-system gallery (not part of CI). Build it first:
//   npx vite build --mode gallery --outDir dist-gallery && npx playwright test -c e2e/gallery.config.ts
const port = Number(process.env.GALLERY_PORT) || 4191;

export default defineConfig({
  testDir: ".",
  testMatch: "gallery.shots.ts",
  timeout: 120_000,
  reporter: [["list"]],
  use: { channel: process.env.AUDIT_CHANNEL || "chrome", baseURL: `http://localhost:${port}`, reducedMotion: "reduce" },
  webServer: { command: `npx vite preview --outDir dist-gallery --port ${port} --strictPort`, url: `http://localhost:${port}/gallery.html`, reuseExistingServer: true, timeout: 60_000 },
});
