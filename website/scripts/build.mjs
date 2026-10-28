// Builds the static marketing site into dist/ — no framework, no dependencies.
//
// Each file in site/pages/ is an HTML fragment that starts with a JSON front-matter
// comment (title, description, schema, ...). The build wraps it in site/layout.html,
// expands {{> partial}} includes and {{site.*}} values, emits JSON-LD from the same data
// the page shows (so FAQ/breadcrumb markup can never drift from the visible content),
// writes sitemap.xml + robots.txt, and fails on SEO mistakes: titles/descriptions out of
// range or duplicated, a page without exactly one <h1>, or an internal link to a page
// that doesn't exist.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const siteDir = join(root, "site");
const outDir = join(root, "dist");
const site = JSON.parse(readFileSync(join(siteDir, "config.json"), "utf8"));
const siteUrl = site.url.replace(/\/$/, "");

const errors = [];
const fail = (page, msg) => errors.push(`${page}: ${msg}`);

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    return statSync(p).isDirectory() ? walk(p) : [p];
  });
}

const escapeHtml = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const stripTags = (s) => s.replace(/<[^>]+>/g, "").replace(/\s+/g, " ").trim();

/** site/pages/index.html → "/", features/x.html → "/features/x/", 404.html → "/404.html" */
function routeFor(file) {
  const rel = relative(join(siteDir, "pages"), file).replace(/\\/g, "/").replace(/\.html$/, "");
  if (rel === "index") return "/";
  if (rel === "404") return "/404.html";
  return `/${rel.replace(/\/index$/, "")}/`;
}

function outFileFor(route) {
  if (route === "/404.html") return join(outDir, "404.html");
  return join(outDir, route, "index.html");
}

function lastModified(file) {
  try {
    const iso = execFileSync("git", ["log", "-1", "--format=%cI", "--", file], { cwd: root, encoding: "utf8" }).trim();
    if (iso) return iso.slice(0, 10);
  } catch {}
  return new Date().toISOString().slice(0, 10);
}

function fingerprint(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex").slice(0, 10);
}

// ---- partials and value substitution -------------------------------------------------

const partials = Object.fromEntries(
  walk(join(siteDir, "partials")).map((f) => [relative(join(siteDir, "partials"), f).replace(/\.html$/, ""), readFileSync(f, "utf8")]),
);

function lookup(path, scope) {
  return path.split(".").reduce((v, k) => (v == null ? undefined : v[k]), scope);
}

function render(template, scope, depth = 0) {
  if (depth > 8) throw new Error("partials nested too deeply");
  return template
    .replace(/\{\{>\s*([\w/-]+)\s*\}\}/g, (_, name) => {
      if (!(name in partials)) throw new Error(`unknown partial: ${name}`);
      return render(partials[name], scope, depth + 1);
    })
    .replace(/\{\{\{\s*([\w.]+)\s*\}\}\}/g, (_, path) => String(lookup(path, scope) ?? "")) // raw
    .replace(/\{\{\s*([\w.]+)\s*\}\}/g, (match, path) => {
      const v = lookup(path, scope);
      if (v === undefined) throw new Error(`unknown value: ${match}`);
      return escapeHtml(v);
    });
}

// ---- structured data -----------------------------------------------------------------

const organization = {
  "@type": "Organization",
  "@id": `${siteUrl}/#organization`,
  name: site.name,
  url: `${siteUrl}/`,
  logo: `${siteUrl}/assets/icon-512.png`,
  ...(site.sameAs?.length ? { sameAs: site.sameAs } : {}),
};

const website = {
  "@type": "WebSite",
  "@id": `${siteUrl}/#website`,
  name: site.name,
  url: `${siteUrl}/`,
  publisher: { "@id": `${siteUrl}/#organization` },
};

