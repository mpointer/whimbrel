// Snapshots the app's GitHub releases into site/data/releases.json for the /changelog/
// page. The build reads only the snapshot, so local builds and tests never need the
// network; the deploy workflow runs this first so the live page is current. Set
// GITHUB_TOKEN to avoid the unauthenticated rate limit (60 requests an hour).
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repo = process.env.RELEASES_REPO ?? "mpointer/kestrel";
const out = join(dirname(fileURLToPath(import.meta.url)), "..", "site", "data", "releases.json");

const headers = { accept: "application/vnd.github+json", "user-agent": "whimbrel-website-build" };
if (process.env.GITHUB_TOKEN) headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`;

const releases = [];
for (let page = 1; page <= 10; page++) {
  const res = await fetch(`https://api.github.com/repos/${repo}/releases?per_page=100&page=${page}`, { headers });
  if (!res.ok) throw new Error(`GitHub releases: HTTP ${res.status} ${await res.text()}`);
  const batch = await res.json();
  releases.push(...batch);
  if (batch.length < 100) break;
}

// Auto-generated notes list one PR per line: "* <title> by @<user> in <PR url>".
const ENTRY = /^\*\s+(.+?)\s+by\s+@\S+\s+in\s+(https:\/\/github\.com\/\S+\/pull\/(\d+))\s*$/;

const snapshot = releases
  .filter((r) => !r.draft && !r.prerelease)
  .map((r) => ({
    tag: r.tag_name,
    date: r.published_at.slice(0, 10),
    url: r.html_url,
    entries: (r.body ?? "")
      .split(/\r?\n/)
      .map((line) => line.match(ENTRY))
      .filter(Boolean)
      .map(([, title, url, number]) => ({ title, url, number: Number(number) })),
  }))
  .sort((a, b) => b.date.localeCompare(a.date) || b.tag.localeCompare(a.tag, undefined, { numeric: true }));

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, `${JSON.stringify(snapshot, null, 2)}\n`);
console.log(`wrote ${snapshot.length} releases → ${out}`);
