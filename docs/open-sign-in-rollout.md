# Open sign-in rollout

Placeholders in `<angle brackets>` are yours to fill in. Run these in order.

## 1. Run the migration

Once, against the live registry. Take the database name from the sync Worker's `TURSO_DATABASE_URL`
(`libsql://<registry-db>-<org>.turso.io`).

```bash
turso db shell <registry-db> < server/migrations/0001_open_sign_in.sql
```

The `ALTER TABLE` lines fail if run twice. Do not re-run the file.

Optional: mark Mike's existing phone as a phone. Rows from before the migration have a NULL
`platform`, which counts as a computer. That is harmless while his email is in
`UNLIMITED_EMAILS`, but it makes the device list honest:

```bash
turso db shell <registry-db> "SELECT d.id, d.name FROM device d JOIN account a ON a.id = d.account_id
  WHERE a.email = 'mpointer@gmail.com' AND d.platform IS NULL AND d.status = 'approved';"
turso db shell <registry-db> "UPDATE device SET platform = 'ios' WHERE id = '<phone id>' AND platform IS NULL;"
```

## 2. Set the secrets

Set all three with `wrangler secret put`, and only that way: a dashboard var with the same name
conflicts with the secret. A secret takes effect as soon as it is put, with no deploy.

```bash
cd server
npx wrangler secret put RESEND_API_KEY      # a Resend key restricted to futuremeanswered.com
npx wrangler secret put UNLIMITED_EMAILS    # mpointer@gmail.com,<other emails>
npx wrangler secret put OPEN_SIGN_IN        # false
```

`UNLIMITED_EMAILS` must be set now, before step 4 flips `OPEN_SIGN_IN`. With `OPEN_SIGN_IN=true`
the free device limit is on, and every account not in `UNLIMITED_EMAILS` is held to one computer
and one phone. While `OPEN_SIGN_IN` is false there is no device limit at all.

Check in the Resend dashboard that the key's account has `futuremeanswered.com` verified, or the
warning emails will fail.

## 3. Deploy the Workers

```bash
cd server && npx wrangler deploy            # sync Worker; also adds the daily cron "0 9 * * *"
cd ../server-updates && npx wrangler deploy # updates Worker; binds CLIENTS -> dataset whimbrel_clients
```

**Smoke check**, right after the sync Worker deploys. On Mike's Mac, open Settings → Devices (it
lists devices with `POST /v1/devices`) and let sync renew its token (`POST /v1/auth/google`). Both
must succeed. In `npx wrangler tail` (from `server/`), look for 200s on those two routes and no
`account_deleting` (503) or 502. A 503 `account_deleting` on every request means the conditional
`UPDATE … RETURNING` writes are misbehaving: roll back with `npx wrangler rollback`.

## 4. Release the apps

Order matters:

1. The sync Worker (step 3).
2. The Mac and iPhone builds.
3. Check that `UNLIMITED_EMAILS` is set (step 2). Then flip the switch:
   `cd server && npx wrangler secret put OPEN_SIGN_IN` and enter `true`. This opens sign-in to any
   Google account **and turns on the free device limit**. It takes effect at once; no deploy is needed.
   To undo, put `false`.
4. Right after the flip, sign in once with a Google account that has never used Whimbrel. It must be
   approved, not `429 rate_limited`: a 429 on a first sign-in from a fresh IP means the per-IP
   reservation (`INSERT … RETURNING` into `account_creation`) is misbehaving. Put `false` back and
   investigate.

## 5. The three numbers

**Weekly clients** (Analytics Engine; API token needs Account Analytics Read):

```bash
curl "https://api.cloudflare.com/client/v4/accounts/$CF_ACCOUNT_ID/analytics_engine/sql" \
  -H "Authorization: Bearer $CF_API_TOKEN" \
  --data-binary @server-updates/scripts/weekly-clients.sql
```

**Retained second devices.** Accounts whose second approved device (by enrolment time) is at least
30 days old and was seen in the last 7 days:

```bash
turso db shell <registry-db> "
SELECT COUNT(*) AS retained_second_devices
FROM (
  SELECT account_id, enrolled_at_unix_ms, last_seen_unix_ms,
         ROW_NUMBER() OVER (PARTITION BY account_id ORDER BY enrolled_at_unix_ms) AS rn
  FROM device
  WHERE status = 'approved'
)
WHERE rn = 2
  AND enrolled_at_unix_ms <= (strftime('%s','now') - 30 * 86400) * 1000
  AND last_seen_unix_ms   >= (strftime('%s','now') - 7 * 86400) * 1000;"
```

**Waitlist, by want.** Each row's `wants` is a JSON array; this counts accounts per want:

