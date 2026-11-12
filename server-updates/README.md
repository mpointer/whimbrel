# kestrel-updates-worker

Serves the auto-update manifests and release files straight out of an R2 bucket — no
database, no KV. Mac: `/version.json` and `/Whimbrel-<version>.zip`, written by
`apps/macos/scripts/release-publish.sh`. Windows: `/windows/latest.json` (Tauri updater
format) and `/windows/Whimbrel_<version>_x64-setup.exe`, written by
`.github/workflows/release-windows.yml`. Writers use `wrangler r2 object put`; this
Worker only ever reads.

## Deploy

```bash
npm install
npx wrangler deploy
```

## Test

```bash
npm test
```

## Model defaults

`models/defaults.json` holds the default AI model per hosted provider. Every app reads it daily from `https://updates.whimbrelapp.com/models/defaults.json`, and core's built-in table is the fallback when it can't be fetched. `previous` lists every default the file has ever named for that provider; when the default changes, add the old one there, so a stored copy of it is treated as never chosen and moves to the new default. `family` must be a hyphen-separated part of model ids, and `version` must be 1. Publishing it is a live change, so it needs Mike's OK each time:

    npx wrangler r2 object put kestrel-updates/models/defaults.json --file models/defaults.json --content-type application/json --remote

The route ships with a Worker deploy (`npm run deploy`), which also needs Mike's OK.
