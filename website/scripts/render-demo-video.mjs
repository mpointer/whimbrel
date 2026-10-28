// Records demo/tour.html into the product-tour video on the home page:
//   site/media/whimbrel-tour.mp4 (H.264), whimbrel-tour.webm (VP9), whimbrel-tour-poster.jpg
// Needs Playwright (with Chromium) and ffmpeg on PATH. Run: node scripts/render-demo-video.mjs
//
// This is an animated walkthrough of the app's UI, not a screen recording. When a real
// screen recording exists, drop it in as site/media/whimbrel-tour.mp4/.webm instead and
// update demoVideo.duration in site/config.json.
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createRequire } from "node:module";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const media = join(root, "site", "media");
mkdirSync(media, { recursive: true });

const require = createRequire(import.meta.url);
let playwright;
try {
  playwright = require("playwright");
} catch {
  playwright = require(join(execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim(), "playwright"));
}

const scratch = mkdtempSync(join(tmpdir(), "whimbrel-tour-"));
const size = { width: 1280, height: 720 };
const browser = await playwright.chromium.launch();
const context = await browser.newContext({ viewport: size, deviceScaleFactor: 1, recordVideo: { dir: scratch, size } });
const page = await context.newPage();
await page.goto(pathToFileURL(join(root, "demo", "tour.html")).toString() + "?render");
await page.waitForLoadState("networkidle");
const startedAt = Date.now();
await page.evaluate(() => window.startTour());
await page.waitForFunction(() => window.tourDone === true, null, { timeout: 120_000, polling: 200 });
const lead = (Date.now() - startedAt) / 1000;
await context.close();
await browser.close();

const raw = join(scratch, readdirSync(scratch).find((f) => f.endsWith(".webm")));
// The recording includes page load before startTour(); trim so the video starts on scene 1.
const probe = Number(execFileSync("ffprobe", ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", raw], { encoding: "utf8" }));
const start = Math.max(0, probe - lead).toFixed(2);
const ff = (...args) => execFileSync("ffmpeg", ["-loglevel", "error", "-y", "-ss", start, "-i", raw, ...args], { stdio: "inherit" });

ff("-vf", "fps=30", "-c:v", "libx264", "-preset", "slow", "-crf", "26", "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-an", join(media, "whimbrel-tour.mp4"));
ff("-vf", "fps=30", "-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "38", "-row-mt", "1", "-an", join(media, "whimbrel-tour.webm"));
// Poster: a frame from scene 2 (live notes), the most representative shot.
execFileSync("ffmpeg", ["-loglevel", "error", "-y", "-ss", String(Number(start) + 17), "-i", raw, "-frames:v", "1", "-q:v", "3", join(media, "whimbrel-tour-poster.jpg")]);

rmSync(scratch, { recursive: true, force: true });
console.log(`rendered ${(probe - Number(start)).toFixed(1)}s tour → site/media/`);