function softwareApplication(platform) {
  const os = { mac: site.requirements.mac, windows: site.requirements.windows };
  return {
    "@type": "SoftwareApplication",
    "@id": `${siteUrl}/#software${platform ? `-${platform}` : ""}`,
    name: platform ? `${site.name} for ${platform === "mac" ? "Mac" : "Windows"}` : site.name,
    applicationCategory: "BusinessApplication",
    applicationSubCategory: "Productivity",
    operatingSystem: platform ? os[platform] : `${os.mac}; ${os.windows}`,
    description: site.description,
    url: `${siteUrl}/download/${platform ? `${platform}/` : ""}`,
    downloadUrl: `${siteUrl}/download/${platform ? `${platform}/` : ""}`,
    image: `${siteUrl}${site.ogImage}`,
    publisher: { "@id": `${siteUrl}/#organization` },
    offers: { "@type": "Offer", price: "0", priceCurrency: "USD", description: site.pricingNote },
  };
}

function videoObject() {
  const v = site.demoVideo;
  return {
    "@type": "VideoObject",
    name: v.title,
    description: v.description,
    thumbnailUrl: [`${siteUrl}${v.poster}`],
    uploadDate: v.uploadDate,
    duration: v.duration,
    contentUrl: `${siteUrl}${v.mp4}`,
    embedUrl: `${siteUrl}/#demo`,
  };
}

function faqPage(faq) {
  return {
    "@type": "FAQPage",
    mainEntity: faq.map(({ q, a }) => ({ "@type": "Question", name: q, acceptedAnswer: { "@type": "Answer", text: a } })),
  };
}

function breadcrumbList(crumbs) {
  const items = [{ name: "Home", path: "/" }, ...crumbs];
  return {
    "@type": "BreadcrumbList",
    itemListElement: items.map((c, i) => ({ "@type": "ListItem", position: i + 1, name: c.name, item: `${siteUrl}${c.path}` })),
  };
}

function renderFaq(faq) {
  return faq
    .map(({ q, a }) => `<details class="faq-item"><summary><h3>${escapeHtml(q)}</h3></summary><p>${a}</p></details>`)
    .join("\n");
}

function renderCrumbs(crumbs) {
  const items = [{ name: "Home", path: "/" }, ...crumbs];
  const lis = items
    .map((c, i) => (i === items.length - 1 ? `<li aria-current="page">${escapeHtml(c.name)}</li>` : `<li><a href="${c.path}">${escapeHtml(c.name)}</a></li>`))
    .join("");
  return `<nav class="crumbs container" aria-label="Breadcrumb"><ol>${lis}</ol></nav>`;
}

// ---- changelog -----------------------------------------------------------------------
// Built from site/data/releases.json (scripts/fetch-releases.mjs snapshots it from the
// GitHub releases). Release notes are PR titles, so only user-facing ones are shown:
// internal prefixes and unreleased work (the iPhone app's milestones) are dropped.

const releasesFile = join(siteDir, "data", "releases.json");
const releases = existsSync(releasesFile) ? JSON.parse(readFileSync(releasesFile, "utf8")) : [];
const INTERNAL_PREFIXES = new Set([
  "api", "build", "chore", "ci", "core", "docs", "ios", "mobile", "refactor", "release",
  "server", "sync-engine", "test", "tests", "website", "worker",
]);
const PLATFORM_PREFIXES = { windows: "Windows", macos: "Mac", mac: "Mac" };
const dateFormat = new Intl.DateTimeFormat("en-US", { year: "numeric", month: "long", day: "numeric", timeZone: "UTC" });

