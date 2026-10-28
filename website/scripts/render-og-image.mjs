// Renders the 1200×630 social-share image (site/assets/og-image.png) from demo/og.html.
// Needs Playwright with Chromium. Run: node scripts/render-og-image.mjs
import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
let playwright;
try {
  playwright = require("playwright");
} catch {
  playwright = require(join(execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim(), "playwright"));
}
const browser = await playwright.chromium.launch();
const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
await page.goto(pathToFileURL(join(root, "demo", "og.html")).toString());
await page.waitForLoadState("networkidle");
await page.screenshot({ path: join(root, "site", "assets", "og-image.png") });
await browser.close();
console.log("rendered site/assets/og-image.png");
