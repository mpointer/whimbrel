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
