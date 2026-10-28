// Renders the site's icons from the Whimbrel mark sources in demo/:
//   whimbrel-mark-small.svg -> site/static/favicon.svg (copied) and site/static/favicon.ico (16, 32, 48)
//   whimbrel-mark.svg       -> site/static/apple-touch-icon.png (180) and site/assets/icon-{192,256,512}.png
// Needs Playwright with Chromium, and Python 3 with Pillow for the .ico.
// Run: node scripts/render-icons.mjs, then node scripts/render-og-image.mjs (the OG image shows icon-512).
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
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
async function render(svg, size, out) {
  const page = await browser.newPage({ viewport: { width: size, height: size } });
  await page.goto(pathToFileURL(join(root, "demo", svg)).toString());
  await page.screenshot({ path: out, omitBackground: true });
  await page.close();
}

const scratch = mkdtempSync(join(tmpdir(), "whimbrel-icons-"));
try {
  await render("whimbrel-mark.svg", 180, join(root, "site", "static", "apple-touch-icon.png"));
  for (const size of [192, 256, 512]) await render("whimbrel-mark.svg", size, join(root, "site", "assets", `icon-${size}.png`));
  const big = join(scratch, "small-256.png");
  await render("whimbrel-mark-small.svg", 256, big);
  execFileSync("python3", [
    "-c",
    "import sys; from PIL import Image; Image.open(sys.argv[1]).save(sys.argv[2], sizes=[(16,16),(32,32),(48,48)])",
    big,
    join(root, "site", "static", "favicon.ico"),
  ]);
  copyFileSync(join(root, "demo", "whimbrel-mark-small.svg"), join(root, "site", "static", "favicon.svg"));
} finally {
  await browser.close();
  rmSync(scratch, { recursive: true, force: true });
}
console.log("rendered favicon.svg, favicon.ico, apple-touch-icon.png and assets/icon-{192,256,512}.png");
