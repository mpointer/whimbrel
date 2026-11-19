// Full-page PNGs of gallery.html: five modes at 100% text, and light at 200%.
// Output: docs/ui-redesign/screenshots/system/windows/gallery-<mode>-<size>.png
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { test } from "@playwright/test";

const OUT = fileURLToPath(new URL("../../../../docs/ui-redesign/screenshots/system/windows/", import.meta.url));
mkdirSync(OUT, { recursive: true });

const SHOTS: { mode: string; size: number; scheme: "light" | "dark"; extra?: { forcedColors?: "active" } }[] = [
  { mode: "system", size: 100, scheme: "light" },
  { mode: "light", size: 100, scheme: "light" },
  { mode: "dark", size: 100, scheme: "dark" },
  { mode: "hc-light", size: 100, scheme: "light" },
  // Light scheme on purpose: with the OS in dark, the generated dark block outranks .w-thread.w-mode-hc-dark (see the report).
  { mode: "hc-dark", size: 100, scheme: "light" },
  { mode: "light", size: 200, scheme: "light" },
];

for (const s of SHOTS) {
  test(`gallery ${s.mode} ${s.size}`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ colorScheme: s.scheme, ...s.extra });
    page.on("pageerror", (e) => console.log(`[pageerror] ${e.message}`));
    await page.goto(`/gallery.html?mode=${s.mode}&size=${s.size}`);
    await page.waitForSelector(".g-root");
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: `${OUT}gallery-${s.mode}-${s.size}.png`, fullPage: true });
  });
}
