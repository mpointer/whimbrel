// Refreshes site/data/releases.json, the snapshot the /changelog/ page is built from,
// with the app's GitHub releases. The build reads only the snapshot, so local builds and
// tests never need the network; the deploy workflow runs this first so the live page is
// current. Set GITHUB_TOKEN to avoid the unauthenticated rate limit (60 requests an hour).
//
// The committed snapshot is the history: releases fetched now are merged over it by
// tag, never replacing it. The public repository (mpointer/whimbrel) carries releases
// only from v0.9.35 on; the earlier ones exist in the snapshot alone.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repo = process.env.RELEASES_REPO ?? "mpointer/whimbrel";
const out = join(dirname(fileURLToPath(import.meta.url)), "..", "site", "data", "releases.json");

const headers = { accept: "application/vnd.github+json", "user-agent": "whimbrel-website-build" };
if (process.env.GITHUB_TOKEN) headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`;

// RELEASES_SOURCE_FILE: a saved `gh api --paginate repos/<repo>/releases` response, for
// refreshing the snapshot from a repository the network here cannot read directly.
const releases = [];
if (process.env.RELEASES_SOURCE_FILE) {
  releases.push(...JSON.parse(readFileSync(process.env.RELEASES_SOURCE_FILE, "utf8")));
} else {
  for (let page = 1; page <= 10; page++) {
    const res = await fetch(`https://api.github.com/repos/${repo}/releases?per_page=100&page=${page}`, { headers });
    if (!res.ok) throw new Error(`GitHub releases: HTTP ${res.status} ${await res.text()}`);
    const batch = await res.json();
    releases.push(...batch);
    if (batch.length < 100) break;
  }
}

// Auto-generated notes list one PR per line: "* <title> by @<user> in <PR url>". The
// public repository's notes are copied with the private links stripped, so a line can
// also be a bare "* <title>".
const LINKED = /^\*\s+(.+?)\s+by\s+@\S+\s+in\s+(https:\/\/github\.com\/\S+\/pull\/(\d+))\s*$/;
const BARE = /^\*\s+(?!\*\*)(.+?)\s*$/;

function entry(line) {
  // The "New Contributors" section's lines are bullets too, but not changes.
  if (/made their first contribution/.test(line)) return null;
  const linked = line.match(LINKED);
  if (linked) return { title: linked[1], url: linked[2], number: Number(linked[3]) };
  const bare = line.match(BARE);
  return bare ? { title: bare[1] } : null;
}

const fetched = releases
  .filter((r) => !r.draft && !r.prerelease)
  .map((r) => ({
    tag: r.tag_name,
    date: r.published_at.slice(0, 10),
    url: r.html_url,
    entries: (r.body ?? "").split(/\r?\n/).map(entry).filter(Boolean),
  }));

const byTag = new Map();
if (existsSync(out)) for (const r of JSON.parse(readFileSync(out, "utf8"))) byTag.set(r.tag, r);
for (const r of fetched) byTag.set(r.tag, r);
const snapshot = [...byTag.values()].sort(
  (a, b) => b.date.localeCompare(a.date) || b.tag.localeCompare(a.tag, undefined, { numeric: true }),
);

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, `${JSON.stringify(snapshot, null, 2)}\n`);
console.log(`wrote ${snapshot.length} releases (${fetched.length} from ${process.env.RELEASES_SOURCE_FILE ?? repo}) → ${out}`);
