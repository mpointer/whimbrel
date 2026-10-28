// Tells IndexNow (Bing, Yandex, Seznam, Naver and the other participating engines; not
// Google) which pages changed in this deploy. Compares the sitemap that was live before
// the deploy with the one just built and submits every URL that is new or whose lastmod
// moved. The key is public by design and is served as /<key>.txt (site/static/).
//
//   node scripts/indexnow.mjs <previous-sitemap.xml> [dist/sitemap.xml]
//   node scripts/indexnow.mjs --all [dist/sitemap.xml]
//
// A missing or unreadable previous sitemap, or --all, submits everything. Never exits non-zero: a
// failed ping must not fail a deploy that already succeeded.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const site = JSON.parse(readFileSync(join(root, "site", "config.json"), "utf8"));
const [previousPath, nextPath = join(root, "dist", "sitemap.xml")] = process.argv.slice(2);

/** Map of loc -> lastmod from a sitemap's XML. */
export function parseSitemap(xml) {
  const urls = new Map();
  for (const [, block] of xml.matchAll(/<url>([\s\S]*?)<\/url>/g)) {
    const loc = block.match(/<loc>([^<]+)<\/loc>/)?.[1]?.trim();
    if (loc) urls.set(loc, block.match(/<lastmod>([^<]+)<\/lastmod>/)?.[1]?.trim() ?? "");
  }
  return urls;
}

/** URLs in `next` that are absent from `previous` or have a different lastmod. */
export function changedUrls(previous, next) {
  return [...next].filter(([loc, lastmod]) => previous.get(loc) !== lastmod).map(([loc]) => loc);
}

async function main() {
  if (!site.indexNowKey) return console.log("indexnow: no indexNowKey in site/config.json, skipping");
  const read = (p) => (p && existsSync(p) ? readFileSync(p, "utf8") : "");
  const next = parseSitemap(read(nextPath));
  if (next.size === 0) return console.log(`indexnow: no URLs in ${nextPath}, skipping`);
  const urlList = previousPath === "--all" ? [...next.keys()] : changedUrls(parseSitemap(read(previousPath)), next);
  if (urlList.length === 0) return console.log("indexnow: no pages changed");

  const origin = new URL(site.url).origin;
  const body = {
    host: new URL(origin).host,
    key: site.indexNowKey,
    keyLocation: `${origin}/${site.indexNowKey}.txt`,
    urlList,
  };
  try {
    const res = await fetch("https://api.indexnow.org/indexnow", {
      method: "POST",
      headers: { "content-type": "application/json; charset=utf-8" },
      body: JSON.stringify(body),
    });
    // 200 and 202 both mean accepted; 403 means the key file wasn't found at keyLocation.
    console.log(`indexnow: submitted ${urlList.length} URL(s), HTTP ${res.status}`);
    for (const u of urlList) console.log(`  ${u}`);
  } catch (err) {
    console.log(`indexnow: request failed: ${err}`);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
