# Windows release evidence: v0.9.23

**Status:** partial. The build and publish rows are proven from CI and git. The installer's hash is **not recorded** and is marked so below. The in-place update was proved later, from v0.9.29 to v0.9.31: see `windows-v0.9.31.md`. Plan: `docs/superpowers/plans/2026-10-17-m3.5-windows-release-evidence.md`.

**Published:** the release job finished 2026-10-17T05:12:07Z (UTC); it started 2026-10-17T04:47:29Z.
**Source commit:** `cbaa937364f7a00873de542b6436d7353f10b1dc` (`release: prepare 0.9.23 (#298)`).
**Tag:** `v0.9.23`, annotated; `git rev-parse v0.9.23^{}` gives the commit above, which is the run's `headSha`.
**Signing:** the installer is **not Authenticode-signed** (decision 2026-10-01). The update is signed with the updater's minisign key; the job's "Build and sign the installer" step produced the `.sig`. SmartScreen can warn on a first manual install.

## Run

GitHub Actions run 39003883679, workflow `release-windows`, on the self-hosted Windows runner, triggered by the tag push. Result: success, 24 minutes 38 seconds. https://github.com/mpointer/kestrel/actions/runs/39003883679

| Step | Result |
|---|---|
| Release tag is vX.Y.Z | success |
| Tag matches the app version | success |
| Write the sync configuration file (embedded at build time) | success |
| Install | success |
| Build the MCP server program (the installer carries it) | success |
| Stage Node and the connectors (the installer carries them) | success |
| Build and sign the installer | success |
| Write latest.json | success |
| Publish installer, then latest.json | success |
| Published manifest serves this version | success |
| Remove the sync configuration file | success |

"Published manifest serves this version" fetches the live `windows/latest.json` from the updater endpoint after publishing and fails the job if its `version` is not `0.9.23`. It checks the version only, not the installer's URL, signature or bytes.

## Artifact

- Filename: `Kestrel_0.9.23_x64-setup.exe` (from the build log: `bundle\nsis\Kestrel_0.9.23_x64-setup.exe`, with its `.sig`).
- Carries (steps above): the app, `kestrel-mcp-server.exe`, a pinned Node (v22.23.3, SHA-256 checked when staged) and the connectors.
- **Size: not recorded.** **SHA-256: not recorded.** The workflow does not log them and the installer was not downloaded for this document. Fill both from a download. (`latest.json` carries the updater's signature for the installer, not a hash, so there is no published hash to compare with.)

## In-place update (the M3.5 gate): **proven on v0.9.31**, see `windows-v0.9.31.md`

This table was never filled from a v0.9.23 install. The check was run instead from a v0.9.29 install updating to v0.9.31, recorded in `windows-v0.9.31.md`. The procedure below is kept as the one that was followed.

CI cannot prove this: it needs an installed older build and a running app. v0.9.23 is the latest release, so the first release it can be proved on is the next one, from a v0.9.23 install. Run it once, then fill the table from what was seen.

Procedure:

1. Install v0.9.23 from its installer. Start Kestrel; leave it running. (Optional: turn the MCP server on in Settings, to also check that the installer is not blocked by the sidecar.)
2. After the next release is published, wait for the update banner (the app checks at launch and daily), or restart Kestrel to check at launch.
3. Click Restart now.
4. Record each row below.

| Observation | Seen? | Notes |
|---|---|---|
| The banner offered the new version | not yet | |
| Restart now closed the app and the installer ran without a prompt | not yet | |
| The new exe relaunched on its own | not yet | |
| The version shown in the app is the new one | not yet | |
| Library, settings and sync enrollment were intact | not yet | |
| Start at login still points at the new exe | not yet | |
| With the MCP server on: the installer was not blocked by `kestrel-mcp-server.exe` | not yet (optional) | |
| Log lines (`%LOCALAPPDATA%\Kestrel\logs`): `update downloaded; waiting for Restart now` and `installing update` | not yet | |
