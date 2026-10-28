# Whimbrel website: SEO strategy

## Positioning for search

Whimbrel combines things people search for separately: bot-free meeting notes, screen memory, and AI routines. Nobody searches for "ambient desktop assistant", so we don't target that phrase. We rank one page per problem people already search for, and every page leads to the same conversion: register, then download.

## Keyword clusters → pages

| Cluster (intent) | Example queries | Page | Primary CTA |
|---|---|---|---|
| Brand + category (navigational/commercial) | whimbrel app, whimbrel ai assistant, ai work memory app; kestrel app (former name, answered by the home page FAQ) | `/` | Register & download |
| Bot-free meeting notes (commercial) | ai meeting notes without bot, meeting recorder no bot, transcribe zoom calls locally, ai notetaker for teams | `/features/meeting-notes/` | Get Whimbrel free |
| Screen memory (commercial) | rewind ai alternative, screen memory app, search everything i've seen on my computer | `/features/screen-memory/` | Get Whimbrel free |
| Routines (commercial) | ai meeting prep, automatic daily recap, ai focus time calendar | `/features/routines/` | Get Whimbrel free |
| Competitor comparison (high intent) | littlebird alternative, littlebird vs granola, ai work assistant comparison | `/compare/` | Download |
| Competitor alternative (high intent) | granola alternative, granola for windows, granola vs … | `/compare/granola-alternative/` | Download |
| Open source (commercial, high intent) | open source ai meeting notes, open source granola alternative, self-hosted ai notetaker, local ai meeting assistant | `/open-source/` | Download free |
| Pricing (commercial) | whimbrel pricing, ai meeting notes pricing, granola pricing alternative | `/pricing/` | Download free |
| Release history (navigational, long tail) | whimbrel changelog, whimbrel 0.9 release notes, what's new in whimbrel | `/changelog/` | Download |
| Download (transactional) | whimbrel download mac, whimbrel for windows, ai meeting notes app windows 11 | `/download/`, `/download/mac/`, `/download/windows/` | Register form |

Rule: one primary cluster per page, no two pages targeting the same query (keyword cannibalization). New pages go into this table before they're written.

## On-page rules (enforced by `scripts/build.mjs`, the build fails otherwise)

- Title ≤ 65 characters including the ` | Whimbrel` suffix; description 70–160 characters.
- No duplicate titles or descriptions across indexable pages.
- Exactly one `<h1>` per page; every `<img>` has `alt`.
- Every internal link points at a page that exists.
- FAQ and breadcrumb JSON-LD are generated from the same front-matter that renders the visible FAQ/breadcrumbs, so the structured data can't drift from what's on the page (Google requires them to match).

## Technical SEO

- **Static HTML**, served from Cloudflare's edge. No client-side rendering, so crawlers get the full content on the first request. The Worker only runs for `/api/*`.
- **Core Web Vitals**: system font stacks (no web-font fetch), one CSS and one deferred JS file, both fingerprinted and cached for a year; the demo video uses `preload="none"` with a poster. Measured mobile CLS on `/download/` is 0.
- **Canonical URLs** on every page, trailing-slash URLs (`/features/meeting-notes/`); the non-slash form redirects.
- **`sitemap.xml`**: generated, indexable pages only, `lastmod` from each page's last git commit.
- **`robots.txt`**: blocks `/api/` and the thank-you/unavailable pages, which are also `noindex`.
- **Structured data**: `Organization`, `WebSite`, `SoftwareApplication` (per platform on the download pages, with OS requirements and a free `Offer`), `VideoObject` for the tour, `FAQPage`, `BreadcrumbList`.
- **Social**: Open Graph + Twitter large-image card on every page (`site/assets/og-image.png`, 1200×630).

## Measurement

- Registrations are stored with first-touch attribution (UTM source/medium/campaign, external referrer, landing page), so leads can be traced to the page and campaign that brought them. Query them with `wrangler d1 execute kestrel-leads --remote --command "…"`.
- Downloads are recorded per lead, platform, and version, so you can tell registration-to-download conversion apart from registrations alone.
- Add Google Search Console and Bing Webmaster Tools for the production domain and submit the sitemap. The site has no third-party analytics script; if you add one, update the CSP in `site/static/_headers` and the privacy policy.

## Content roadmap (in priority order)

1. **Comparison pages**: `/compare/otter-alternative/`, `/compare/rewind-alternative/`, `/compare/fireflies-alternative/`. These have the highest purchase intent. Each needs a fact check against the competitor's current site before publishing, and an honest "choose them if" section, as on the Granola page.
2. **Use-case pages**: meeting notes for sales calls, for engineering managers, for consultants. Map to the personas in `docs/01-product-vision.md`.
3. **Guides** (top of funnel, for links): "How to record a Teams meeting without a bot", "Is it legal to record a meeting? (by country/state)", "Running local LLMs for meeting summaries with Ollama".
4. ~~**Changelog**~~ Shipped 2026-10-21 as `/changelog/`, generated from GitHub releases and redeployed on every release.

## Before launch

- [x] Production domain `whimbrelapp.com` (the product was renamed from Kestrel, 2026-10-21) set in `site/config.json` and attached as Custom Domains in `wrangler.jsonc`. Needs the zone's imported `@`/`www` records removed first (README).
- [x] Live on whimbrelapp.com (2026-10-23); the workers.dev host and www 301 to it.
- [x] IndexNow: changed URLs are submitted on every deploy (README).
- [ ] Submit `https://whimbrelapp.com/sitemap.xml` to Google Search Console and Bing Webmaster Tools.
- [ ] Point `whimbrelapp.app` and `whimbrelapp.org` at `whimbrelapp.com` (still on Namecheap nameservers on 2026-10-23).
- [x] Whimbrel mark for the site (2026-10-23): `demo/whimbrel-mark.svg` (full bird) and `demo/whimbrel-mark-small.svg` (head and bill, for favicons); `scripts/render-icons.mjs` renders favicon.ico/svg, apple-touch-icon and assets/icon-*.png. The Mac and Windows app icons are still the whimbrel.
- [x] Replace `contactEmail` and `salesEmail` (both mike@futuremeanswered.com, 2026-10-21).
- [ ] Re-verify every Littlebird and Granola claim on `/compare/` and `/compare/granola-alternative/` and update the "last reviewed" dates. Competitor pricing especially changes often.
- [ ] Have counsel review `/privacy/`, especially if you market to the EU/UK (GDPR) or California (CCPA).
- [ ] Replace the animated tour with a real screen recording when one exists (see README).
