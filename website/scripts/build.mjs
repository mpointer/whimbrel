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
// Translated sites: pages live in site/pages/<locale>/, partials in site/partials/<locale>/ and
// strings in site/i18n/ (written by scripts/translate-site.py). A page with no translation stays English.
const locales = Object.fromEntries(
  ["es", "fr"].map((l) => [l, JSON.parse(readFileSync(join(siteDir, "i18n", `${l}.json`), "utf8"))]),
);
const localeOf = (route) => route.match(/^\/(es|fr)\//)?.[1] ?? null;
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
      const key = scope.locale && `${scope.locale}/${name}` in partials ? `${scope.locale}/${name}` : name;
      if (!(key in partials)) throw new Error(`unknown partial: ${name}`);
      return render(partials[key], scope, depth + 1);
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

function breadcrumbList(crumbs, home = "Home") {
  const items = [{ name: home, path: "/" }, ...crumbs];
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

function renderCrumbs(crumbs, home = "Home") {
  const items = [{ name: home, path: "/" }, ...crumbs];
  const lis = items
    .map((c, i) => (i === items.length - 1 ? `<li aria-current="page">${escapeHtml(c.name)}</li>` : `<li><a href="${c.path}">${escapeHtml(c.name)}</a></li>`))
    .join("");
  return `<nav class="crumbs container" aria-label="Breadcrumb"><ol>${lis}</ol></nav>`;
}

// ---- changelog -----------------------------------------------------------------------
// Built from the repository's CHANGELOG.md, the plain-language notes each release adds.
// Releases older than CHANGELOG.md come from site/data/releases.json
// (scripts/fetch-releases.mjs snapshots it from the GitHub releases), whose notes are PR
// titles, so only user-facing ones are shown: internal prefixes and unreleased work (the
// iPhone app's milestones) are dropped.

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

const changelogFile = join(siteDir, "..", "..", "CHANGELOG.md");
// The iPhone app is not released yet; its notes stay off the public page.
const HIDDEN_SECTIONS = new Set(["iPhone"]);

/** CHANGELOG.md as [{ version, date, intro: [paragraphs], items: [{ section, text }] }],
 * newest first. "## [Unreleased]" is skipped; "### Mac" etc. name each item's platform. */
function parseChangelog(md) {
  const out = [];
  let release = null;
  let section = null;
  for (const line of md.split(/\r?\n/)) {
    const head = line.match(/^## \[([^\]]+)\](?:\s*-\s*(\d{4}-\d{2}-\d{2}))?/);
    if (head) {
      release = head[1] === "Unreleased" ? null : { version: head[1], date: head[2], intro: [], items: [] };
      if (release) out.push(release);
      section = null;
      continue;
    }
    if (!release) continue;
    const sub = line.match(/^### (.+)$/);
    if (sub) section = sub[1].trim();
    else if (line.startsWith("- ")) {
      if (!HIDDEN_SECTIONS.has(section)) release.items.push({ section, text: line.slice(2).trim() });
    } else if (line.trim() && !section) release.intro.push(line.trim());
  }
  return out;
}

/** Escaped text with `code` spans; the private repository's "(#123)" references dropped. */
function inlineMarkdown(text) {
  return escapeHtml(text.replace(/\s*\((?:#\d+(?:,\s*)?)+\)/g, "")).replace(/`([^`]+)`/g, "<code>$1</code>");
}

const notes = existsSync(changelogFile) ? parseChangelog(readFileSync(changelogFile, "utf8")) : [];
const versionKey = (v) => v.split(".").map((n) => n.padStart(4, "0")).join(".");

function renderNotes(n) {
  const r = releases.find((x) => x.tag === `v${n.version}`);
  const url = r?.url ?? `https://github.com/mpointer/whimbrel/releases/tag/v${n.version}`;
  const date = n.date ?? r?.date;
  const intro = n.intro.map((p) => `<p>${inlineMarkdown(p)}</p>`).join("");
  const list = n.items.length
    ? `<ul>${n.items.map((e) => `<li>${e.section ? `<span class="tag">${escapeHtml(e.section)}</span> ` : ""}${inlineMarkdown(e.text)}</li>`).join("")}</ul>`
    : "";
  const when = date ? `<time datetime="${date}">${dateFormat.format(new Date(`${date}T00:00:00Z`))}</time> · ` : "";
  return `<article class="release" id="v${n.version.replace(/\./g, "-")}"><h2>Version ${escapeHtml(n.version)}</h2><p class="muted">${when}<a href="${escapeHtml(url)}">Release on GitHub</a></p>${intro}${list || (intro ? "" : `<p class="muted">Maintenance release: internal improvements.</p>`)}</article>`;
}

function renderChangelog() {
  const covered = new Set(notes.map((n) => n.version));
  const fromNotes = notes.map((n) => ({ version: n.version, html: renderNotes(n) }));
  const fromReleases = releases.filter((r) => !covered.has(r.tag.replace(/^v/, ""))).map((r) => ({ version: r.tag.replace(/^v/, ""), html: renderRelease(r) }));
  return [...fromNotes, ...fromReleases]
    .sort((a, b) => versionKey(b.version).localeCompare(versionKey(a.version)))
    .map((x) => x.html)
    .join("\n");
}

function renderRelease(r) {
    const version = r.tag.replace(/^v/, "");
    // A pull-request link is shown only when it is readable: releases before the
    // public repository link the private one, and the public notes carry no links.
    const items = r.entries
      .map((e) => ({ ...changelogEntry(e.title), url: e.url, number: e.number }))
      .filter((e) => e.text)
      .map((e) => ({ ...e, link: Boolean(e.url && e.number && !e.url.includes("github.com/mpointer/kestrel/")) }));
    const list = items.length
      ? `<ul>${items
          .map(
            (e) =>
              `<li>${e.platform ? `<span class="tag">${e.platform}</span> ` : ""}${escapeHtml(e.text)}${e.link ? ` <a class="pr" href="${escapeHtml(e.url)}">#${e.number}</a>` : ""}</li>`,
          )
          .join("")}</ul>`
      : `<p class="muted">Maintenance release: internal improvements.</p>`;
    return `<article class="release" id="v${version.replace(/\./g, "-")}"><h2>Version ${escapeHtml(version)}</h2><p class="muted"><time datetime="${r.date}">${dateFormat.format(new Date(`${r.date}T00:00:00Z`))}</time> · <a href="${escapeHtml(r.url)}">Release on GitHub</a></p>${list}</article>`;
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

const layouts = {
  en: readFileSync(join(siteDir, "layout.html"), "utf8"),
  es: readFileSync(join(siteDir, "i18n", "layout.es.html"), "utf8"),
  fr: readFileSync(join(siteDir, "i18n", "layout.fr.html"), "utf8"),
};
const pageFiles = walk(join(siteDir, "pages")).filter((f) => f.endsWith(".html"));
const routes = new Set(pageFiles.map(routeFor));
/** An internal link as this locale sees it: its translated page if there is one. */
function localizePath(path, locale) {
  const [, base, rest] = path.match(/^([^?#]*)(.*)$/);
  return routes.has(`/${locale}${base}`) ? `/${locale}${base}${rest}` : path;
}

/** The hreflang links and the header's language picker for a page. */
function languageLinks(route, locale, indexable) {
  const enRoute = locale ? route.slice(locale.length + 1) : route;
  const alternates = [
    ["en", "English", enRoute, routes.has(enRoute)],
    ...Object.entries(locales).map(([l, c]) => [c.hreflang, c.label, `/${l}${enRoute}`, routes.has(`/${l}${enRoute}`), l]),
  ];
  const current = locale ? locales[locale].hreflang : "en";
  const links = alternates.map(([tag, label, path, exists, l]) => {
    const href = exists ? path : l ? `/${l}/` : "/";
    return `<a href="${href}" hreflang="${tag}" lang="${tag}"${tag === current ? ' aria-current="true"' : ""}>${label}</a>`;
  });
  const present = alternates.filter((a) => a[3]);
  const tags =
    indexable && present.length > 1
      ? [...present.map(([tag, , path]) => `<link rel="alternate" hreflang="${tag}" href="${siteUrl}${path}">`),
         `<link rel="alternate" hreflang="x-default" href="${siteUrl}${enRoute}">`].join("\n  ")
      : "";
  return { tags, picker: `<nav class="lang-switch" aria-label="Language">${links.join("")}</nav>` };
}

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
  const locale = localeOf(route);
  const home = locale ? locales[locale].home : "Home";
  const pageSite = locale ? { ...site, ...locales[locale].site } : site;
  if (locale) {
    const source = join(siteDir, "pages", relative(join(siteDir, "pages", locale), file));
    const hash = createHash("sha256").update(readFileSync(source)).digest("hex").slice(0, 12);
    if (meta.sourceHash !== hash) console.warn(`warning: ${route} is behind its English page; run scripts/translate-site.py`);
  }
  const indexable = !meta.noindex;

  // SEO checks. Limits follow what Google typically shows before truncating.
  const fullTitle = route === "/" || route === `/${locale}/` ? meta.title : `${meta.title} | ${site.name}`;
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
    site: pageSite,
    locale,
    page: { ...meta, route },
    faq: meta.faq ? renderFaq(meta.faq) : "",
    crumbs: meta.crumbs ? renderCrumbs(meta.crumbs, home) : "",
    changelog: meta.changelog ? renderChangelog() : "",
  };
  if (meta.changelog && releases.length === 0 && notes.length === 0) fail(route, "no releases in CHANGELOG.md or site/data/releases.json");
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
  if (meta.crumbs) graph.push(breadcrumbList(locale ? meta.crumbs.map((c) => ({ ...c, path: localizePath(c.path, locale) })) : meta.crumbs, home));
  const jsonLd = graph.length
    ? `<script type="application/ld+json">${JSON.stringify({ "@context": "https://schema.org", "@graph": graph }).replace(/</g, "\\u003c")}</script>`
    : "";

  const canonical = route === "/404.html" ? "" : `${siteUrl}${route}`;
  const languages = languageLinks(route, locale, indexable);
  let html = render(layouts[locale ?? "en"], {
    site: pageSite,
    page: {
      lang: locale ? locales[locale].hreflang : "en",
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
    raw: { content, jsonLd, canonicalTag: canonical ? `<link rel="canonical" href="${canonical}">` : "", hreflang: languages.tags, langSwitch: "<!--lang-switch-->" },
  });
  if (locale) {
    html = html.replace(/href="(\/(?!assets\/|media\/|api\/)[^"]*)"/g, (_, href) => `href="${localizePath(href, locale)}"`);
  }
  html = html.replace("<!--lang-switch-->", languages.picker);

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
    const lastmod = meta.changelog && (notes[0]?.date ?? releases[0]?.date) ? (notes[0]?.date ?? releases[0].date) : lastModified(file);
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
  `User-agent: *\nAllow: /\nDisallow: /api/\nDisallow: /download/thanks/\nDisallow: /download/unavailable/\nDisallow: /es/download/thanks/\nDisallow: /es/download/unavailable/\nDisallow: /fr/download/thanks/\nDisallow: /fr/download/unavailable/\n\nSitemap: ${siteUrl}/sitemap.xml\n`,
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
