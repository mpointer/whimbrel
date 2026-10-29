# apps/windows

Windows 11 desktop client — Phase 3. Design: [docs/12-windows-app-design.md](../../docs/12-windows-app-design.md).

Tauri v2 shell (Rust process owning capture, tray, notifications, and all calls into `core/`) with a React + TypeScript + Vite UI in WebView2. Capture is UI Automation for active-window text with `Windows.Media.Ocr` fallback, and WASAPI process loopback + microphone for meetings. No C#.

**Status:** M3.2 (shell skeleton), per [docs/superpowers/plans/2026-09-10-m3.2-shell-skeleton.md](../../docs/superpowers/plans/2026-09-10-m3.2-shell-skeleton.md): ambient capture, the Library, sync enrollment and content sync, Settings, Ask, Today, and the tray. Results and measurements: [docs/superpowers/plans/2026-09-10-m3.2-results.md](../../docs/superpowers/plans/2026-09-10-m3.2-results.md). M3.1 (capture spike): [docs/superpowers/plans/2026-09-10-m3.1-capture-spike.md](../../docs/superpowers/plans/2026-09-10-m3.1-capture-spike.md).

Layout:

```
package.json  Tauri CLI entry point (`npm run tauri dev`)
design/       tray-glyph.svg (the tray icon source)
src-tauri/    Rust shell — its own Cargo workspace, path deps into ../../../core
  src/app/      Tauri builder, window lifecycle
  src/tray.rs   tray icon states and the quick-glance popup
  src/restart.rs  panic log and crash test (crash restart, Task 10)
  src/commands/ the Tauri commands the UI calls; schema.rs generates the UI's types
  src/capture/  uia.rs, ocr.rs, audio.rs, window.rs, proc_stats.rs, policy.rs, service.rs
  src/syncing/  enrollment, the Devices pane, content sync
  src/bin/      capture_spike.rs (M3.1 measurement CLI), gen_schema.rs
  tauri.conf.json, capabilities/, icons/ (app icon from apps/macos/design/app-icon-flat*.svg; icons/tray/ from design/tray-glyph.svg)
scripts/      dev-env.ps1, check-no-libsql-core.ps1, render-icons.mjs (app and tray icons)
ui/           React + TypeScript + Vite; src/tokens.css is the ported design system
```

## Running the app

Prerequisites: everything under Building below, plus Node LTS. WebView2 ships with Windows 11.

```powershell
. .\apps\windows\scripts\dev-env.ps1
cd apps\windows
npm install; npm --prefix ui install      # first time only
npm run tauri dev                         # debug build, UI hot-reloads from Vite on :5173
npm run tauri build -- --no-bundle        # release exe, no installer
npm run tauri build                       # release exe plus the NSIS installer
```

