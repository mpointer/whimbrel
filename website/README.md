# website

The Whimbrel marketing site: feature pages, a product-tour video, an SEO-checked static build, and a registration form that gates the Mac/Windows download. It runs as one Cloudflare Worker (`kestrel-website`): static pages are served from Workers static assets, and the Worker code handles `/api/*` only.

```
site/pages/      one HTML fragment per page, with JSON front-matter (title, description, schema, FAQ)
site/partials/   shared blocks: registration form, app mock, CTA, demo video
site/layout.html page shell: meta tags, Open Graph, JSON-LD, header/footer
site/config.json site URL, OS requirements, demo-video metadata, Turnstile site key
site/static/     copied to the site root: favicon, manifest, _headers (CSP, caching)
scripts/build.mjs        builds dist/ and fails on SEO mistakes (see SEO.md)
src/                     Worker: /api/register, /api/beta, /beta/review, /api/download/{mac,windows}, /api/releases
migrations/              D1 schema: leads, downloads, email preferences, beta_signups
demo/                    sources for the tour video and OG image (not deployed)
```

## How the download gate works

1. A visitor fills in the form. `POST /api/register` validates it, upserts a row in `leads` (one per email), and sets an HttpOnly `whimbrel_lead` cookie holding the lead's random id.
2. The thank-you page calls `/api/download/<platform>`. The Worker checks the cookie, reads the current release from the **same manifests the apps' auto-updaters use** (`server-updates/`: `/version.json` and `/windows/latest.json`), records the download, and redirects to the installer. Redirects only ever go to the updates Worker's host.
3. Visitors without a cookie are sent to the form first.

This is a soft gate: the installers are public on the updates Worker because the auto-updaters need them to be. The gate's job is to capture and attribute the lead, not to protect the file.

Spam protection: a hidden honeypot field always, plus Cloudflare Turnstile when `TURNSTILE_SECRET` is set (and `turnstileSiteKey` in `site/config.json`).

Email (Resend, from `whimbrel@futuremeanswered.com`): when the `RESEND_API_KEY` secret is set, a first-time registrant gets one email with personal download links, and `LEAD_ALERT_EMAIL` gets an alert for every registration, new or returning. The links carry the lead's id as `/api/download/<platform>?lead=<id>`, which the download route accepts in place of the cookie. Sender, reply-to and alert address are `vars` in `wrangler.jsonc`. Email is sent after the response and never fails a registration.

## Mobile beta sign-up

`/mobile-beta/` asks for name, email, phone (iPhone, Android or both) and computer (Mac, Windows or none). `POST /api/beta` runs the same checks, honeypot and Turnstile as `/api/register`, upserts the lead (same product-email double opt-in; a lead created here gets no next-day desktop follow-up), and saves one `beta_signups` row per open request: a repeat sign-up updates the pending row. The tester gets a "we got your request" email; `LEAD_ALERT_EMAIL` gets an alert with Approve and Decline links.

The links open `/beta/review`, signed with HMAC-SHA256 over the request id and action using the `BETA_REVIEW_SECRET` secret. A GET only shows the request and a confirm button; only the POST changes anything, so a mail scanner opening the link approves no one. Without the secret, sign-ups are still saved and the alert says review links are unavailable.

- **Approve** marks the request approved and sends the install email once, with only the sections for the phones asked for. iPhone uses `TESTFLIGHT_PUBLIC_URL`; while it is empty the email says the TestFlight link follows and the request is not marked sent, so approving again after setting it sends the full email. Android uses `ANDROID_RELEASE_URL`.
- **Decline** marks it declined and sends nothing. **Decline and tell them** also sends a short note.

Reading requests:

```bash
npx wrangler d1 execute kestrel-leads --remote --command \
  "SELECT email, phones, computer, status, created_at, install_email_sent_at FROM beta_signups ORDER BY created_at DESC LIMIT 50"
```

Lead routing: set `LEAD_WEBHOOK_URL` and every registration is also POSTed there as JSON (Zapier, Make, a HubSpot workflow, a Slack incoming webhook, …).

## Changelog

`/changelog/` is built from the repository's `CHANGELOG.md`, the plain-language notes each release adds (`## [x.y.z] - date`, `### Mac`/`### Windows`/… sections, `- ` items; `## [Unreleased]` and `### iPhone` stay off the page, and `(#123)` references are dropped). Releases older than `CHANGELOG.md` come from the GitHub release notes: `scripts/fetch-releases.mjs` snapshots them into `site/data/releases.json`, and the build shows their user-facing PR titles only, dropping internal prefixes (`ci:`, `docs:`, `server:`, `mobile:`, …) and milestone work. A change to `CHANGELOG.md` on `main` redeploys the site.

## Develop

Use npm 11. npm 10 crashes resolving vitest's optional peer dependencies (`Cannot read properties of null (reading 'edgesOut')`).

