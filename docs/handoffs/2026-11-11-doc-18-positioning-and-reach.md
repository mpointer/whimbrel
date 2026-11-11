# Handoff: doc 18, positioning and reach

A prompt for the coding agent working on this repository. Paste it whole. It scopes the first increment of
[18-second-ring-competitors-differentiation-monetization.md](../18-second-ring-competitors-differentiation-monetization.md)
§6 and nothing else. Written 2026-11-11 against 0.9.68.

---

## Read first, in this order

1. `docs/18-second-ring-competitors-differentiation-monetization.md`. Sections 0, 4, 5 and 6
   are the brief. §1 lists where docs 16 and 17 are already out of date; trust the tree and
   `CHANGELOG.md` over any older doc.
2. PR [mpointer/kestrel#727](https://github.com/mpointer/kestrel/pull/727): doc 18, the two
   test landing pages (`website/site/pages/for/`), the new product-tour video and the
   harness that captures real app screens for it (`website/scripts/capture-app-shots.py`,
   `website/demo/`).
3. `website/README.md` and `website/SEO.md`: how pages are built and what the build rejects.
4. `CONTRIBUTING.md` and the README of whichever app you touch.

Verify every claim doc 18 makes about the tree before you build on it, and correct the doc
in your first PR if one is stale.

## The one rule that governs all copy

Whimbrel's privacy claim must be true on every path, including hosted models (doc 18 §4.5
and §5.1). Say: recordings, transcripts and history are stored on the user's computer and
not on Whimbrel's servers; AI runs on the computer or on a model the user chooses; every
way data can leave is listed in the app. Do not write "nothing leaves your computer",
"never leaves" or "we never receive" except about local-only mode by name. With a hosted
model the text of each request goes to that model, and Whimbrel AI requests pass through
Whimbrel's server. There is no encryption at rest yet; do not imply there is. Every claim
about a competitor needs a source URL and the date it was read.

## What Mike decided on 2026-11-11

All six decisions in doc 18 §9 are made. They are settled; do not design around them.

1. **Buyer:** confidential client work (consultants, small-firm lawyers, advisers,
   recruiters) first; AI-tools users second, with their own page; no team seats for now.
2. **Plus:** one paid plan at $12 a month annual, $15 monthly, with unlimited devices and
   an AI allowance, a ceiling the user sets, and fallback to the local model at the
   ceiling. It replaces Cloud at $5 and the Whimbrel AI credit column. Existing Cloud
   subscribers keep their plan. The allowance is sized from a month of measured usage.
3. **Freeze:** bug fixes only on skins, Today layouts, new languages, Away Mode and Android
   beyond parity, until Plus has paying users. Do not start new work in those areas.
4. **Away Mode:** stays in the app as a beta setting; comes off the website and marketing.
5. **Remote access for AI tools:** a tunnel to the user's own computer, never a hosted copy
   of the memory.
6. **Privacy wording:** the rule above.

## Scope, in this order, one PR each

1. **Seven "alternative" pages** (doc 18 N2): Littlebird, Granola, Shadow, Voicenotes,
   Bluedot, Grain, Fellow, under `website/site/pages/compare/`, in the style of the existing
   `granola-alternative.html`. Each has a sourced, dated comparison table and says plainly
   where the other product is better (doc 18 §3 has the rows). Re-read each vendor's pricing
   and privacy pages on the day you write; doc 18's figures are from 2026-11-11 and several
   are tagged unverified. Grain's pricing page rendered malformed then: check it by hand.
   Indexable, in the sitemap, linked from `/compare/`. English only.
2. **Finish the tour video's last scene.** The video ends on a text card because the
   capture harness has no demo data for Settings. Add the fixtures Settings needs to
   `website/demo/capture/`, capture the real "Your data" / Status screen, and add it as a
   scene between Ask and the end card. Then make `scripts/render-demo-video.mjs` record
   reliably: Chrome's built-in video capture cropped the bottom of the frame on the machine
   that made this video, which was worked around by taking timed screenshots and handing
   them to ffmpeg. Port that approach into the script. If a Mac build can be driven with
   demo data, prefer Mac screens: the current ones are the Windows UI.
3. **Home page for the chosen buyer** (doc 18 N1, §5.2). Replace the hero with "Your client
   calls stay on your computer." and the sub-line in §5.2; replace the three chips with the
   three in §5.2; move "No meeting bot" into the feature list; lead the features with the
   client page (Topics) and the Your data screen; link `/for/ai-tools/` as the page for the
   second audience and make both `/for/` pages indexable. Remove Away Mode from the page.
   Leave the pricing section and the pricing page alone; item 6 changes them. Update `tagline` and `description` in `site/config.json` to match,
   then re-translate `/es/` and `/fr/` with `scripts/translate-site.py`. Show Mike the page
   before it merges.
4. **Design doc for the hotkey panel** (doc 18 X1, doc 17 item 2.1): one global hotkey that
   opens a panel to ask about the current window, draft a reply to the thread on screen,
   capture a note to a client's page, and dictate; user-defined actions as saved prompts.
   Design only, in `docs/superpowers/specs/`, covering Mac and Windows, the permission each
   action needs, what text is sent where under each AI setting, and how it behaves in
   local-only mode. No code until Mike approves the design. This is the largest feature gap
   against Shadow, Voicenotes and Littlebird.
5. **Design doc for encryption at rest** (doc 18 X4, doc 17 item 3.5): SQLCipher in
   `core/memory`, key in the OS keychain, one-time migration, measured search latency before
   and after, and what happens to existing installs on both platforms. Design only.

6. **Design doc for the Plus plan** (doc 18 §5.3). Cover: the Stripe product and prices;
   the monthly allowance as a grant into the existing credit ledger; the user-set ceiling
   and optional top-ups at the existing 30%; fallback to the local model when the ceiling is
   reached; the setup choice between "Hosted" and "Private (on this computer only)"; what
   existing Cloud subscribers and unspent credit balances keep; and the new two-column
   pricing page. First measure a month of real token usage from the ledger and propose the
   allowance from it; doc 14's estimates were never measured. Design only: no live Stripe
   change, no price on the site and no entitlement code until Mike approves it.

## Not in scope

Phone recording, building the remote MCP tunnel, consent records, per-client controls,
building Plus, Practice, cloud routines, CRM write-back, SOC 2, new languages, new platforms. Doc 18
§5.4 says why for the ones that are refusals rather than deferrals.

## How to work

- One PR per item, small enough to review in one sitting. Use a fresh agent per task.
- `node website/scripts/build.mjs` must pass with no new warnings before any website PR.
  Merging to `main` deploys the site, so do not merge your own PRs; Mike merges.
- Never run `wrangler deploy`, `wrangler secret put` or live migrations.
- New UI strings are translated before merge, on Mike's word. Website pages in this scope
  stay English.
- Report what you verified and what you could not. If a doc 18 claim is wrong, say so in
  the PR and fix the doc.

## Done means

Seven alternative pages live and sourced; a home page that names the buyer; a tour video
whose every app screen is a real capture, re-renderable with one command; three design
docs (hotkey panel, encryption at rest, Plus) approved by Mike. Then stop and ask Mike
which of doc 18 §6 "Next" to build first.
