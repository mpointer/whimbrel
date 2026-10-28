# Windows release evidence: v0.9.31

**Status:** the in-place update is **proven**. An installed v0.9.29 updated itself to v0.9.31 through the updater on the runner PC (FEYNMAN) on 2026-10-25. This is the M3.5 gate check that `windows-v0.9.23.md` left open. The two on-screen rows were confirmed by the user. Plan: `docs/superpowers/plans/2026-10-17-m3.5-windows-release-evidence.md`.

**Published:** the release job finished 2026-10-24T21:29:52Z (UTC); it started 2026-10-24T21:15:09Z. `windows/latest.json` gives `pub_date` 2026-10-24T21:28:43Z.
**Source commit:** `e4722ef1647cb46118106897dad08fc2ae3f384a` (`release: 0.9.31 (#383)`).
**Tag:** `v0.9.31`, annotated; `git rev-parse v0.9.31^{}` gives the commit above, which is the run's `headSha`.
**Signing:** the installer is **not Authenticode-signed** (decision 2026-10-01). The update is signed with the updater's minisign key, which the app checks before installing.

## Run

GitHub Actions run 40133384211, workflow `release-windows`, on the self-hosted Windows runner, triggered by the tag push. Result: success, 14 minutes 43 seconds. https://github.com/mpointer/kestrel/actions/runs/40133384211

| Step | Result |
|---|---|
| Release tag is vX.Y.Z | success |
| Tag matches the app version | success |
| Secrets.toml (sync configuration, embedded at build time) | success |
| Install | success |
| Build the MCP server program (the installer carries it) | success |
| Stage Node and the connectors (the installer carries them) | success |
| Build and sign the installer | success |
| Write latest.json | success |
| Publish installer, then latest.json | success |
| Published manifest serves this version | success |
| Remove Secrets.toml | success |

## Artifact

- Filename: `Kestrel_0.9.31_x64-setup.exe`.
- Size: 54,041,406 bytes. SHA-256: `9c117c4142dcdb3060ee084fb87c0391a85052597c159e3e3215ec81a1b17b42`. Both are from the "Write latest.json" step's log line, which hashes the file the next step uploads.

## In-place update (the M3.5 gate): **proven**

The starting point was v0.9.29, installed on 2026-10-23 from `Kestrel_0.9.29_x64-setup.exe` (SHA-256 checked against that release's log), with Start at login pointing at the installed copy. Kestrel was left running as usual. The PC skipped v0.9.30: the first update it saw was v0.9.31.

Times are UTC, from `%LOCALAPPDATA%\Kestrel\logs\kestrel.2026-10-25.log`:

```
12:06:14  Kestrel starting version="0.9.29"
12:06:20  update downloaded; waiting for Restart now version=0.9.31
12:16:18  the update banner now offers 0.9.31
13:52:10  installing update version=0.9.31
13:52:44  Kestrel starting version="0.9.31"
13:52:45  device identity loaded device="sFpWOJmD" created=false
13:52:46  start at login applied recorded=Some(true)
13:52:46  sync status status=ready
13:52:56  content sync round ok trigger=BecameReady
```

| Observation | Seen? | Notes |
|---|---|---|
| The banner offered the new version | yes | Log, 12:16:18. The user clicked Restart now from it. |
| Restart now closed the app and the installer ran without a prompt | yes | No prompt, only the progress window (confirmed by the user). | `installing update` at 13:52:10, and the new version starting 34 seconds later. The updater runs NSIS in passive mode, which shows only a progress window. |
| The new exe relaunched on its own | yes | `Kestrel starting version="0.9.31"` at 13:52:44, from `C:\Users\micha\AppData\Local\Kestrel`. The installed `kestrel-windows.exe` reports version 0.9.31. |
| The version shown in the app is the new one | yes | Settings → About → Version shows 0.9.31 (confirmed by the user). |
| Library, settings and sync enrollment were intact | yes | Same device identity (`created=false`). Capture came back enabled, and sync reached `ready` and completed a content round. The calendar connector synced its 1,444 meetings. The first retention sweep after the update deleted no orphan recordings. No WARN or ERROR lines after the update. |
| Start at login still points at the new exe | yes | The HKCU `Run` value `Kestrel` is `C:\Users\micha\AppData\Local\Kestrel\kestrel-windows.exe`: the same path, now holding the 0.9.31 exe. The app logged `start at login applied recorded=Some(true)`. |
| With the MCP server on: the installer was not blocked by `kestrel-mcp-server.exe` | not checked | The MCP server was not turned on for this run. |
| Log lines `update downloaded; waiting for Restart now` and `installing update` | yes | Above. |