function changelogEntry(title) {
  const m = title.match(/^([\w-]+)(?:\([^)]*\))?:\s*(.+)$/);
  const prefix = m?.[1].toLowerCase();
  if (prefix && INTERNAL_PREFIXES.has(prefix)) return null;
  if (/\bM\d+\.\d+\b|\bminors\b/i.test(title)) return null;
  let text = (m ? m[2] : title).replace(/\s*\((?:M\d|task|#\d)[^)]*\)\s*$/i, "");
  let platform = prefix ? PLATFORM_PREFIXES[prefix] ?? null : null;
  const lead = text.match(/^(Mac|Windows)\s+/);
  if (lead) {
    platform ??= lead[1];
    text = text.slice(lead[0].length);
  }
  text = text.charAt(0).toUpperCase() + text.slice(1);
  // Without its prefix a fix's title can read like a new bug ("X deleted Y").
  if (prefix === "fix") text = `Fixed: ${text}`;
  return { text, platform };
}

function renderChangelog() {
  return releases
    .map((r) => {
      const version = r.tag.replace(/^v/, "");
      const items = r.entries
        .map((e) => ({ ...changelogEntry(e.title), url: e.url, number: e.number }))
        .filter((e) => e.text);
      const list = items.length
        ? `<ul>${items
            .map(
              (e) =>
                `<li>${e.platform ? `<span class="tag">${e.platform}</span> ` : ""}${escapeHtml(e.text)} <a class="pr" href="${escapeHtml(e.url)}">#${e.number}</a></li>`,
            )
            .join("")}</ul>`
        : `<p class="muted">Maintenance release: internal improvements.</p>`;
      return `<article class="release" id="v${version.replace(/\./g, "-")}"><h2>Version ${escapeHtml(version)}</h2><p class="muted"><time datetime="${r.date}">${dateFormat.format(new Date(`${r.date}T00:00:00Z`))}</time> · <a href="${escapeHtml(r.url)}">Release on GitHub</a></p>${list}</article>`;
    })
    .join("\n");
}

// ---- build ---------------------------------------------------------------------------

rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });
cpSync(join(siteDir, "assets"), join(outDir, "assets"), { recursive: true });
if (existsSync(join(siteDir, "media"))) cpSync(join(siteDir, "media"), join(outDir, "media"), { recursive: true });
for (const f of readdirSync(join(siteDir, "static"))) cpSync(join(siteDir, "static", f), join(outDir, f), { recursive: true });

const assetVersion = {
  css: fingerprint(join(siteDir, "assets", "site.css")),
  js: fingerprint(join(siteDir, "assets", "site.js")),
};

const layout = readFileSync(join(siteDir, "layout.html"), "utf8");
const pageFiles = walk(join(siteDir, "pages")).filter((f) => f.endsWith(".html"));
const routes = new Set(pageFiles.map(routeFor));
const seenTitles = new Map();
const seenDescriptions = new Map();
const sitemap = [];

for (const file of pageFiles) {
  const route = routeFor(file);
  const raw = readFileSync(file, "utf8");
  const m = raw.match(/^<!--\s*(\{[\s\S]*?\})\s*-->\s*/);
  if (!m) {
    fail(route, "missing JSON front-matter comment");
    continue;
  }
  const meta = JSON.parse(m[1]);
  const body = raw.slice(m[0].length);
  const indexable = !meta.noindex;

  // SEO checks. Limits follow what Google typically shows before truncating.
  const fullTitle = route === "/" ? meta.title : `${meta.title} | ${site.name}`;
  if (indexable) {
    if (fullTitle.length > 65) fail(route, `title is ${fullTitle.length} chars (max 65): "${fullTitle}"`);
    if (meta.description.length < 70 || meta.description.length > 160)
      fail(route, `description is ${meta.description.length} chars (want 70–160)`);
    if (seenTitles.has(fullTitle)) fail(route, `duplicate title with ${seenTitles.get(fullTitle)}`);
    if (seenDescriptions.has(meta.description)) fail(route, `duplicate description with ${seenDescriptions.get(meta.description)}`);
    seenTitles.set(fullTitle, route);
    seenDescriptions.set(meta.description, route);
  }

  const scope = {
    site,
    page: { ...meta, route },
    faq: meta.faq ? renderFaq(meta.faq) : "",
    crumbs: meta.crumbs ? renderCrumbs(meta.crumbs) : "",
    changelog: meta.changelog ? renderChangelog() : "",
  };
  if (meta.changelog && releases.length === 0) fail(route, "no releases in site/data/releases.json (run scripts/fetch-releases.mjs)");
  const content = render(body, scope);

  const h1s = content.match(/<h1[\s>]/g) ?? [];
  if (h1s.length !== 1) fail(route, `has ${h1s.length} <h1> elements (want exactly 1)`);
  for (const [, alt] of content.matchAll(/<img(?![^>]*\balt=)[^>]*>/g)) fail(route, `<img> without alt ${alt ?? ""}`);

  const graph = [];
  for (const kind of meta.schema ?? []) {
    if (kind === "organization") graph.push(organization);
    else if (kind === "website") graph.push(website);
    else if (kind === "software") graph.push(softwareApplication(null));
    else if (kind === "software-mac") graph.push(softwareApplication("mac"));
    else if (kind === "software-windows") graph.push(softwareApplication("windows"));
    else if (kind === "video") graph.push(videoObject());
    else throw new Error(`${route}: unknown schema kind ${kind}`);
  }
  if (meta.faq) graph.push(faqPage(meta.faq.map(({ q, a }) => ({ q, a: stripTags(a) }))));
  if (meta.crumbs) graph.push(breadcrumbList(meta.crumbs));
  const jsonLd = graph.length
    ? `<script type="application/ld+json">${JSON.stringify({ "@context": "https://schema.org", "@graph": graph }).replace(/</g, "\\u003c")}</script>`
    : "";

  const canonical = route === "/404.html" ? "" : `${siteUrl}${route}`;
  const html = render(layout, {
    site,
    page: {
      title: fullTitle,
      description: meta.description,
      canonical,
      robots: indexable ? "index, follow, max-image-preview:large" : "noindex, follow",
      ogType: meta.ogType ?? "website",
      ogImage: `${siteUrl}${meta.ogImage ?? site.ogImage}`,
      bodyClass: meta.bodyClass ?? "",
    },
    assets: assetVersion,
    year: String(new Date().getFullYear()),
    raw: { content, jsonLd, canonicalTag: canonical ? `<link rel="canonical" href="${canonical}">` : "" },
  });

  for (const [, href] of html.matchAll(/href="(\/[^"#?]*)/g)) {
    if (href.startsWith("/assets/") || href.startsWith("/media/") || href.startsWith("/api/")) continue;
    if (/\.(xml|txt|webmanifest|png|svg|ico)$/.test(href)) continue;
    if (!routes.has(href)) fail(route, `links to missing page ${href}`);
  }

  const outFile = outFileFor(route);
  mkdirSync(dirname(outFile), { recursive: true });
  writeFileSync(outFile, html);
  if (indexable && route !== "/404.html") {
    // The changelog changes with each release, not when its template file does.
    const lastmod = meta.changelog && releases.length ? releases[0].date : lastModified(file);
    sitemap.push({ loc: `${siteUrl}${route}`, lastmod, priority: meta.priority ?? 0.5 });
  }
}

sitemap.sort((a, b) => b.priority - a.priority || a.loc.localeCompare(b.loc));
writeFileSync(
  join(outDir, "sitemap.xml"),
  `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${sitemap
    .map((u) => `  <url><loc>${u.loc}</loc><lastmod>${u.lastmod}</lastmod><priority>${u.priority.toFixed(1)}</priority></url>`)
    .join("\n")}\n</urlset>\n`,
);
writeFileSync(
  join(outDir, "robots.txt"),
  `User-agent: *\nAllow: /\nDisallow: /api/\nDisallow: /download/thanks/\nDisallow: /download/unavailable/\n\nSitemap: ${siteUrl}/sitemap.xml\n`,
);

// Launch blockers that shouldn't break local builds or CI.
for (const key of ["contactEmail", "salesEmail"]) {
  if (/example\.com$/.test(site[key] ?? "")) console.warn(`warning: site/config.json ${key} is still a placeholder`);
}

if (errors.length) {
  console.error(`SEO/build check failed:\n  ${errors.join("\n  ")}`);
  process.exit(1);
}
console.log(`built ${pageFiles.length} pages → ${relative(process.cwd(), outDir) || "dist"} (${sitemap.length} in sitemap)`);
