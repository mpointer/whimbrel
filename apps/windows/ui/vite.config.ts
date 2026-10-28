import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri serves the dev build from a fixed port (tauri.conf.json `devUrl`) and loads
// the production build from `dist/` inside WebView2 (current Chromium).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // themes/ at the repo root is shared with the Mac app and imported by themes.ts, so the dev server
  // (and Vitest) may read outside this folder. Paths here are relative to it: ../../.. is the repo.
  server: { port: 5173, strictPort: true, fs: { allow: ["../../.."] } },
  build: { target: "es2022", outDir: "dist" },
  // colors.test.ts reads tokens.css and app.css as text; by default Vitest blanks CSS imports.
  // e2e/*.spec.ts belong to Playwright (npm run test:matrix); e2e/*.test.ts are ordinary unit tests.
  test: { css: true, include: ["src/**/*.test.{ts,tsx}", "e2e/**/*.test.ts"] },
});