The release exe lands in `%LOCALAPPDATA%\kestrel-win-target\release\whimbrel-windows.exe` (or in `release\` under `WHIMBREL_TARGET_DIR` if you set it; see below), and the installer in `...\release\bundle\nsis\Whimbrel_<version>_x64-setup.exe`. A running Whimbrel locks the exe, so quit Whimbrel from the tray before rebuilding.

- **Installing and updates (M3.5):** the installer is per-user (no admin) and installs into `%LOCALAPPDATA%\Whimbrel`, with the data in its `data\` subfolder; its uninstaller removes only the files it installed, so `data\` survives updates and uninstalls. Through 0.9.34 the data was in `%LOCALAPPDATA%\Kestrel`, and the first launch after that moves it (`paths.rs`). An installed release build checks `windows/latest.json` on the updates Worker at launch and daily, downloads a newer installer in the background (minisign-verified against `plugins.updater.pubkey` in `tauri.conf.json`), and shows a "Restart now" banner. Release installers and `latest.json` are built and published by CI on the `v*` tag. Debug builds never check. A release exe run from the target folder does, and accepting an update installs into `%LOCALAPPDATA%\Whimbrel` and relaunches from there; start at login follows, since it is re-registered at every launch.

- **Node and the connectors ship in the installer (M3.4):** `node\node.exe` (a pinned Node LTS) and `connectors\` are installed beside the exe, so connecting Calendar or Gmail needs nothing on the PC. `apps\windows\scripts\prepare-bundle-resources.ps1` stages them (and checks Node against a pinned SHA-256), `prepare-sidecar.ps1 -ForBundle` stages the MCP server, and the release build adds both with `npx tauri build --config src-tauri/tauri.bundle.conf.json`. Under PowerShell 7, `npm run tauri build -- --config x` loses `--config`; use `npx tauri build --config x`. A development build has none of this and falls back to the `node` on `PATH` and `prepare-connectors.ps1`'s copy. To move to a newer Node, change the version and both hashes at the top of `prepare-bundle-resources.ps1`.
- **Sync needs `src-tauri/Secrets.toml`.** Copy `Secrets.toml.example` and fill in the sync Worker URL and the Google OAuth client ID and secret (the same values as `apps/macos/Secrets.xcconfig`). It is gitignored and read at build time, so rebuild after changing it. Without it the app runs, and Settings → Sync says "not configured". Never commit it.
- **Where it runs:** Whimbrel lives in the tray. The icon follows state: normal while capturing, dimmed while capture is off or paused, amber on a sync error. Left-click opens a small popup (the capture toggle, a status line such as "Capturing · Synced 2:05 PM", Open Whimbrel, Quit); right-click shows the menu. Closing the main window destroys it and its WebView2 renderer; Quit is the only way to exit. A second launch focuses the running instance.
- **Start at login:** on by default after the first launch of a release build (an HKCU `Run` entry named Whimbrel). Settings → General turns it off. Debug builds never register themselves.
- **Build the app with `npm run tauri build`, not `cargo build`.** A plain `cargo build --release` produces an exe that loads the Vite dev server (`localhost:5173`) and shows "could not connect" when Vite isn't running. Only the Tauri CLI enables `tauri/custom-protocol`, which serves the embedded `ui/dist`. `cargo build` is still right for `capture-spike` and for tests.

## Data, logs and resetting

Everything local is under `%LOCALAPPDATA%\Whimbrel\data\`:

- `kestrel.sqlite`: the memory store.
- `settings.json`: settings, keyed by the Mac's settings-sync field names.
- `models\`: the semantic-search model (Settings → Semantic search).
- `logs\whimbrel.<date>.log`: the log. Set `WHIMBREL_LOG=debug` for more detail. `logs\crash.log` gets one line per panic, written before the process exits.

Meeting snapshots (one PNG per slide shown in a recorded meeting) are not in `data\`. They go in `Pictures\Whimbrel Snapshots\<yyyy-MM-dd HH.mm> <title>\`, found through the known-folder API, so a Pictures folder redirected to OneDrive is followed. Settings → Meetings can choose another folder. If the folder can't be written, they go in `data\Whimbrel Snapshots\`. Each meeting's folder holds a `.kestrel-snapshots` marker, and retention only deletes a folder that has the marker and nothing but PNGs. Resetting `data\` leaves these folders in place.

Secrets (API keys, Google tokens, the device key and the account key) are in Windows Credential Manager under three services: `com.mpointer.kestrel`, `com.mpointer.kestrel.sync` and `com.mpointer.kestrel.google-oauth`.

To reset this PC to a fresh install:

```powershell
# Quit Whimbrel from the tray first. Turning off Start at login in Settings → General
# before quitting also removes the HKCU Run entry.
Remove-Item -Recurse "$env:LOCALAPPDATA\Whimbrel\data"   # the app itself stays installed
cmdkey /list | Select-String "com.mpointer.kestrel"   # then cmdkey /delete:<target> for each
```

Deleting the device key makes this PC a new device to sync: sign in again and approve it from the Mac.

## Crash testing

Release builds use `panic = "abort"`, so a panic anywhere ends the process, and a supervisor process (`whimbrel-windows.exe --supervise`, started by every release launch) relaunches it: after 1, 2, 5, 10, then 30 s, giving up after six crashes in ten minutes. Exit 0 (Quit) ends supervision; exit 75 asks for an immediate relaunch. Each exit is logged in `logs\supervisor.log`.

`WHIMBREL_CRASH_TEST=actor` (a store job panics), `main` (the UI thread panics) or `restart` (a requested restart) makes a launch do that 5 seconds in, to exercise this path. Only the first child gets it; relaunches run normally. Never set it for normal use.

## Building

Prerequisites: Rust stable (MSVC toolchain), VS 2022 Build Tools with the C++ workload including **C++ CMake tools** and **C++ Clang tools** (whisper.cpp needs CMake; bindgen needs libclang). No Visual Studio IDE.

```powershell
. .\apps\windows\scripts\dev-env.ps1     # puts cmake + libclang on PATH, short CARGO_TARGET_DIR
cd apps\windows\src-tauri                 # must build from here — see below
cargo build --release
cargo test --lib
..\scripts\check-no-libsql-core.ps1
```

Three things that are not obvious:

- **Build from `src-tauri/`, not with `--manifest-path`.** `.cargo/config.toml` there sets `+crt-static`, and Cargo only reads config from the current directory's ancestry. Without it the final link fails (LNK2038/LNK2005): sherpa-onnx ships Windows libs only as static-CRT (`/MT`) builds, and everything else defaults to `/MD`.
- **Short target directory.** whisper.cpp's CMake try-compile runs under MSBuild, whose FileTracker fails (`FTK1011`) once a path passes 260 characters; a deep checkout (a git worktree) crosses it. `dev-env.ps1` sets `CARGO_TARGET_DIR=%LOCALAPPDATA%\kestrel-win-target`. That folder grows fast (a debug and a release build together pass 100 GB), so to keep it on a bigger drive set `WHIMBREL_TARGET_DIR` once, for example `setx WHIMBREL_TARGET_DIR E:\kestrel-win-target`, and `dev-env.ps1` will use it. It is deliberately not `CARGO_TARGET_DIR`: the self-hosted CI runner runs as the same user and its jobs need that unset.
- **Never enable libsql's `core` feature** anywhere in this graph (docs/12 §3). `check-no-libsql-core.ps1` verifies.

The UI's TypeScript types are generated from the Rust DTOs: `cargo run --bin gen-schema`, then `npm --prefix ui run gen:types`. A test fails when the committed schema is stale.

## capture-spike

```powershell
$spike = "$env:LOCALAPPDATA\kestrel-win-target\release\capture-spike.exe"
& $spike windows --exe msedge.exe                     # windows + §4.3 app identity
& $spike measure --exe msedge.exe --title Gmail --label "Edge: Gmail thread"
& $spike ocr --foreground --delay 3 --request-access
& $spike sessions                                      # per-process audio sessions
& $spike models                                        # whisper + diarization models via core/api::models
& $spike record --exe ms-teams.exe --secs 900          # process loopback + mic; Ctrl+C stops
& $spike transcribe --sys <system-audio.wav> --mic <mic-audio.wav>
& $spike ambient --secs 600                            # NFR-PRF-1 idle-capture CPU/RSS
& $spike cpu --exe whimbrel-windows.exe --secs 1800     # a running process tree's CPU and memory
```

`uia`, `ocr`, `measure` and `ambient` never persist captured text — they report counts and timings (`--show N` prints a prefix to the console). `record` writes WAVs to `%LOCALAPPDATA%\Whimbrel\data\recordings\`, as the product will.
