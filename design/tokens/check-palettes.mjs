// Checks the fixed colors (recording, AI ink, paused, focus) against every palette the app ships, not only Thread.
// A skin cannot change the fixed colors, so each must stay readable on any bundled palette's page and card.
// Run: node design/tokens/check-palettes.mjs   (prints failures; exits 1 with --strict when there are any)
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("../../", import.meta.url));
const T = JSON.parse(readFileSync(`${ROOT}design/tokens/tokens.json`, "utf8"));
const lum = (hex) => {
  const n = parseInt(hex.slice(1), 16);
  const ch = (v) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return 0.2126 * ch((n >> 16) & 255) + 0.7152 * ch((n >> 8) & 255) + 0.0722 * ch(n & 255);
};
const ratio = (a, b) => (Math.max(lum(a), lum(b)) + 0.05) / (Math.min(lum(a), lum(b)) + 0.05);
const TEXT = ["recording", "aiInk", "paused"];
const NON_TEXT = ["focus"];
const failures = [];
let checked = 0;
for (const dir of ["themes/", "skins/palettes/"])
  for (const file of readdirSync(`${ROOT}${dir}`).filter((f) => f.endsWith(".json"))) {
    const palette = JSON.parse(readFileSync(`${ROOT}${dir}${file}`, "utf8"));
    for (const [variant, index] of [["light", 0], ["dark", 1]]) {
      // A one-variant palette serves both appearances, so its colors meet the fixed colors of its own kind.
      const colors = palette[variant];
      if (!colors) continue;
      const isDark = lum(colors.backgroundPage) < 0.2;
      const fixedIndex = isDark ? 1 : 0;
      for (const [names, min] of [[TEXT, 4.5], [NON_TEXT, 3]])
        for (const name of names)
          for (const surface of ["backgroundPage", "backgroundCard"]) {
            checked++;
            const r = ratio(T.fixed[name].$value[fixedIndex], colors[surface]);
            if (r < min) failures.push(`${dir}${file} ${variant}: ${name} on ${surface} = ${r.toFixed(2)} (needs ${min})`);
          }
      void index;
    }
  }
console.log(`${checked} pairs checked, ${failures.length} under their rule`);
for (const line of failures) console.log(`  ${line}`);
if (failures.length && process.argv.includes("--strict")) process.exit(1);
