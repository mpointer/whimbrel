# server

Cloudflare Worker: verifies Google ID tokens against a fail-closed sign-in allowlist, maintains a control-plane account/device registry, provisions one isolated Turso database per account, and mints device tokens scoped to only that account's database via `POST /v1/auth/google`. The wrapped-group-key handoff (KV store-and-forward) is implemented but not yet wired into any route — that lands with the full enrollment-flow plan. Not a data-plane component — sync payload data (`capture_item`/`meeting`/`transcript_segment`/`summary`) goes straight from a device to its account's own Turso database once the device holds a token from this Worker. See `docs/superpowers/specs/2026-07-29-multi-device-sync-accounts-design.md` for the design and `docs/superpowers/plans/2026-07-29-sync-auth-worker.md` / `docs/superpowers/plans/2026-07-29-sync-worker-account-isolation.md` for what's implemented so far.

## Before deploying

Deployment variables, Turso and OAuth setup, and pointing the apps at your own relay: [docs/self-hosting.md](../docs/self-hosting.md).

- `SCHEMA_SQL` (in `src/accounts.ts`) uses `CREATE TABLE IF NOT EXISTS` — it will not retroactively add new columns to an already-created control-plane table. Run it once against a fresh database; if the control-plane database already exists from an earlier version of this Worker, apply the missing columns manually first.
- `TURSO_DB_GROUP` (a Worker variable; `wrangler.jsonc` sets `keep_vars` and carries none of the deployment values) must name a Turso group that already exists (`turso group create <name>`) — this Worker does not create it.
- Every account gets its own Turso database (`whimbrel-sync-<uuid>`), created on that account's first sign-in via Turso's Platform API. The free tier supports up to 100 databases.
- `POST /v1/devices` and `POST /v1/devices/:id/approve` are for the native app's device-approval flow — there is no native caller yet as of this plan; `core/ffi`'s crypto bindings and the native enrollment UI are separate, later plans.
- `GOOGLE_IOS_CLIENT_ID` (a Worker variable, unset by default) is the iOS app's Google OAuth client ID, accepted as a second ID-token audience alongside `GOOGLE_CLIENT_ID`. Empty/unset means iOS tokens are rejected. Production needs this set and a deploy; the `ALLOWED_GOOGLE_EMAILS` allowlist still applies.
