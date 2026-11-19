// Writes e2e/.generated/skins.json for the screenshot harness, straight from the repo's skin and palette JSON.
// CI builds this file with a cargo test in src-tauri (which also repairs palettes per layout); that test only
// runs on Windows, so this stands in for it elsewhere. Run from apps/windows/ui: node e2e/audit/make-skins.mjs
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("../../../../../", import.meta.url));
const OUT = fileURLToPath(new URL("../.generated/", import.meta.url));
const read = (path) => JSON.parse(readFileSync(path, "utf8"));
const palette = (id) => {
  for (const dir of ["themes/", "skins/palettes/"]) if (existsSync(`${ROOT}${dir}${id}.json`)) return read(`${ROOT}${dir}${id}.json`);
  throw new Error(`no palette named ${id}`);
};
const skins = readdirSync(`${ROOT}skins`)
  .filter((file) => file.endsWith(".json") && file !== "icons.json")
  .map((file) => {
    const skin = read(`${ROOT}skins/${file}`);
    return { skin, palettes: skin.palettes.map(palette), builtIn: true };
  });
mkdirSync(OUT, { recursive: true });
writeFileSync(`${OUT}skins.json`, JSON.stringify(skins));
console.log(`wrote ${skins.length} skins`);
