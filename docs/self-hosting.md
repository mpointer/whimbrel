# Self-hosting the relay and the update feed

Whimbrel's desktop apps work without any server: capture, transcription, memory, chat and
routines run on the device. Two small Cloudflare Workers exist for the parts that cross
devices, and both can be run under your own account.

| Worker | Directory | What it does | What it can see |
|---|---|---|---|
| Sync relay | `server/` | Verifies Google sign-in against an allowlist, keeps the account and device registry, creates one Turso database per account, mints device tokens scoped to that database, and passes the wrapped group key between devices during enrollment | Account email, device names and ids, database names, token issuance. Never content: sync payloads are end-to-end encrypted and go from a device straight to the account's own Turso database |
| Update feed | `server-updates/` | Serves `version.json`, `windows/latest.json` and the release files out of an R2 bucket | Nothing about users; it is a static file server |

The hosted deployment is the default the apps point at: `sync.whimbrelapp.com` and
`updates.whimbrelapp.com`, which are custom domains in front of the `*.mpointer.workers.dev`
Workers (the workers.dev hostnames stay live for builds that still pin them). Everything that
ties the Workers to that deployment is a deployment variable or a `routes` entry, not code, so
a fork gets a relay of its own, not the hosted one's users.

## Sync relay (`server/`)

Prerequisites: a Cloudflare account, a [Turso](https://turso.tech) account, and a Google
Cloud project with an OAuth client. On the free tiers both are fine for personal use (Turso
allows 100 databases; each account gets one).

1. **Turso.** Create a group and a control-plane database, then a platform API token:

   ```bash
   turso group create default
   turso db create whimbrel-control-plane --group default
   turso db tokens create whimbrel-control-plane
   turso auth api-tokens mint whimbrel-relay
   ```

   Apply the control-plane schema once: paste `SCHEMA_SQL` from `server/src/accounts.ts`
   into `turso db shell whimbrel-control-plane`. The Worker does not create or migrate its
   own tables.

2. **Google OAuth.** In Google Cloud Console, create a "Desktop app" OAuth client (the Mac
   and Windows apps share it) and, if you build the iOS app, an "iOS" client registered for
   your bundle id. The relay verifies ID tokens against these client ids; the apps hold the
   same ids in their `Secrets.xcconfig` / `Secrets.toml`.

3. **KV namespace and domain.** `wrangler kv namespace create ENROLLMENT_HANDOFF` and put
   the id it prints into `server/wrangler.jsonc` in place of the hosted deployment's. In the
   same file, delete the `routes` block (the hosted relay's custom domain) or replace the
   pattern with a domain whose zone is on your Cloudflare account; without it the Worker
   answers on its `*.workers.dev` hostname only.

4. **Variables and secrets.** `server/wrangler.jsonc` sets `keep_vars: true` and carries no
   vars, so set these on the Worker once (Cloudflare dashboard, or `wrangler secret put`
   for each; secrets work for plain values too) and `wrangler deploy` leaves them alone:

   | Name | Value |
   |---|---|
   | `ALLOWED_GOOGLE_EMAILS` | Comma-separated emails allowed to sign in. Fail-closed: empty means nobody |
   | `GOOGLE_CLIENT_ID` | The desktop OAuth client id |
   | `GOOGLE_IOS_CLIENT_ID` | The iOS client id, or leave unset to reject iOS sign-ins |
   | `TURSO_ORG_SLUG` | Your Turso organization slug |
   | `TURSO_DB_GROUP` | The group from step 1 (must already exist) |
   | `TURSO_DATABASE_URL` | The control-plane database URL (`libsql://…` or `https://…`) |
   | `TURSO_DATABASE_AUTH_TOKEN` | The database token from step 1 |
   | `TURSO_PLATFORM_API_TOKEN` | The platform API token from step 1 (creates per-account databases) |

5. **Deploy and test.**

   ```bash
   cd server && npm ci && npm test && npx wrangler deploy
   ```

6. **Point the apps at it.** Set `SYNC_WORKER_BASE_URL` in `apps/macos/Secrets.xcconfig`
   (write `//` as `/$()/`), `sync_worker_base_url` in `apps/windows/src-tauri/Secrets.toml`,
   and `SYNC_WORKER_BASE_URL` in `apps/mobile/ios/Secrets.xcconfig`, together with the OAuth
   client id and secret from step 2. The apps read these at build time.

## Update feed (`server-updates/`)

Only needed if you distribute your own builds and want the in-app updater to find them.

1. Create the bucket: `wrangler r2 bucket create kestrel-updates` (or change `bucket_name`
   in `server-updates/wrangler.jsonc`). In the same file, delete the `routes` block or put
   your own domain in it, as for the relay.
2. `cd server-updates && npm ci && npm test && npx wrangler deploy`.
3. Point the apps at it: the Mac manifest URL is the `manifestURL` default in
   `apps/macos/Whimbrel/Support/UpdateChecker.swift`; the Windows endpoint is
   `plugins.updater.endpoints` in `apps/windows/src-tauri/tauri.conf.json`;
   `.github/workflows/release-windows.yml` and `website/wrangler.jsonc` carry the same host
   in `UPDATES_URL`.
4. Signing is pinned, on purpose. The Mac updater refuses any download whose code
   signature is not from the Team ID in `UpdateChecker.expectedTeamID`
   (`apps/macos/Whimbrel/Support/UpdateChecker.swift`) and `apps/macos/project.yml`'s
   `DEVELOPMENT_TEAM`; the Windows updater verifies `plugins.updater.pubkey` in
   `tauri.conf.json` (a minisign key; generate yours with `npx tauri signer generate`).
   Replace both with your own before publishing, or the apps you ship will reject the
   updates you serve.
5. Publish releases with `apps/macos/scripts/release-publish.sh` (Developer ID signing,
   notarization, `wrangler r2 object put`) and the `release-windows` workflow (needs the
   `TAURI_SIGNING_PRIVATE_KEY`, `WINDOWS_SECRETS_TOML`, `CLOUDFLARE_API_TOKEN` and
   `CLOUDFLARE_ACCOUNT_ID` repository secrets and a self-hosted Windows runner; the job
   only runs in the upstream repository, so a fork edits that condition).

## What stays with the hosted deployment

The hosted relay's allowlist, Turso organization, OAuth clients, KV namespace, R2 bucket
and signing identities are not in this repository. A clone of the code is a clone of the
code; it does not reach the hosted service's accounts or devices.
