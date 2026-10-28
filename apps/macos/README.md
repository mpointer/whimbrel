# apps/macos

SwiftUI shell + native capture services — M1.1 capture spike (docs/05-roadmap.md).

Implements, as a menu-bar app with no production capture/storage logic yet:

- **FR-CAP-5** — menu-bar capture on/off toggle, capture state always visible via the menu-bar icon.
- **FR-MTG-3 / FR-MTG-10** — system audio (ScreenCaptureKit) + microphone (AVAudioEngine) captured as two separate timestamped files; system-audio failures (e.g. another process already holding the tap) degrade to mic-only with a visible warning instead of losing the recording.
- **FR-CAP-1…3** — Accessibility-API text extraction of the frontmost window on focus change and on debounced content-change notifications, logged as JSON Lines with app bundle ID, window title, URL (when available), and timestamp. No screenshots persisted.
- **FR-CAP-2** — Vision-OCR fallback for windows with little/no AX text (e.g. canvas-rendered Electron apps).
- **FR-ONB-1 / FR-ONB-2** — staged permission checklist (Screen & System Audio Recording, Microphone, Accessibility) with deep links to System Settings; the app runs with partial grants and shows which capability is inactive and why.

Also, as of M1.2 groundwork, the app links `core/ffi` as a static library and calls into it (`CoreBridge.swift`) — see "Core (Rust) integration" below.

## Prerequisites