```bash
npx npm@11 ci
npm run db:migrate:local
npm run dev            # build + wrangler dev on http://localhost:8787
npm test               # build (incl. SEO checks) + Worker tests
npm run typecheck
```

## Deploy

Pushes to `main` that touch `website/`, and every published app release, deploy automatically (`.github/workflows/website-deploy.yml`): refresh the changelog, typecheck, build and test, apply D1 migrations, `wrangler deploy`, smoke-test. It needs the repository secrets `CLOUDFLARE_API_TOKEN` (Workers Scripts: Edit and D1: Edit on the account, Workers Routes: Edit on the `whimbrelapp.com` zone) and `CLOUDFLARE_ACCOUNT_ID`. Run it by hand from the Actions tab (workflow_dispatch) when needed.

### First time, or by hand

```bash
# The kestrel-leads D1 database already exists; its id is in wrangler.jsonc.
npm run db:migrate
npx wrangler secret put TURNSTILE_SECRET  # optional
npx wrangler secret put LEAD_WEBHOOK_URL  # optional
npx wrangler secret put RESEND_API_KEY    # turns on welcome and lead-alert email
npx wrangler secret put BETA_REVIEW_SECRET  # signs the mobile beta Approve/Decline links (any long random string)
npm run deploy
```

## Domain and the Kestrel to Whimbrel rename

The site is `https://whimbrelapp.com` (`url` in `site/config.json`; canonicals, sitemap and OG URLs all come from it). `wrangler.jsonc` attaches `whimbrelapp.com` and `www.whimbrelapp.com` to this Worker as Custom Domains on deploy. For that to succeed the zone must be on this Cloudflare account and those two names must have no other DNS records (delete any A, AAAA or CNAME records for `@` and `www` that Cloudflare imported when the zone was added).

The Worker keeps its name `kestrel-website`, and `kestrel-leads` keeps its name: both are resource names on the rename design's keep-list (`docs/superpowers/specs/2026-10-21-kestrel-to-whimbrel-rename-design.md`). Renaming the Worker would create a new one without its secrets or its workers.dev address.

Hostnames listed in `REDIRECT_HOSTS` 301 to `CANONICAL_HOST` (308 for non-GET requests). It lists `www.whimbrelapp.com` and the old `kestrel-website.mpointer.workers.dev` address, which `workers_dev: true` keeps enabled; without that setting, listing `routes` turns the workers.dev address off and old links 404.

`UPDATES_HOSTS` lists the updates Worker's other hostnames (`updates.whimbrelapp.com`), so the download gate accepts release manifests that name either host.

`renameNote` in `site/config.json` is an optional callout on the download pages. It explained the rename until the first Whimbrel-named release (0.9.35) shipped and is now `""`. The home page FAQ's "used to be called Kestrel" entry stays.

### IndexNow

Every deploy tells IndexNow (Bing, Yandex, Seznam, Naver and the other participating engines; Google does not take part) which pages changed: the workflow saves the live `sitemap.xml` before deploying, and `scripts/indexnow.mjs` submits the URLs that are new or whose `lastmod` moved. The key is `indexNowKey` in `site/config.json`, served as `/<key>.txt` from `site/static/`; it is public by design. To change it, replace both. To resubmit every URL, run the website-deploy workflow by hand with "Submit every sitemap URL to IndexNow" ticked.

After the domain is live, submit `https://whimbrelapp.com/sitemap.xml` to Search Console and Bing Webmaster Tools. See the launch checklist in [SEO.md](SEO.md).

Reading leads:

```bash
npx wrangler d1 execute kestrel-leads --remote --command \
  "SELECT email, name, company, platform, marketing_opt_in, utm_source, created_at FROM leads ORDER BY created_at DESC LIMIT 50"
```

## Demo video and OG image

`site/media/whimbrel-tour.{mp4,webm}` and the poster are recorded from `demo/tour.html`, an animated walkthrough built from the app's wireframes (`docs/refactor-package/reference/`). **It is not a screen recording of the real app.** Replace it with a real one when you have it: overwrite the two video files and the poster, and update `demoVideo.duration` (ISO 8601, e.g. `PT1M30S`) and `uploadDate` in `site/config.json`.

To re-render after editing the sources (needs Playwright with Chromium, plus ffmpeg):

```bash
node scripts/render-demo-video.mjs
node scripts/render-og-image.mjs
```

## Translated site

Spanish lives at `/es/` and Canadian French at `/fr/`. Translated pages are in `site/pages/<locale>/`, partials in `site/partials/<locale>/`, layouts and strings in `site/i18n/`. Links go to a page's translation when there is one; privacy, terms, compare and the changelog stay English. The build warns when a translated page is behind its English page. To refresh, run `ANTHROPIC_API_KEY=... python3 website/scripts/translate-site.py --model claude-sonnet-5-5`; it re-translates only the pages that changed. A changed partial or layout needs its translated file deleted first.
