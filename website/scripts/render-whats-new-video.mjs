// Records demo/whats-new.html into the "What's new in 0.9.80" video on the client-controls and encryption pages:
//   site/media/whimbrel-whats-new.mp4 (H.264), whimbrel-whats-new.webm (VP9), whimbrel-whats-new-poster.jpg
// Same approach as render-demo-video.mjs (the November 11 tour, which this script never touches).
// Needs Playwright (with Chromium) and ffmpeg on PATH. Run: node scripts/render-whats-new-video.mjs
// If Playwright is not installed next to the repo, set PLAYWRIGHT_DIR to a folder that holds node_modules/playwright.
//
// The tour is recorded as timed screenshots (see below), not Chrome's video capture.
// It shows real app screens (demo/shots/, captured by scripts/capture-app-shots.py
// from the Windows UI with demo data) with captions; it is not a screen recording. When a
// real recording exists, drop it in as site/media/whimbrel-whats-new.mp4/.webm instead and
// update demoVideo.duration in site/config.json.
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
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
  playwright = process.env.PLAYWRIGHT_DIR
    ? require(join(process.env.PLAYWRIGHT_DIR, "node_modules", "playwright"))
    : require(join(execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim(), "playwright"));
}

const scratch = mkdtempSync(join(tmpdir(), "whimbrel-whats-new-"));
const size = { width: 1280, height: 720 };
const browser = await playwright.chromium.launch();
const context = await browser.newContext({ viewport: size, deviceScaleFactor: 1 });
const page = await context.newPage();
await page.goto(pathToFileURL(join(root, "demo", "whats-new.html")).toString() + "?render");
await page.waitForLoadState("networkidle");

// Chrome's built-in video capture cropped the bottom of the frame on some machines, so the tour is
// recorded as timed screenshots instead: each frame is shown from its capture time until the next
// one, and ffmpeg's concat demuxer turns that list into constant-rate video.
const frames = [];
await page.evaluate(() => window.startTour());
const t0 = Date.now();
for (let n = 0; ; n++) {
  const at = Date.now() - t0;
  const file = join(scratch, `f${String(n).padStart(5, "0")}.jpg`);
  await page.screenshot({ path: file, type: "jpeg", quality: 92 });
  frames.push({ file, at });
  if (await page.evaluate(() => window.tourDone === true)) break;
  if (at > 120_000) throw new Error("video did not finish within 120s");
}
const total = (Date.now() - t0) / 1000;
await context.close();
await browser.close();

const list = frames
  .map((f, i) => `file '${f.file}'\nduration ${(((frames[i + 1]?.at ?? total * 1000) - f.at) / 1000).toFixed(3)}`)
  .join("\n");
const listFile = join(scratch, "frames.txt");
writeFileSync(listFile, list + `\nfile '${frames[frames.length - 1].file}'\n`);
const ff = (...args) => execFileSync("ffmpeg", ["-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i", listFile, ...args], { stdio: "inherit" });

ff("-vf", "fps=30", "-c:v", "libx264", "-preset", "slow", "-crf", "26", "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-an", join(media, "whimbrel-whats-new.mp4"));
ff("-vf", "fps=30", "-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "38", "-row-mt", "1", "-an", join(media, "whimbrel-whats-new.webm"));
// Poster: a frame from scene 1 (a client's page with its four controls).
execFileSync("ffmpeg", ["-loglevel", "error", "-y", "-ss", "8", "-i", join(media, "whimbrel-whats-new.mp4"), "-frames:v", "1", "-q:v", "3", join(media, "whimbrel-whats-new-poster.jpg")]);

rmSync(scratch, { recursive: true, force: true });
console.log(`rendered ${total.toFixed(1)}s video (${frames.length} frames) → site/media/`);