- Xcode 15+ (tested with Xcode 26.5) with the macOS SDK.
- Rust/cargo on `PATH` (or at `~/.cargo/bin`) — a build phase runs `cargo build --release -p ffi` automatically before every Xcode build, so `core/` doesn't need to be built manually first.
- CMake on `PATH` (`brew install cmake`) — `core/ffi` depends on `core/transcribe`, which builds whisper.cpp from source via `whisper-rs-sys`; that build needs CMake. You'll only notice this the first time `cargo build -p ffi` actually compiles (not just relinks) whisper.cpp.
- Network access at build time — `core/ffi` also depends on `core/diarize`, which pulls in `sherpa-onnx-sys`. Unlike whisper.cpp, this needs no CMake; instead it downloads a prebuilt ONNX Runtime static library archive (~19.5MB) from GitHub during the build. The first `cargo build -p ffi` (or Xcode build) that touches this dependency needs network access — an offline/air-gapped build will fail at that step with a confusing network error rather than a clear "no network" message.
- [xcodegen](https://github.com/yonaskolb/XcodeGen) is only needed if you change `project.yml` and want to regenerate `Whimbrel.xcodeproj` (`brew install xcodegen && xcodegen generate`). The generated project is committed, so you don't need it just to build.
- An Apple ID added to Xcode (Settings → Accounts) with automatic signing selected for the `Whimbrel` target's personal team. A free account is enough. Without this, the app is ad-hoc signed and **TCC permission grants will not persist across rebuilds** — you'd have to re-grant Screen Recording/Accessibility every time you rebuild.
- App Sandbox is intentionally OFF for this spike (ScreenCaptureKit system audio, cross-process Accessibility, and the local file writes below all need it off). Not for App Store distribution — see NFR-SEC-3.

## Core (Rust) integration

`apps/macos` now genuinely calls into `core/` rather than just bundling a stub (NFR-PRT-1):

- The Xcode project links `core/target/release/libwhimbrel_core.a` (built automatically by a prebuild script phase) and imports its C header (`core/ffi/include/whimbrel_core.h`) via a bridging header.
- `Whimbrel/Support/CoreBridge.swift` wraps the C-ABI (open/close a store, ingest a JSONL file, keyword search, semantic search, chat, digest, transcription) in a small Swift API.
- Since M3.0 (2026-09) the OS-independent orchestration lives in `core/api` and reaches Swift through the same bridge: transcript post-processing, retention/orphan sweeps, model download (start/poll/cancel), update-manifest and SHA-256 checks, starter prompts and urgency, hosted model lists, meeting-workspace state, routine write-action plan/settle, connector sync request/apply, the connected-account registry, OAuth (PKCE, loopback, exchange, refresh), the sync Worker client, and the settings-sync iteration rules. Swift keeps timers, Keychain/UserDefaults, `ASWebAuthenticationSession`, the MCP HTTP calls and UI. Per-item table: `docs/12-windows-app-design.md` §5.
- `Whimbrel/Support/CoreSelfTest.swift` runs automatically when the debug window opens: ingests a small fixture (plus the real `screen-text.jsonl` if present), runs both search paths, and writes a plain-text pass/fail report to `~/Library/Application Support/Whimbrel/core-selftest.txt`. Check that file if you want to confirm the Rust core is actually working without digging through Xcode's console.

**Important:** `core/ffi`'s `Cargo.toml` must build **only** `["staticlib", "lib"]` — no `"cdylib"`. If a `.dylib` gets built alongside the `.a` in `core/target/release/`, the linker prefers the `.dylib` for `-lwhimbrel_core` (standard `ld` behavior when both exist with the same name), so the app silently links dynamically instead of statically. That's harmless until the app is properly signed with Hardened Runtime on — at that point Library Validation refuses to load a dylib that isn't signed with the same Team ID as the app, and the app crashes on launch with a `dyld` error citing "different Team IDs." If you ever see that crash, check `core/ffi/Cargo.toml`'s `crate-type` first, and delete any stale `.dylib` under `core/target/release/` (Cargo doesn't clean up old outputs when `crate-type` changes) before rebuilding.

**Also important:** `project.yml`'s `OTHER_LDFLAGS` includes `-lc++` — `core/ffi` now links `core/transcribe`, which builds whisper.cpp/ggml (C++), and nothing else in this otherwise Swift/C project pulls the C++ runtime in automatically. If you ever see `ld: symbol(s) not found` for symbols like `___cxa_throw` or `___gxx_personality_v0`, this is why.

### Transcription (FR-MTG-4)

The debug window has a **"Transcribe Latest Meeting"** button: it finds the most recent `system-audio-*.caf`/`mic-audio-*.caf` pair (or mic-only, if system audio degraded — FR-MTG-10), converts each to 16kHz mono WAV (`Whimbrel/Support/AudioConversion.swift`, via `AVAudioConverter` — whisper.cpp needs exactly that format and core/transcribe deliberately doesn't resample), transcribes each separately via `core/transcribe`'s whisper.cpp integration (mic = "Me" always; system audio gets real per-speaker "Speaker N" labels via `core/diarize` when diarization models are configured, or "Others" otherwise), merges them in chronological order with bleed-through filtering in `core/transcribe::postprocess` (via `whimbrel_transcript_postprocess`), and writes `transcript-<timestamp>.txt` into the spike folder.

This needs a GGML whisper model at `~/Library/Application Support/Whimbrel/models/ggml-tiny.en.bin` — it's not bundled with the app (same ~75MB file `core/transcribe/README.md` documents fetching; you can symlink or copy it there):

```bash
mkdir -p ~/"Library/Application Support/Whimbrel/models"
curl -L -o ~/"Library/Application Support/Whimbrel/models/ggml-tiny.en.bin" \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin
```

Without the model present, the button reports a clear "model not found" status rather than failing silently or crashing.

### Google Calendar connector (MCP integration)

The `mcp-servers/google-calendar/` MCP server (a Node.js tool) integrates read-only Google Calendar access into the agent runtime. It's spawned as a child process by the `mcp-host` runtime (`CoreBridge.swift`'s `McpHost`, driven by `CalendarConnectorState.swift`). Its compiled `dist/` and its `node_modules/` are bundled into the `.app` as XcodeGen folder-reference resources (`project.yml`'s `sources`, copied into `Contents/Resources/dist` and `Contents/Resources/node_modules` by Xcode's own Copy Bundle Resources phase, so the connector itself is self-contained — no source checkout needed on the machine running the built app). A system `node` binary is still a separate, required runtime dependency that is **not** bundled (`McpNodeLocator.swift` looks for it at the usual Homebrew/system paths, falling back to actually probing whether `env node --version` resolves via the process's inherited `PATH` — covers nvm/fnm installs); if it's missing, connecting reports a clear "Node.js isn't installed" error rather than failing silently. `dist/` is checked into git (rebuilding requires `cd mcp-servers/google-calendar && npm install && npm run build`), but there's no drift-detection if `src/` changes without a rebuild — check `git status` in that directory after editing its source. `node_modules/` itself is gitignored, so it must exist on disk before building the app (`cd mcp-servers/google-calendar && npm ci --omit=dev`); `scripts/release-publish.sh` does this automatically before archiving, but a plain Xcode build from a fresh checkout needs it run manually first.

A native UI entry point exists: Settings → Connectors (`ConnectorsSettingsPane` in `SettingsView.swift`) has a "Connect Google Calendar" button, backed by `CalendarConnectorState`, that runs the OAuth consent flow and then proves the connector actually works by calling `list_events`, showing live connected/connecting/error status. The OAuth flow (consent screen, token storage) is handled by a second, connector-scoped instance of the reusable `GoogleOAuthHelper.swift` (loopback listener + Keychain; `calendar.readonly` scope, a distinct Keychain account from sync's own `openid email` instance).

### Outlook 365 connector (MCP integration)

The `mcp-servers/outlook365/` MCP server integrates read-only Outlook Calendar and Mail access via Microsoft Graph, the same way the Google Calendar connector does (`CoreBridge.swift`'s `McpHost`, driven by `Outlook365ConnectorState.swift`). Its compiled `dist/` and `node_modules/` are bundled the same way Gmail's are — `project.yml`'s `subpath: outlook365` folder-reference resources, landing at `Contents/Resources/outlook365/dist` and `Contents/Resources/outlook365/node_modules`. The same system `node` dependency (`McpNodeLocator.swift`) applies. `node_modules/` itself is gitignored, so it must exist on disk before building the app (`cd mcp-servers/outlook365 && npm ci --omit=dev`); `scripts/release-publish.sh` does this automatically before archiving, but a plain Xcode build from a fresh checkout needs it run manually first.

OAuth (consent screen, token storage) is handled by `MicrosoftOAuthHelper.swift`, built on the same shared loopback/PKCE mechanism `GoogleOAuthHelper.swift` uses (`OAuthLoopbackFlow.swift`) but against Azure AD's `common` endpoint and with no client secret (a public client). To connect a real account:

1. Register an app in the [Azure Portal](https://portal.azure.com) → App registrations → New registration → "Accounts in any organizational directory and personal Microsoft accounts" → platform type "Mobile and desktop applications" → redirect URI `http://localhost`.
2. Copy `Secrets.xcconfig.example` to `Secrets.xcconfig` if you haven't already, and fill in `MICROSOFT_OAUTH_CLIENT_ID` with that app's client ID.
3. `xcodegen generate` picks this up automatically.

A native UI entry point exists at Settings → Connectors → "Add Microsoft Account", which runs the OAuth flow and then proves the connector works by calling `list_events`.

### MCP server process (Settings → MCP Clients)

The real `whimbrel-mcp-server` binary (`core/mcp-server/src/bin/whimbrel-mcp-server.rs`, serving `http://127.0.0.1:4319/mcp` with Task 3's bearer-token auth enforced) is bundled into the `.app` the same way the Google Calendar connector's `dist`/`node_modules` are bundled above — an XcodeGen resource entry in `project.yml` (`type: file` rather than `type: folder`, since it's a single binary), copied into `Contents/Resources/whimbrel-mcp-server` by Xcode's own Copy Bundle Resources phase. `project.yml`'s "Build Rust core" prebuild script runs `cargo build --release -p mcp-server --bin whimbrel-mcp-server` before that copy phase, so the binary exists on disk by the time Xcode needs it — no separate manual build step. Settings → MCP Clients has an "Enable MCP Server" toggle (`McpServerProcessManager.swift`) that spawns/stops this binary as a real, persistent child process — unlike this codebase's other `Process()` uses (`McpNodeLocator`, `ModelDownloadService`), which are one-shot, run-to-completion calls, this one has to actually be torn down: explicitly on toggle-off, and on full app quit via an `NSApplication.willTerminateNotification` observer (child processes are not killed automatically when their parent quits). The toggle's on/off state persists across relaunches, so the server resumes automatically if it was left enabled in a previous session.

### Sync configuration

The native sync enrollment UI (Google Sign-In, waiting screen, and Devices settings panel) integrates with the Cloudflare Worker that manages device enrollment and group key distribution. To test it locally, configure your credentials in `Secrets.xcconfig`:

1. Copy `Secrets.xcconfig.example` to `Secrets.xcconfig`. Without it, sync is simply unavailable — the app degrades gracefully rather than failing to build.
2. Fill in `GOOGLE_OAUTH_CLIENT_ID` and `GOOGLE_OAUTH_CLIENT_SECRET` — these are the same Google Cloud OAuth credentials used by the Calendar connector (same Google Cloud project), but the sync UI uses a separate OAuth instance with a different scope (`openid email`).
3. Set `SYNC_WORKER_BASE_URL`. The Worker is deployed at `https://sync.whimbrelapp.com` — use that for real sync against the live control-plane/data-plane databases. For local Worker-code changes, point at a local `wrangler dev` server instead: run `cd server && npx wrangler dev` (defaults to `http://localhost:8787`). Either way, write the value with `//` escaped — xcconfig treats a literal `//` as a comment marker, so use `https:/$()/sync.whimbrelapp.com` or `http:/$()/localhost:8787` (as in `Secrets.xcconfig.example`).

`xcodegen generate` picks up the `Secrets.xcconfig` reference automatically — it's already wired into the committed `project.yml` and `Whimbrel.xcodeproj`, so no regeneration is needed after editing the secrets file.

## Build & run

Open in Xcode (recommended for the permission prompts and console output):

```bash
open apps/macos/Whimbrel.xcodeproj
```

Select the `Whimbrel` scheme → Run (⌘R). Or from the command line:

```bash
cd apps/macos
xcodebuild -project Whimbrel.xcodeproj -scheme Whimbrel -configuration Debug build
open ~/Library/Developer/Xcode/DerivedData/Whimbrel-*/Build/Products/Debug/Whimbrel.app
```

The app shows both a Dock icon and a menu-bar icon (waveform/circle) — no `LSUIElement` anymore; it was menu-bar-only during the original M1.1 spike, dropped once this became something to actually launch/find day to day. No custom app icon artwork yet, so the Dock icon is currently Xcode's default placeholder.

## Settings window (⌘,)

As of the settings restructuring, configuration lives in a real macOS Settings window (menu-bar icon → **Settings…**, or ⌘, once the app is focused) instead of being stacked into the debug window. The debug window is now just live status + one-shot actions (capture toggle, current/upcoming meetings, "Transcribe Latest Meeting"); everything persisted moved to Settings:

- **Permissions** — the onboarding checklist (was previously also reachable as a standalone "Permissions…" window; that's gone now, this replaces it).
- **Capture** — FR-CAP-4's per-app exclusion list: toggle off any currently-running app to stop its screen text from ever being captured (checked before any Accessibility/OCR extraction happens, not filtered after). Only lists *running* apps, not everything installed — no pre-seeded defaults (password managers, banking domains, etc.) yet, and no per-URL-pattern exclusion yet either, both still open per FR-CAP-4.
- **Meetings** — auto-start toggle + prompt lead-time (was inline in the debug window's Meetings section).
- **Transcription** — which GGML whisper model file to use (was hardcoded to the spike's `models/ggml-tiny.en.bin`; now shows the current path and a "Choose Model File…" picker).
- **Retention** — per-data-class day steppers + "Apply Retention Now" (was inline in the debug window).
- **Diagnostics** — "Print Diagnostics to Console" and "Run Core Self-Test" (dev tooling, moved out of the debug window's footer).

## Permission prompts to expect, and in what order

Click the menu-bar icon → **Settings…** → **Permissions**. Grant in this order (matches the order the checklist lists them):

1. **Screen & System Audio Recording** — click "Grant Access." macOS shows its system dialog; on first grant **you must quit and relaunch the app** (a Screen Recording grant only takes effect on relaunch — this is standard macOS TCC behavior, not app-specific).
2. **Microphone** — click "Grant Access"; a standard mic-permission dialog appears immediately, no relaunch needed.
3. **Accessibility** — click "Grant Access"; macOS opens System Settings → Privacy & Security → Accessibility with Whimbrel already listed — toggle it on. No relaunch needed, but the app polls on next activation so switch back to it after granting.
4. **Calendar** — click "Grant Access"; macOS shows its standard Calendar access dialog (Full Access). No relaunch needed.
5. **Notifications** — click "Grant Access"; macOS shows its standard notification-permission dialog. No relaunch needed.

Each row shows a short explanation of what it unlocks and turns from a hollow to a filled green checkmark once granted. The app is usable with any subset granted — toggling capture on with partial grants starts whatever it can and reports the rest under "Inactive" in the menu, with the reason (FR-ONB-2). Calendar/Notifications aren't required for audio/screen-text capture itself — they only gate the meeting-detection prompt (FR-MTG-1) in the debug window's "Meetings" section.

## Manual test script

Output lands in `~/Library/Application Support/Whimbrel/`:
- `system-audio-<timestamp>.caf`, `mic-audio-<timestamp>.caf` — one pair per capture session.
- `screen-text.jsonl` — one JSON object per line, appended for the life of the app.

1. **Grant all five permissions** via the onboarding checklist above; relaunch after granting Screen Recording.
2. **Meeting audio test:** join or start a Zoom call (or any call). Click the menu-bar icon → toggle **Capture enabled** on. Play some audio through the meeting (e.g. a YouTube tab shared/audible) and speak into your mic. After ~30s, toggle capture off. In Finder, go to the spike folder and play both `.caf` files — confirm the system-audio file has the meeting/call audio and the mic file has your voice, and that they're separate files covering roughly the same time window.
3. **Screen-text test:** with capture still on, switch focus between **Safari** (on a text-heavy page), an **Electron app** (e.g. Slack or VS Code), and **Terminal**. Wait a beat between switches (debounce is ~800ms) and edit/scroll a little in each so a content-change notification fires. Then `tail -f ~/Library/Application\ Support/littlebird/spike/screen-text.jsonl` (or open it in an editor) and confirm one record appears per window/content change, each with `bundle_id`, `window_title`, `text`, and `source`. Safari records should include a `url` and `"source":"ax"`; if the Electron app's AX tree exposes little text, its record should show `"source":"ocr"` instead — that's the fallback working.
4. **Toggle-off test:** toggle **Capture enabled** off from the menu. Note the file sizes/line count of the `.jsonl` and `.caf` files, wait ~15s while switching windows and talking, then check again — nothing should have changed. Toggle back on and confirm new activity resumes (new timestamped `.caf` files, new `.jsonl` lines).
5. **Degraded-audio test (FR-MTG-10), if you can arrange it:** start local recording in Zoom (or another tool known to hold the system-audio tap) first, then toggle capture on in Whimbrel. The menu should show "Inactive → System audio: …" with a reason, and only `mic-audio-*.caf` should be written — no `system-audio-*.caf` for that session, or an empty/near-empty one if the stream started before failing.
6. **Transcription test (FR-MTG-4):** after the meeting audio test above (and with the whisper model fetched — see "Transcription" above), click **"Transcribe Latest Meeting"** in the debug window. It should report `Wrote transcript-<timestamp>.txt to the spike folder.`; open that file and confirm the mic lines are `Me:` and the system-audio lines are `Speaker 1:`/`Speaker 2:`/etc. if diarization models are configured (see Settings → Transcription), or `Others:` if they aren't — roughly matching what you said/played, in chronological order.
7. **Calendar trigger test (FR-MTG-1):** create a calendar event starting a couple of minutes from now with a Zoom/Meet/Teams/Webex link in the location (or add another attendee) — any calendar synced into macOS Calendar (local/iCloud/Exchange) works. It should appear under "Meetings" in the debug window once it's within the 2-hour lookahead window. At the configured lead time (1 minute by default, Settings → Meetings) before it starts, you should get a macOS notification and see an orange "Meeting starting soon" banner with **Start Recording**/**Dismiss** buttons; clicking Start Recording should turn Capture on and show the meeting title in the status line. Toggling capture off afterward should complete the `meeting` row (verifiable indirectly — nothing surfaces this in the UI yet, but there's no separate check to run here beyond confirming capture behaves normally). Turning on "Auto-start recording for calendar meetings" (Settings → Meetings) and repeating the test should skip the prompt and start capture automatically instead.
8. **Capture exclusion test (FR-CAP-4):** with capture on, open Settings → Capture, search for a running app (e.g. Terminal), and toggle it excluded. Switch focus to that app and edit/scroll to trigger a capture — confirm no new line appears in `screen-text.jsonl` for it while excluded. Toggle it back on and confirm capture resumes for that app.

## Hardening (M1.5)

### Retention (FR-MEM-4)

Settings → Retention has a per-data-class retention window — screen/audio capture items and meetings retain independently (meetings default to 90 days, capture items to 30, on the theory that a meeting with a transcript/summary attached is higher-value than a raw screen-text snippet; either can be set to "Keep forever"). "Apply Retention Now" hard-deletes anything older than the window (rows and their recording/transcript files, via `core/api::retention`) and reports how many rows of each class were removed; `RetentionScheduler` runs the same sweep on a timer, and the orphaned-recording scan and delete also run in core.

### Auto-restart on crash (NFR-REL-1, partial)

This is still a single monolithic app (no capture-daemon split — see docs/04-architecture.md), so what's implemented is the whole-app version of "crash → auto-restart": a LaunchAgent using launchd's `KeepAlive.SuccessfulExit=false`, which relaunches Whimbrel after a crash/abnormal exit but *not* after a deliberate Quit (which exits cleanly). Verified against a throwaway test LaunchAgent during development — confirmed it does NOT restart on a clean `exit 0`, and DOES restart (after launchd's own ~10s throttle) on `exit 1`.

```bash
# Build/copy the app to /Applications first (a DerivedData path won't survive a rebuild), then:
apps/macos/scripts/install-launch-agent.sh
# ...or pass a custom path: apps/macos/scripts/install-launch-agent.sh /path/to/Whimbrel.app

# To remove it:
apps/macos/scripts/uninstall-launch-agent.sh
```

### Performance budget (NFR-PRF-1)

`apps/macos/scripts/sample-performance.sh [process-name] [duration-seconds]` samples a running process's CPU% and RSS every 2s and reports the average against the ≤5% CPU / ≤500MB RSS idle-capture budget (excluding any resident local LLM — this only samples the app process itself). Defaults to sampling `Whimbrel` for 300s. Run it while Whimbrel is running with **Capture on** and nothing else in progress (no active transcription/chat/digest call — NFR-PRF-2 explicitly permits CPU bursts during those, so this measures the idle/ambient baseline, not a worst case):

```bash
apps/macos/scripts/sample-performance.sh Whimbrel 300
```

The script itself was verified against dummy processes (a CPU-hogging busy loop correctly reported FAIL; an idle process correctly reported PASS) — the actual app has not been measured yet, since that needs a real capture session running.

## What isn't verified yet

This was built and run on a macOS 26 host with a 14.0 deployment target — it has **not** been run on actual macOS 14 or 15 hardware, and the exact error macOS returns when another process holds the system-audio tap (FR-MTG-10) hasn't been observed empirically, only handled defensively (any system-audio start/stream failure degrades to mic-only). Please report back what you see for both when you test.

Also not yet verified: the actual app's CPU/RSS against the NFR-PRF-1 budget (tooling above is ready, but needs a real capture session to run against), and the LaunchAgent's behavior against a real app crash (verified against a throwaway dummy process, not the real Whimbrel binary).

## Known issue: Accessibility / Screen Recording grants don't survive a rebuild (Debug builds only)

On at least one dev machine (Xcode 26.5), Debug builds are still signed with an **Apple Development** certificate (Automatic signing picks whatever "Apple Development" identity is present locally for local Debug/Run builds, regardless of the paid team configured in `DEVELOPMENT_TEAM`/`project.yml`), which always carries the `com.apple.security.get-task-allow` entitlement. Granting Accessibility or Screen & System Audio Recording to a Debug build appears to work — the native system prompt fires, System Settings shows the toggle on — but `AXIsProcessTrusted()` / `CGPreflightScreenCaptureAccess()` still return `false` from inside the running app, every time.

Root cause, confirmed via `log stream --predicate 'subsystem == "com.apple.TCC"'`: TCC's stored grant is pinned to a literal `cdhash` (the exact hash of one specific build) instead of a stable Team-ID-based designated requirement, because `get-task-allow`-entitled binaries are distrusted enough to lose the normal Team-ID-based grant path. Every Xcode rebuild produces a new binary hash, so the previous grant stops matching immediately. Microphone is unaffected (it doesn't show the same distrust pattern).

**Resolved for distributable builds:** a paid Apple Developer Program membership is now in place (team `GP8PFLM3Q3`), and `scripts/release-publish.sh` produces a real **Developer ID Application**-signed, notarized, stapled build — confirmed via `codesign -dvvv` (`Authority=Developer ID Application: Michael Pointer (GP8PFLM3Q3)`, no `get-task-allow`) and `spctl -a -vv --type execute` (`accepted`, `source=Notarized Developer ID`). Install that build to `/Applications` (not run from DerivedData) and launch it from Finder — TCC grants hold across rebuilds for that path, since a new release build doesn't change the underlying stable Team-ID-based requirement the way every Debug rebuild's fresh `get-task-allow` binary does.

Debug builds (day-to-day Xcode ⌘R iteration) are unaffected by this fix and still lose grants on every rebuild — that's expected and not worth chasing further, since `scripts/release-publish.sh` exists specifically for hands-on dogfooding/distribution builds where this matters.

## Cutting a release

Two scripts, split by what they're allowed to touch:

Note: the live update manifest previously served `0.3.11` from an incident where a version bump was built and published without ever being committed to git (see `docs/superpowers/specs/2026-08-18-release-integrity-design.md`) — that historical `0.3.11` artifact has no corresponding commit or tag, and this doesn't retroactively fix that. `v0.3.12` is the first release cut with `release-prepare.sh`/`release-publish.sh`'s preflight guarantees in place: it is the forward-corrected cutover, not a repair of `0.3.11`'s provenance. See `docs/release-evidence/v0.3.12.md` for the chain-of-custody record.

- `apps/macos/scripts/release-prepare.sh <version>` bumps `VERSION`, `apps/macos/project.yml`'s `MARKETING_VERSION`, and `core/Cargo.toml`'s version, regenerates the Xcode project (`xcodegen generate`), and verifies everything agrees (`scripts/check-version.sh`) — then stops. It never builds, signs, commits, or tags anything.
- `apps/macos/scripts/release-publish.sh [output-zip-path]` (defaults to `~/Desktop/Whimbrel.zip`) builds, signs, and notarizes a Developer ID `Whimbrel.app`, then publishes it as an auto-update. It refuses to run at all unless the working tree is clean, `HEAD` is tagged `vX.Y.Z` matching `VERSION`, and every tracked version source agrees — all three checks run before any build step.

The full operator flow for a new release:

```bash
apps/macos/scripts/release-prepare.sh 0.3.12
# review the diff (git status / git diff)
git add VERSION apps/macos/project.yml apps/macos/Whimbrel.xcodeproj/project.pbxproj core/Cargo.toml core/Cargo.lock
git commit -m "release: prepare 0.3.12"
git tag -a v0.3.12 -m "v0.3.12"
git push && git push --tags
apps/macos/scripts/release-publish.sh
```

`release-publish.sh` requires Xcode signed into the paid "Michael Pointer" Developer Program account (`GP8PFLM3Q3`, Xcode > Settings > Accounts), a `notarytool` keychain profile with app-specific-password credentials for that account (one-time setup: `xcrun notarytool store-credentials "kestrel-notary" --apple-id <apple-id> --team-id GP8PFLM3Q3`), and `gh` (GitHub CLI) authenticated. It exports the archive exactly once and notarizes/staples that exact artifact — an earlier version of this pipeline exported twice (once to submit for notarization, once to produce a local `.app`) on the wrong assumption both passes would be byte-identical; they aren't (each `-exportArchive` re-signs with its own timestamp), which produced a 100%-reproducible "Record not found" from `stapler` against a binary that was never actually submitted. Root-caused 2026-08-06; the current single-export design avoids it structurally.

Once notarized and stapled, `release-publish.sh` computes the zip's SHA-256, creates a GitHub Release for the tag (`gh release create`), uploads the zip to a Cloudflare R2 bucket, and publishes an extended `version.json` manifest (`version`, `url`, `sha256`, `commit`, `tag`, `published_at`, `release_notes_url`). Already-running Whimbrel installs (`UpdateChecker.swift`) poll that manifest on launch and every 24h, verify a newer download's SHA-256 before unzipping it (a mismatch deletes the download and refuses to stage it — never reaches the relaunch path), and show a "Restart Now" banner that hands off to `RelaunchHelper.sh` to swap the app bundle and relaunch.

The manual AirDrop/copy flow still works too: the produced `Whimbrel.zip` is ready to copy to any Apple Silicon Mac (AirDrop, USB, etc.) — the target Mac does **not** need to be signed into this Apple Developer account, and Gatekeeper accepts it fully offline (the notarization ticket is stapled into the bundle, not just checked online against Apple's servers).

The whisper transcription model and the two speaker-diarization models are not bundled into the app (see `core/transcribe/README.md`/`core/diarize/README.md`) — Whimbrel's in-app first-run model downloader (the "Set Up Models" prompt) handles fetching these automatically on first launch, or you can copy them to the target Mac's `~/Library/Application Support/Whimbrel/models/` manually.

This is `arm64`-only (see `project.yml`'s `ARCHS` comment) — the produced build will not run on an Intel Mac.

`.github/workflows/release-integrity.yml` runs on every PR and push, with no signing credentials required — it proves `VERSION` and every generated/tracked version field agree, and that `UpdateChecker`'s manifest-decoding and SHA-256-verification logic behaves correctly against known-good and deliberately-malformed fixtures.