```bash
turso db shell <registry-db> "
SELECT j.value AS want, COUNT(DISTINCT w.account_id) AS accounts
FROM waitlist w, json_each(w.wants) j
GROUP BY j.value
ORDER BY accounts DESC;"
```

Devices that existed before the migration have a NULL `last_seen_unix_ms` until they next sync, so the
retained count starts low.

## 6. The Windows hand-off

`apps/windows` is untouched. If the Windows app talks to the Worker through the Rust core's
`WorkerClient` (`core/api`), every error below is already mapped to a message in
`core/api/src/sync_ops/enrollment.rs` (`user_message`); the app still needs the screens.

- **`platform: "windows"`** goes in the JSON body of three routes, the only ones that read it:
  `POST /v1/auth/google`, `POST /v1/devices/swap` and `POST /v1/waitlist`. (`POST /v1/devices` only
  lists devices and ignores it.)
- **Update-check headers.** On every manifest poll of `https://updates.whimbrelapp.com/windows/latest.json`,
  send `X-Whimbrel-Client: <32 hex chars, random per install per ISO week>`, `X-Whimbrel-Platform: windows`
  and `X-Whimbrel-Version: <major.minor.patch>`. A missing or malformed header means the poll is not counted.
- **Error codes** the app must handle, by route:

  | Status and body | From | Meaning |
  |---|---|---|
  | `409 {"error":"device_limit","kind":"computer"\|"phone","devices":[{id,name,platform}]}` | `/v1/auth/google`, `/v1/devices/swap`, `/v1/devices/<id>/approve` | Over the free limit. Nothing was enrolled or approved; show the limit screen |
  | `409 {"error":"desktop_required"}` | `/v1/auth/google` | A phone can't be an account's first device |
  | `429 {"error":"rate_limited"}` | `/v1/auth/google`, `/v1/waitlist` | Transient; try again shortly |
  | `503 {"error":"account_deleting"}` | Any route that signs in | The account is being deleted; transient |
  | `409 {"error":"reset_in_progress"}` | `/v1/devices/swap`, `/v1/account/reset` | Another reset is running; transient |
  | `403 {"error":"other_device_active","devices":[{id,name,platform}]}` | `/v1/account/reset` | A device was seen in the last 7 days; approve from it instead |
  | `403 {"error":"reset_requires_computer"}` | `/v1/account/reset` | Only a computer can start fresh |
  | `400 {"error":"reset_confirm_required"}` | `/v1/account/reset`, `/v1/account/delete` | The body lacked `confirm: true` |
  | `400 {"error":"swap_kind_mismatch"}` | `/v1/devices/swap` | A computer can only replace a computer, a phone a phone |
  | `400 {"error":"wants_required"}` | `/v1/waitlist` | No valid `wants` were sent |
  | `400 {"error":"device is resetting"}` | `/v1/devices/<id>/approve` | That device is mid-reset |

  The older errors still apply (`403 device revoked`, `404 device not found`, `400 device already approved`)
  and are already in the Rust mapping.
- **Device status `resetting`.** The device list (`POST /v1/devices`) can return `status: "resetting"`
  besides `approved`, `pending` and `revoked`. Treat it like pending; `/v1/auth/google` returns
  `{"status":"pending"}` for it.
- **Screens**, matching the Mac copy (`apps/macos/Whimbrel/Support/SyncState.swift`):
  - Limit: "Whimbrel's free sync includes one computer and one phone. `<name>` is your synced computer."
    Without a name, "Your other computer".
  - Swap: button "Use this computer instead" calls `POST /v1/devices/swap` with `replaceDeviceId`, then shows
    "Approve this computer on `<name>` or on your phone."
  - Waitlist: button "Join the waitlist", `POST /v1/waitlist`, four checkboxes sending these `wants`:

    | Label | `wants` |
    |---|---|
    | More devices | `more_devices` |
    | Routines that run while my computer is closed | `routines_while_closed` |
    | Notifications on my phone | `phone_notifications` |
    | Backups | `backups` |

  - Fresh start: while a swap is pending, "`<name>` isn't available?" leads to a destructive "Start fresh"
    (`POST /v1/account/reset` with `confirm: true`). Warning: "No other device can unlock your synced data.
    Starting fresh makes a new encryption key and clears the copy on the sync server. This computer uploads
    what it has; anything that was only on `<name>` or only on the server will not come back. Nothing on any
    of your devices is deleted."
  - Delete: "Delete my sync account…" in Devices settings (`POST /v1/account/delete` with `confirm: true`).
    Alert: "Delete your sync account? This deletes the encrypted copy on the sync server and signs out every
    device. Nothing on your devices is deleted." On success, clear every local sync credential.
