// Renders the Windows app icons from the Whimbrel vector source in apps/macos/design,
// and the tray icons from apps/windows/design/tray-glyph.svg.
// Run from the repo root after `npm ci` in apps/windows/ui:
//   node apps/windows/scripts/render-icons.mjs
import { createRequire } from "node:module";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(join(process.cwd(), "apps/windows/ui/package.json"));
const { chromium } = require("@playwright/test");

const design = "apps/macos/design";
const out = "apps/windows/src-tauri/icons";
const full = readFileSync(join(design, "app-icon-flat.svg"), "utf8");
// Below 64 px the full bird goes too thin; the small crop has a heavier bill.
const small = readFileSync(join(design, "app-icon-flat-small.svg"), "utf8");
const glyph = readFileSync("apps/windows/design/tray-glyph.svg", "utf8");

const pngs = [
  ["32x32.png", 32, small],
  ["64x64.png", 64, full],
  ["128x128.png", 128, full],
  ["128x128@2x.png", 256, full],
  ["icon.png", 512, full],
];
const ico = [
  [16, small],
  [24, small],
  [32, small],
  [48, small],
  [64, full],
  [256, full],
];

// Windows can't tint a tray icon the way macOS tints a template image, so the states that
// follow the taskbar come as a dark glyph (light taskbar) and a light one (dark taskbar).
// The tray is 16, 20, 24 or 32 px at 100, 125, 150 or 200% scaling.
const TRAY_SIZES = [16, 20, 24, 32, 40, 48, 64];
const DARK = "#1f1f1f";
const LIGHT = "#ffffff";
const PAUSED = 0.4;
const tray = [
  ["idle-light", DARK, 1],
  ["idle-dark", LIGHT, 1],
  ["paused-light", DARK, PAUSED],
  ["paused-dark", LIGHT, PAUSED],
  ["sync-error", "#f0a020", 1],
  ["recording", "#dc2626", 1],
];

// Edge ships with Windows, so no Playwright browser download is needed.
const browser = await chromium.launch({ channel: "msedge" });
const page = await browser.newPage();

async function render(svg, size, style = "") {
  await page.setViewportSize({ width: size, height: size });
  const sized = svg.replace("<svg ", `<svg width="${size}" height="${size}" style="${style}" `);
  await page.setContent(`<html><body style="margin:0;background:transparent">${sized}</body></html>`);
  return page.locator("svg").screenshot({ omitBackground: true });
}

// An .ico of PNG entries: ICONDIR, one ICONDIRENTRY per size, then the images.
function writeIco(path, images) {
  const header = Buffer.alloc(6 + 16 * images.length);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  let offset = header.length;
  images.forEach(([size, png], i) => {
    const at = 6 + 16 * i;
    header.writeUInt8(size >= 256 ? 0 : size, at);
    header.writeUInt8(size >= 256 ? 0 : size, at + 1);
    header.writeUInt16LE(1, at + 4);
    header.writeUInt16LE(32, at + 6);
    header.writeUInt32LE(png.length, at + 8);
    header.writeUInt32LE(offset, at + 12);
    offset += png.length;
  });
  writeFileSync(path, Buffer.concat([header, ...images.map(([, png]) => png)]));
}

for (const [name, size, svg] of pngs) {
  writeFileSync(join(out, name), await render(svg, size));
}

const images = [];
for (const [size, svg] of ico) images.push([size, await render(svg, size)]);
writeIco(join(out, "icon.ico"), images);

for (const [name, color, opacity] of tray) {
  const sizes = [];
  for (const size of TRAY_SIZES) {
    // Below 24 px the eye is a smudge of one or two pixels, so it's left out.
    const svg = size < 24 ? glyph.replace(' mask="url(#eye)"', "") : glyph;
    sizes.push([size, await render(svg, size, `color:${color};opacity:${opacity}`)]);
  }
  writeIco(join(out, "tray", `${name}.ico`), sizes);
}
await browser.close();
console.log(`wrote ${pngs.length} PNGs, icon.ico (${ico.map(([s]) => s).join(", ")}) and ${tray.length} tray icons`);
