# 12 — Windows Desktop Client: Design

**Status:** Draft for review, 2026-09-04. Phase 3 of [05-roadmap.md](05-roadmap.md).
**Scope:** How Whimbrel ships on Windows 11 with feature parity to the Phase-1 macOS scope (capture, meetings, memory, chat) plus the Phase-2 surfaces the macOS app already has (routines, connectors, sync, MCP server). This is the design; the per-milestone implementation plans go under `docs/superpowers/plans/` as each milestone starts.
**Reconciled against:** [04-architecture.md](04-architecture.md), [03-non-functional-requirements.md](03-non-functional-requirements.md) NFR-PRT-1…3, [06-risks-open-questions.md](06-risks-open-questions.md) open question 5, `.github/workflows/rust-tests.yml`'s `windows` job, and a file-by-file survey of `apps/macos/Whimbrel/` (20,720 lines of Swift as of `b94986d`).

---

## 0. The honest summary

The roadmap's Phase 3 line reads "port capture layer, reuse Rust core unchanged." The core part is true: every crate in `core/` already builds on `x86_64-pc-windows-msvc` in CI, the only platform-conditional code in the whole workspace is `memory`'s two-line `accelerate` feature gate, and the roadmap's "<5% platform-conditional code" gate is already met. Nothing in this design changes that.

What the roadmap line hides is that **the Rust core is not where all the portable logic lives.** Roughly 6,000 of the macOS app's 20,720 Swift lines are OS-independent orchestration that ended up in the shell: OAuth PKCE flows, connector polling and response decoding, the sync enrollment state machine and both sync coordinators, transcript post-processing (the bleed-through filter and its three tuned thresholds), retention, the routine scheduler's write-action round trip, the model downloader, and the update-manifest logic. NFR-PRT-1 says all of that belongs in core. It doesn't, because each piece was built for one platform under time pressure and Swift was the closest editor.

Every one of those lines has to be written a second time for Windows. The design choice this document makes is *where* that second copy goes: into Rust in `core/` (written once more, then serving Android in Phase 4 for free) rather than into the Windows shell (written a second time now, and a third time for Android). That is the largest single cost in Phase 3 and it lands before any Windows-specific code. §5 sizes it; M3.0 in §9 sequences it.

The second honest point: the macOS architecture doc's platform decision 1 ("Swift native shell, not Electron/Tauri") does not transfer to Windows, and this design does not carry it over. §2 explains why and what the shell is instead.

## 1. Goals and non-goals

**Goals**

1. Phase-1 parity on Windows 11 (23H2 or later): ambient active-window text capture with OCR fallback, calendar- and heuristic-triggered meeting capture, on-device transcription and diarization, memory/search/timeline, grounded chat with streaming, daily digest.
2. The Phase-2 surfaces the macOS app has already shipped: routines and approvals, Google Calendar / Gmail / Outlook 365 connectors, device enrollment and settings + content sync, the local MCP server, auto-update.
3. A mixed Mac + Windows consolidated timeline on one account (the roadmap exit gate), verified live on the user's own two Macs plus one Windows laptop.
4. A capture layer whose measured overhead meets NFR-PRF-1's budget on a mid-range x86 laptop, not just on Apple silicon.
5. Development on the Windows laptop itself with VS Code and PowerShell, no Visual Studio required. Build and package in CI on `windows-latest`.

**Non-goals for Phase 3**

- Windows 10. Process-loopback audio capture, `IsBorderRequired` on window capture, and the toast/AUMID story all differ on 10. 11 only.
- ARM64 Windows (Snapdragon X laptops). `sherpa-onnx-sys` ships no prebuilt ONNX Runtime for `aarch64-pc-windows-msvc` in the version we pin, so `diarize` and therefore the whole app cannot build natively. x64 emulation on ARM64 Windows will run the x64 build, at a transcription-speed penalty. Revisit when the prebuilt exists.
- Agentic desktop automation (FR-AGT). The `ComputerDriver` trait `04-architecture.md` describes does not exist in `core/agent` yet on any platform; nothing Windows-specific to design until it does. §4.9 notes the Windows implementation strategy for when it does.
- Microsoft Store distribution. Sideloaded signed installer only, same posture as the Mac (no App Store).
- Encryption at rest for the SQLite file. The macOS app doesn't have it either (`memory` uses plain bundled SQLite, not SQLCipher); NFR-PRV-2 is an open gap on both platforms and should be closed once, in `core/memory`, not per platform.

## 2. Shell technology: Tauri v2 (Rust shell + React/TypeScript UI)

### The decision

The Windows shell is a Tauri v2 application: a Rust process that owns capture, process lifecycle, tray, notifications, and all calls into `core/`, with the UI rendered in WebView2 from a React + TypeScript + Vite frontend. There is no C# in the project.

### Why the macOS reasoning doesn't transfer

`04-architecture.md` rejected Electron/Tauri for macOS because "capture quality and TCC UX are the product; native APIs are unavoidable." Both halves of that are macOS-specific:

- **There is no TCC on Windows.** The entire staged-permission onboarding (`PermissionsManager`, `PermissionKind`, deep links into System Settings, R3, R10) collapses to one thing: the Privacy → Microphone toggle for desktop apps. UI Automation needs no grant. WASAPI loopback needs no grant. Window capture for OCR needs no grant. There is no per-build hash pinning of grants to fight.
- **The native APIs are as reachable from Rust as from C#.** The `windows` crate (Microsoft's own bindings) exposes UI Automation, WASAPI, Windows.Graphics.Capture, Windows.Media.Ocr, WinEvent hooks, session/power notifications, Credential Manager, and toast notifications with the same surface C# gets. The macOS shell had to be Swift because ScreenCaptureKit, AXObserver, and EventKit are Swift/ObjC-first. Windows has no equivalent constraint.

### Why not C# (WPF or WinUI 3)

C# is the reference implementation for UIA and NAudio makes WASAPI loopback a five-line call. It is genuinely the lower-risk path *for the capture code in isolation*. It loses on everything around it:

- **Two languages instead of one.** Every one of the ~120 functions in `whimbrel_core.h` needs a P/Invoke declaration and a JSON model class. That's the `CoreBridge.swift` tax (2,052 lines) paid again in C#. A Rust shell links `core/` crates directly and pays none of it (§3).
- **Tooling.** WPF and WinUI 3 want Visual Studio for XAML designers, packaging, and debugging. Tauri is first-class in VS Code with `cargo` and `npm`, which is how this repo is already developed.
- **Skills on hand.** Rust is already the repo's second language; React/TypeScript/Vite is the frontend stack the author is actively using. C# is not.
- **WinUI 3 specifically** still has no first-party tray icon, requires the Windows App Runtime redistributable for unpackaged deployment, and has a slower cold start than a tray-resident app should. WPF fixes those but ships a 2006 visual baseline that would need the entire design system re-applied in XAML.

### What the choice costs, stated plainly

- **WebView2 memory.** A WebView2 renderer process is 80–150 MB resident on its own. NFR-PRF-1's 500 MB RSS budget still holds, but with less headroom than the Swift app has. The main window is closed most of the day (tray-resident app), and Tauri can destroy the webview when the last window closes, so the steady-state ambient cost is the Rust process only. This must be measured in M3.1, not assumed.
- **UIA from Rust is more verbose than from C#.** COM interface calls through `windows` are roughly 2× the line count of the `System.Windows.Automation` equivalent. It's a one-time cost in one module (§4.1).
- **Two UI codebases.** SwiftUI and React share nothing but design tokens (§6). This is true under every option; SwiftUI does not run on Windows.

### The escape hatch

M3.1 is a capture spike with no UI at all: a Rust CLI that proves UIA text extraction, OCR fallback, and WASAPI loopback + mic end-to-end into `core/transcribe`. If UIA-from-Rust turns out to be unworkable on the target app list in §4.1, the fallback is a C# capture *process* talking to the Rust shell over a local pipe, not a C# rewrite of the shell. The spike is scoped so that decision is made on measurements before the UI exists.

## 3. How the shell calls the core

### The FFI layer is the wrong seam for a Rust shell

`core/ffi` (8,780 lines) is the C-ABI boundary the Swift app links. A Rust shell calling `whimbrel_llm_answer` through `CString` marshalling inside the same binary would be absurd. But `ffi` is not a thin wrapper: functions like `whimbrel_routines_tick` (seeding built-in routines, due-checking, per-routine spend-cap isolation), `whimbrel_llm_answer` (retrieval, spend-cap check, provider chain, usage logging, envelope encoding), and `whimbrel_llm_prepare_ask` / `finalize_ask` (the streaming split from #74) contain real orchestration that any shell needs. That logic is currently only reachable through `extern "C"`.

### Decision: split `core/ffi` into `core/api` + `core/ffi`

**Landed 2026-09-07** — plan and audit rule in `docs/superpowers/plans/2026-09-07-core-api-split.md`. What exists:

- **`core/api`**: pure-Rust orchestration, `&memory::Store` in, `Result<T, ApiError>` out, every `T` a `serde::Serialize` type with the field names the current JSON uses. Actual modules (no module may be named `llm`/`memory`/`routines`/`sync`/`mcp_host` — those would shadow the crates): `api::ask::{answer, prepare_ask, finalize_ask, discard_ask, generate_conversation_title}`, `api::summary::summarize_meeting`, `api::digest::daily_digest`, `api::spend::spend_today`, `api::routine_runs::tick`, `api::inference::{start, poll, cancel, release}` (the ADR 0002 run registry, moved whole), `api::jobs::{run_episode_summarize_batch, run_thread_summarize_batch}`, `api::sync_ops::{hidden_episode_sync_fields, apply_hidden_episode_sync_fields}`, `api::mcp::{generate_client_token, list_client_tokens}`, plus `api::routing::{RoutingConfig, build_chain}` and `api::envelope::{ApiError, envelope_json}`.
- **`core/ffi`**: parse C strings → call `api` → `envelope_json` → C string for the functions above. Everything that was one `Store`/`sync`/`sync_engine`/`mcp_host` call with no shaping (the `whimbrel_memory_*` get/set/list family, `whimbrel_conversations_*`, the other `whimbrel_routines_*`, all of `sync_ffi.rs` including `whimbrel_sync_content` (`whimbrel_sync_settings` became `whimbrel_sync_settings_iteration` / `_apply_plan` over `api::sync_ops::coordinators` in M3.0 part 2), the `McpHost` functions) stays a pass-through with no `api` twin — the Tauri shell calls those crates directly. `whimbrel_core.h` and every JSON payload are byte-identical to before.

The Tauri shell depends on `core/api`, `core/memory`, `core/mcp-host`, `core/sync-engine`, etc. as ordinary path dependencies. Tauri commands (`#[tauri::command]`) are thin: deserialize the frontend's request, call `api`, return the result, which Tauri serializes with the same serde derives. The React frontend therefore receives byte-for-byte the same JSON the Swift app receives from the C layer. `CoreBridge.swift`'s `Codable` mirror types become TypeScript interfaces generated once from the `api` structs (via `ts-rs` or `specta`), not hand-maintained.

The split was a mechanical refactor of `ffi` with no behavior change, verified by the existing `ffi` tests (untouched), new `api` tests alongside, and the macOS app building unchanged against the same header. It was the first M3.0 PR; §5's table is the rest of M3.0.

### Store ownership

`memory::Store` wraps one SQLite connection and is `!Sync` by design; the Swift app serializes access through one dispatch queue. The Tauri shell does the same with an **actor task**: one `tokio` task owns the `Store`, receives requests over an `mpsc` channel, and replies over oneshots. This is the `ProfileEngine` shape ADR 0001 deferred, done here because a fresh shell has no existing call sites to migrate. ADR 0001's rule holds unchanged: **no network or provider call runs while holding the store.** Streaming inference runs in the registry's own runtime; the actor is only touched at prepare and finalize.

### The Windows crate is not a `core/` workspace member

`apps/windows/src-tauri` is its own Cargo workspace with path dependencies into `../../core/*`, exactly as `apps/macos` links `core/ffi`. It cannot build on macOS or Linux (it links `windows`-crate APIs), so putting it in `core/`'s workspace would break `cargo test --workspace` on the `test` and `android` CI jobs.

### One link-time trap, already documented

`sync-engine`'s test binary can't link on MSVC (LNK2005: rusqlite's bundled `sqlite3` collides with libsql's `core`-feature bundled `sqlite3`). That conflict is **test-only**: the non-test dependency enables only `remote` + `tls`, so libsql never compiles its own SQLite into the production graph. CI's Windows job already links one real executable with rusqlite's bundled SQLite (`whimbrel-mcp-server`, built by `cargo test` on the `mcp-server` package); the first full link of the shell itself happens in the M3.1 spike. The Windows shell crate must never enable libsql's `core` feature. A `cargo deny` or a comment-and-CI-check on `apps/windows/src-tauri/Cargo.toml` keeps it that way.

## 4. Component mapping: macOS shell → Windows shell

Every native service in `apps/macos/Whimbrel/` and what replaces it. Line counts are the current Swift, to size the port.

| Concern | macOS (Swift) | Windows (Rust, `apps/windows/src-tauri/src/…`) | Notes |
|---|---|---|---|
| Active-window text (FR-CAP-1) | `ScreenTextCaptureService` + `AXTextWalker` (227) | `capture/uia.rs`: `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_FOCUS)` for triggers; `IUIAutomation::ElementFromHandle(GetForegroundWindow())` + `TreeWalker` over the control view reading `Name`, `ValuePattern`, `TextPattern.DocumentRange`; UIA `TextChanged` event handler debounced 0.8 s, 5 s idle re-poll | §4.1 |
| OCR fallback (FR-CAP-2) | `OCRFallback` via Vision (47) | `capture/ocr.rs`: `Windows.Graphics.Capture` one frame of the foreground HWND → `Windows.Media.Ocr` → discard frame | §4.2 |
| Capture record | `ScreenTextRecord` → `screen-text.jsonl` → `whimbrel_memory_ingest_jsonl` | Identical JSONL shape (`timestamp, bundle_id, window_title, url, text, source`) → `ingest::ingest_jsonl` directly | `bundle_id` carries the app identity per §4.3 |
| Exclusions (FR-CAP-4) | `CaptureExclusions` keyed by bundle ID, always-exclude `loginwindow`/`ScreenSaver.Engine` (65) | Keyed by the §4.3 identity; always-exclude `LockApp.exe`, `LogonUI.exe`, `CredentialUIBroker.exe`, `consent.exe`; default-exclude `1Password.exe`, `Bitwarden.exe`, `KeePass*.exe`; InPrivate/Incognito detected by window-title suffix and never captured | |
| Lock / sleep (FR-CAP-7) | `PowerIdleGate` (37) | `WTSRegisterSessionNotification` (lock/unlock), `RegisterPowerSettingNotification(GUID_CONSOLE_DISPLAY_STATE)`, `GetLastInputInfo` for idle, `GetSystemPowerStatus` for AC | |
| System audio (FR-MTG-3) | `AudioCaptureService` via ScreenCaptureKit (348) | `capture/audio.rs`: WASAPI **process loopback** (`ActivateAudioInterfaceAsync` + `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`) on the detected meeting app's process tree, falling back to endpoint loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK` on the default render device) | §4.4 |
| Microphone | AVAudioEngine tap | WASAPI shared-mode capture on the default capture endpoint; `IMMNotificationClient` for device changes | |
| Audio conversion | `AudioConversion` via AVAudioConverter → 16 kHz mono WAV (90) | `rubato` resampler → `hound` WAV writer, same 16 kHz mono contract `whimbrel_transcribe_wav` documents | Portable; candidate for `core/transcribe` |
| FR-MTG-10 single-capturer conflict | `degradedReason`, watchdog on 4096-byte files | **Does not exist on Windows.** Loopback is shared-mode; any number of clients can tap. The watchdog stays (a stalled stream is still possible); the "another app holds the tap" message goes | A real Windows advantage |
| Meeting detection (FR-MTG-2) | `MeetingHeuristicDetector` + `AudioDeviceActivity` + browser AX check (269 + 46 + 100) | `capture/meeting_detect.rs`: process list match (`Zoom.exe`, `ms-teams.exe`, `Teams.exe`, `slack.exe`, `CiscoCollabHost.exe`, `Webex.exe`) AND `IAudioSessionManager2` reporting an active render session **and** an active capture session for that process; browser Meet/Teams/Zoom-web via UIA address-bar read on `msedge.exe`/`chrome.exe`/`firefox.exe` | §4.5. Per-process mic-in-use is a first-class Windows signal, better than the Mac's device-in-use check |
| Meeting end | `MeetingEndDetector` (227), issue #84 | Same silence heuristic on the loopback stream; plus the process/session going inactive, which Windows reports directly | #84's calendar-less end detection gets a real signal here |
| Participant-count hints | Zoom/Meet/Teams AX readers (452) | Deferred. UIA readers for Teams/Zoom are feasible; not Phase-3 gating | |
| Calendar (FR-MTG-1) | EventKit (`CalendarService`, 153) + connector polling | **Connector-only.** No usable local-calendar API on Windows for a desktop app (`Windows.ApplicationModel.Appointments` only sees the inbox Calendar app). Outlook 365 and Google Calendar connectors already exist; the polling services move to core (§5) | |
| Alert panel | `MeetingAlertPanelController` floating NSPanel (144) | Tauri window: `always_on_top`, `decorations: false`, `skip_taskbar`, positioned top-right of the primary monitor's work area | |
| Menu-bar / tray | `MenuBarContentView` + `StatusIcon` (114) | Tauri `TrayIcon` with `.ico` state variants (idle / recording-red / degraded-amber), left-click opens a quick-glance popup window, right-click menu | Icons regenerated from `apps/macos/design/` SVGs (§6) |
| Global hotkeys (FR-CAP-5, FR-AGT-3) | none yet on Mac | `tauri-plugin-global-shortcut` | Windows gets this first |
| Notifications | UNUserNotificationCenter | Windows toasts via `windows` crate `Windows.UI.Notifications`; the installer registers the AUMID + Start Menu shortcut toasts require for unpackaged apps | |
| Run at login / crash restart (NFR-REL-1) | LaunchAgent with `KeepAlive.SuccessfulExit=false` | `tauri-plugin-autostart` (HKCU `Run`) for login; crash restart via a tiny supervisor: the tray process re-execs itself through a Scheduled Task with restart-on-failure, or a parent watchdog | **Not a Windows Service.** Session-0 isolation means a service cannot see the user's UIA tree or audio sessions |
| Secrets (NFR-SEC-1) | `KeychainService`, `SyncKeychain`, OAuth helpers' Keychain items | `keyring` crate → Windows Credential Manager (DPAPI, per-user). Same `service`/`account` names as the Mac so settings-sync field keys line up | Issue #33 (service strings not bundle-scoped) applies here too; fix once in the shared naming |
| Data directory | `~/Library/Application Support/Whimbrel/` (moved from the legacy `littlebird/spike/` on first launch, #180) | `%LOCALAPPDATA%\Whimbrel\data\` (`kestrel.sqlite`, `models\`, `recordings\`, `logs\`). **Local**, not Roaming: a 1 GB SQLite file must never ride Windows profile roaming | Moved from `%LOCALAPPDATA%\Kestrel\` on the first launch after 0.9.34 (one rename; stored paths re-rooted) |
| Node connectors | System `node` required, connectors bundled in Resources (`McpNodeLocator`, 133) | **Bundle a pinned Node runtime** in the installer (`resources/node/node.exe`, ~35 MB) alongside the three connectors' `dist/` + `node_modules/` | §4.6 |
| MCP server sidecar | `McpServerProcessManager` spawns `whimbrel-mcp-server` (156), issue #59 orphan reaping | Tauri sidecar; the shell assigns itself to a **Job Object** with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` at startup so the sidecar and every spawned connector die with the shell, structurally | Kills #59's class of bug rather than reaping after the fact |
| Auto-update (NFR-SEC-3) | `UpdateChecker` + `RelaunchHelper.sh` against `version.json` on R2 (232) | `tauri-plugin-updater` with its signed-manifest format, served from the same R2 bucket under `windows/`; installer is NSIS, Authenticode-signed | §7 |
| Sync enrollment + coordinators | `SyncState` (517), `SyncClient` (157), `SettingsSyncCoordinator` (412), `ContentSyncCoordinator` (110) | Moved to `core/api::sync` (§5); the shell provides only the browser-open for Google Sign-In and the Credential Manager read/write | |
| Shell navigation | `WhimbrelShellView` five tabs | Same five tabs (`Today / Meetings / Library / Automations / Settings`) in React; Settings is a tab, not a separate window, matching Windows convention | §6 |

### 4.1 UIA text extraction: what "parity" actually means

Open question 5 in `06-risks-open-questions.md` asks how much OCR fallback Windows will need. The answer depends almost entirely on Chromium, because on the author's machine the apps that matter are Chromium-hosted: Edge, Chrome, Teams (new Teams is WebView2), Slack, VS Code, and every Electron tool. Chromium exposes a UIA provider and turns on renderer accessibility when a UIA client attaches to it. That is the same mechanism as the deferred `AXEnhancedUserInterface` question on macOS, with one difference: on Windows it happens automatically on first query, with a measurable CPU cost inside the Chromium process that stays on for its lifetime. Word, Outlook desktop, Explorer, and Notepad are native UIA and extract cleanly.

M3.1 measures, on the real laptop, for each of: Edge (a Google Doc, a Gmail thread, a Jira page), new Teams (chat and a meeting window), Slack, VS Code, Outlook desktop, Word, Windows Terminal:

- text yield (characters extracted vs. characters visible),
- extraction latency,
- the Chromium process's CPU delta after Whimbrel attaches,
- whether OCR fallback fires.

The spike's deliverable is that table. The design assumes UIA-first, OCR-fallback, same as the Mac; the table is what confirms it.

### 4.2 OCR without persisting a screenshot

`Windows.Graphics.Capture` is the only supported way to grab one window's pixels on Windows 11. It draws a yellow border around the captured window by default; `GraphicsCaptureSession.IsBorderRequired = false` removes it after a one-time `GraphicsCaptureAccess.RequestAccessAsync(Borderless)` consent. The frame lives in a `Direct3D11` surface, is handed to `OcrEngine.RecognizeAsync`, and is dropped. Nothing is written to disk. FR-CAP-3's "screenshots are not persisted" holds by construction; FR-CAP-9's opt-in screenshot retention is out of scope for Phase 3 on both platforms.

**The one exception: meeting snapshots (FR-MTG-11).** Ambient capture still never writes a frame. Meeting snapshots do, and only under all three of these conditions:
- a meeting is recording;
- the frame is of the meeting's own window (never the whole screen, and never an excluded, private or hidden window);
- Settings → Meetings → "Save slides and shared screens" is on (the default; local to the PC, not synced).

They are PNGs of slides and shared screens in `Pictures\Whimbrel Snapshots\<date time> <title>\`. Their text is local-only and never synced. Retention deletes the folder with the meeting unless "Keep snapshots" is on. Plan: `docs/superpowers/plans/2026-10-28-windows-meeting-snapshots.md`.

### 4.3 App identity: what goes in `bundle_id` on Windows

Windows has no bundle ID. The field is populated with, in order of preference:

1. The **AppUserModelID** when the window has one (packaged apps, new Teams, Store apps): e.g. `MSTeams_8wekyb3d8bbwe!MSTeams`.
2. Otherwise the **lowercased executable basename** of the window's process: `msedge.exe`, `code.exe`, `slack.exe`.

The exclusion UI and the timeline's per-app filters work on this string. Cross-device timeline views (FR-SYNC-3) display a friendly name mapped from either form; the raw value stays as captured so per-device attribution stays honest. `capture_item.app_bundle_id` keeps its column name; no schema change.

### 4.4 Audio: process loopback first

Endpoint loopback captures everything the machine plays, including the Spotify session the user forgot about. Windows 10 20H1+ and all of Windows 11 support **process loopback**: capture only the audio of a given process tree. For a detected or calendar-triggered meeting whose app is known, Whimbrel taps that process. When the app is unknown (a manual "record now" with nothing detected), it falls back to endpoint loopback with a visible note.

Bluetooth headsets are the known Windows failure mode: when the mic opens, many headsets drop from A2DP to HFP and the *system audio* leg degrades to narrowband. This is not fixable by Whimbrel; it is detected (`IMMNotificationClient` reports the format change) and surfaced as a degraded-quality warning, the same slot the Mac uses for its built-in-speakers warning.

Both streams are written as separate timestamped WAVs (`system-audio-<ts>.wav`, `mic-audio-<ts>.wav`) into `recordings\`, then resampled to 16 kHz mono for `core/transcribe` and `core/diarize`. The transcript merge, speaker labeling, and bleed-through filter run in core (§5), unchanged.

### 4.5 Meeting detection gets a better signal than the Mac has

`IAudioSessionManager2` enumerates per-process audio sessions on both the render and the capture endpoint, with `AudioSessionStateActive` per session. "Zoom is playing audio AND Zoom has the mic open" is a direct query, no heuristics over device-in-use flags. The 30-second detection budget in FR-MTG-2 is comfortably met with a 5 s poll. Issue #84's "no end signal for non-calendar meetings" also improves: the meeting app's capture session going inactive is a hard end signal, layered under the existing silence heuristic.

### 4.6 Node: bundle it

The Mac deliberately does not bundle Node ("disproportionate weight for a signed/notarized macOS app"). That reasoning was about notarization of a foreign binary under Hardened Runtime. Windows has neither constraint: `node.exe` is already Authenticode-signed by the OpenJS Foundation, Node's license permits redistribution, and the NSIS installer just copies it. A Windows user is far less likely than a Homebrew-using Mac developer to have Node on `PATH`, and "install Node.js first" is not an acceptable onboarding step. Bundle a pinned LTS `node.exe` (~35 MB); `McpNodeLocator`'s search logic reduces to one path. The three connectors' `dist/` + `node_modules/` ship as installer resources exactly as they ship in the Mac bundle.

### 4.7 The MCP server sidecar and process hygiene

`whimbrel-mcp-server` is already a Windows-buildable Rust binary. It ships as a Tauri sidecar. Port 4319 and the bearer-token auth are unchanged. The Job Object noted in the table means a crashed shell takes its children with it; the orphan-reaping logic from #59 becomes a belt-and-braces check, not the primary mechanism.

### 4.8 Model files

Whisper GGML (`ggml-tiny.en.bin` by default), the two sherpa-onnx diarization models, and the MiniLM embedding files live under `%LOCALAPPDATA%\Whimbrel\data\models\`. The downloader (`ModelDownloadService`, 156 Swift lines including the tar extraction) moves to core (§5) so both platforms share the URL list, the size floors, and the archive handling. Whisper runs CPU-only in Phase 3 (whisper.cpp's Vulkan backend is an optional later feature flag; CUDA is out of scope). `memory`'s candle embedder runs on CPU without `accelerate`; MiniLM at that size is fine on any x86 laptop from the last five years.

### 4.9 Agent driver (deferred, noted)

When `core/agent` grows its `ComputerDriver` trait, the Windows implementation is UIA's `InvokePattern` / `ValuePattern` / `SelectionPattern` for semantic actions and `SendInput` for the coordinate fallback, both from the same `windows` crate. Nothing here needs deciding in Phase 3.

## 5. M3.0: move portable Swift logic into core first

This is the prerequisite milestone. The table lists the Swift that is OS-independent in substance, where it goes, and what the Mac keeps. The Mac app is switched to the core version of each item **as it moves**, so the Mac remains the reference implementation and the moved code gets real Rust tests before Windows ever calls it. Swift keeps only the platform-bound calls (Keychain reads, opening a browser, a loopback socket listener, UI).

| Swift before (lines) | Landed in (M3.0 part 2, PRs #98–#103 and following) | Mac keeps (lines after) |
|---|---|---|
| `OAuthLoopbackFlow` (299), `GoogleOAuthHelper` (474), `MicrosoftOAuthHelper` (302) | `core/api::oauth`: PKCE, state, loopback listener (`std::net` on 127.0.0.1), code exchange, refresh, scope/JWT helpers; `whimbrel_oauth_begin/wait/cancel/refresh/scope_satisfies/jwt_claims` | Keychain read/write of the stored-token blob, `ASWebAuthenticationSession`, forwarding a closed window to `cancel` (100 + 238 + 189) |
| `SyncState` (517), `SyncClient` (157), `SyncConfig` (38) | `core/api::sync_ops::enrollment`: Worker client, status machine (`next_status`), group-key decision, permanent-failure and user-message tables; `whimbrel_sync_worker_*`, `whimbrel_sync_enrollment_step`, `whimbrel_sync_group_key_decision` | `SyncClient` is a shim (82). `SyncState` (507) still drives its own status transitions from its Combine pipeline and Keychain reads — `whimbrel_sync_enrollment_step` exists for the Rust shell; switching the Mac's `SyncState` onto it is a UI refactor left for later. |
| `SettingsSyncCoordinator` (412), `ContentSyncCoordinator` (110), `SettingsFieldTimestamps` (42) | `core/api::sync_ops::coordinators`: push planning (API-key tombstone rule), the settings round trip, the apply gate; `whimbrel_sync_settings_iteration`, `whimbrel_sync_settings_apply_plan` (replacing `whimbrel_sync_settings`). No `SettingsSource` trait: the shell passes a JSON snapshot in and applies a JSON plan out. | The 60 s timers, `.ready` gating, and the field-key ↔ UserDefaults/Keychain mapping (337 + 110 + 42). Content sync stays on `whimbrel_sync_content` (one core call; the prepare/exchange/apply split changes lease and cursor semantics and was not proven equivalent). |
| `GoogleCalendarPollingService` (201), `GoogleCalendarMeetingDecoding` (170), `GmailPollingService` (181), `GmailSyncDecoding` (57), `CalendarEventItemMapping` (26) | `core/api::connectors::{calendar,gmail}`: request/apply pairs around the shell's MCP call, decoding, link detection, merge, cursor handling; `whimbrel_connector_*`, `whimbrel_calendar_*`, `whimbrel_meeting_link_detect` | The polling timers and the MCP HTTP call (169 + 142); the three decoding files are gone. |
| `CalendarConnectorState` (251), `GmailConnectorState` (297), `Outlook365ConnectorState` (207), `ConnectedAccountStore` (67) | `core/api::connectors::accounts` over a new `connector_account` table; `whimbrel_connector_accounts_*`. Device-local, outside settings sync. | Settings UI and Keychain tokens; `ConnectedAccountStore` (128) is a main-actor cache with a one-release UserDefaults migration shim; the three states (255 + 301 + 211) are the connect/disconnect UI flows. |
| `TranscriptPostProcessing` (196), `TranscriptFormatting` (77), `TranscriptionService` merge logic | `core/transcribe::postprocess`; `whimbrel_transcript_postprocess`, `whimbrel_transcript_parse_turns` | Audio capture and conversion call sites |
| `RetentionSettings` (115), `RetentionScheduler` (74), `RecordingFileOrphanScanner` (63), `MeetingFileCleanup` (50) | `core/api::retention` (sweep, sibling-file deletion, orphan scan); `whimbrel_retention_*` (replacing `whimbrel_memory_apply_retention` and `whimbrel_memory_all_referenced_audio_paths`) | Settings UI, the schedule timer (111 + 73) |
| `RoutineScheduler` write-action round trip | `core/api::write_action` plan/settle; `whimbrel_routines_plan_write_action`, `whimbrel_routines_settle_write_action` | The 60 s timer, the notification, and one generic MCP tool call (327) |
| `ModelDownloadService` (156), `EmbeddingModelLocator` (22) | `core/api::models` (download, size floors, `.tar.bz2` unpack via `tar`+`bzip2` crates); start/poll/cancel/release registry | Progress UI as a 200 ms polling shim (66); `EmbeddingModelLocator` (22) stays — it reads `Bundle.main` |
| `HostedModelsService` (99), `PromptLibrary` (~60), `QueryUrgencyClassifier` (20) | `core/api::hosted_models`, `core/api::prompts` (no `core/api::llm` module, by design) | `HostedModelsService` shim (38); `PromptLibrarySettings` (124) and `DailyDigestCache` (34) stay — UserDefaults-bound |
| `UpdateChecker` manifest decode + SHA-256 verify | `core/api::update`; `whimbrel_update_check_manifest`, `whimbrel_update_verify_sha256` | Timer, download staging, `ditto`, `spctl`, `RelaunchHelper.sh` (191) |
| `MeetingWorkspaceState` (96) | `core/api::meeting_workspace` (`derive_state`, `load_state`); `whimbrel_meeting_workspace_state` (pure form) | The `Decodable` mirror of the tagged state (132) |
| `InferenceStreamClient` (160), `StreamingAskViewModel` (124) | Already mirror `core/llm::streaming`; Windows uses the Rust types directly | Unchanged |

**Outcome (2026-09-07):** `apps/macos/Whimbrel` went from 20,720 to 19,833 Swift lines. The plan's ≤ 15,500 exit-gate figure assumed the Swift files would shrink to call sites; in practice each moved capability leaves behind a `Codable` mirror of its JSON shape plus the platform-bound half (Keychain, UserDefaults, timers, `ASWebAuthenticationSession`, MCP HTTP calls), and three items (Task 6 workspace state, Task 9 accounts, Task 11 enrollment) grew Swift slightly. The rule that mattered — every moved capability is callable from a Rust shell with no Swift in the loop and carries Rust tests at least as strong as the Swift tests it replaced — holds for every row above. The header is a superset of `main`'s at the start of the plan except for the three C functions whose only Swift callers moved in the same PR (`whimbrel_memory_apply_retention`, `whimbrel_memory_all_referenced_audio_paths`, `whimbrel_sync_settings`).

The original estimate was that roughly 5,500–6,500 lines of Swift would become the same number of Rust lines with tests; the Rust side came in around that figure, the Swift side shrank less than expected (see outcome above). This was the bulk of Phase 3's calendar time and it produced no visible Windows feature. It is also the work that makes Phase 4 (Android via JNI onto the same C ABI) cheap instead of a third rewrite. If it is skipped, the Windows shell reimplements all of it in Rust anyway, just in a place Android can't reach.

## 6. UI

- **Same information architecture.** Five tabs, same names, same contents as `docs/10-meeting-workspace-v0.4.0.md`. The Before/During/After meeting workspace state derivation is pure logic and moves to `core/api` in M3.0 so both UIs derive identical states from identical inputs.
- **Design tokens ported, not redesigned.** `DesignSystem.swift`'s 8 adaptive colors, serif display / sans body pairing, 4 pt spacing scale, card and pill-button styles become CSS custom properties in `apps/windows/ui/src/tokens.css`. Segoe UI for body; a bundled serif (Source Serif or Georgia, in-box) for display. The dark rust contrast issue noted under FR-UI-3 is fixed in the token file on both platforms at the same time.
- **Windows conventions where they differ from Mac:** Settings is a tab, not a ⌘, window. Tray left-click opens the quick-glance popup (the Mac's menu-bar popover); right-click is the menu. Toasts carry the meeting-detected prompt's Accept/Dismiss as toast buttons. Light/dark follows the system via `prefers-color-scheme`; Mica/acrylic is not used (WebView2 can't composite it cleanly and it's cosmetic).
- **Accessibility (NFR-ACC-1):** the React tree is plain semantic HTML with ARIA where needed, tested with Narrator and NVDA. The irony NFR-ACC-1 warns about applies doubly to an app that is itself a UIA client.
- **Icon:** `apps/macos/design/app-icon.svg` rendered to a multi-resolution `.ico` (16/24/32/48/256) for the installer and taskbar; `menubar-glyph.svg` rendered as **colored** 16/20/24/32 tray `.ico` variants with the red/amber badge baked in per state (Windows tray icons are not template-tinted).

## 7. Build, packaging, release

- **Layout:**

  ```
  apps/windows/
    README.md
    src-tauri/            Rust shell (own Cargo workspace; path deps into ../../core)
      src/capture/        uia.rs, ocr.rs, audio.rs, meeting_detect.rs, session.rs
      src/commands/       tauri::command wrappers over core/api
      src/store_actor.rs  the single-owner Store task
      src/tray.rs, notifications.rs, secrets.rs, jobs.rs
      tauri.conf.json
    ui/                   React + TypeScript + Vite
      src/tokens.css      ported design system
      src/generated/      TS types emitted from core/api structs
    design/               .ico renders and the script that produces them
  ```

- **Dev loop:** `npm run tauri dev` from `apps/windows` on the Windows laptop. Prerequisites: Rust (MSVC toolchain), Node LTS, CMake (for whisper.cpp, same as the Mac), WebView2 (in-box on Win11). No Visual Studio; the Build Tools workload is enough for `link.exe`.
- **CI:** a new `windows-app` job in `rust-tests.yml` on `windows-latest`: builds `apps/windows/src-tauri` in release, runs its unit tests (capture modules have pure-function tests over fixture UIA trees and WAV files; live capture is not testable on a runner), builds the frontend, and produces an unsigned installer as an artifact. Required status check once green, same as the existing `windows` job.
- **Version parity:** `VERSION` already drives `core/Cargo.toml` and `project.yml`; `tauri.conf.json`'s `version` and `apps/windows/ui/package.json` join `scripts/check-version.sh`'s agreement check so one release number covers both platforms.
- **Signing:** an Authenticode code-signing certificate (an OV cert on a hardware token or Azure Trusted Signing). Unsigned installers trip SmartScreen hard enough that "personal/small-group use" still needs a signature. This is a purchase decision, called out in §8.
- **Updates:** `tauri-plugin-updater` reads a signed `latest.json` (Tauri's own minisign-based format, separate from the Mac's `version.json`) from the same R2 bucket under a `windows/` prefix. `release-publish.sh` grows a Windows leg, or a sibling `release-publish.ps1` runs on the Windows machine; either way it refuses to run under the same preconditions the Mac script enforces (clean tree, `vX.Y.Z` tag matching `VERSION`).

## 8. Risks and open questions specific to this design

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| W1 | **Chromium UIA yield or CPU cost is worse than expected.** Chromium enables renderer accessibility when a UIA client attaches; the cost stays on for the process lifetime. If Edge/Teams/Slack extraction is poor or expensive, OCR fallback fires far more than on the Mac. | Medium | High | M3.1 measures before any UI exists (§4.1). Fallback order: throttle UIA polling per-app → OCR for Chromium apps only → the C# capture-process escape hatch in §2. |
| W2 | **WebView2 RSS eats into NFR-PRF-1.** | Medium | Medium | Destroy the webview when the last window closes; measure ambient RSS in M3.1 with the tray-only process. |
| W3 | **M3.0's core consolidation regresses the Mac.** Moving ~6k lines of shipped behavior is the biggest churn the Mac app has seen. | High | High | Move one item at a time; the Mac switches to each core version in the same PR with the existing Swift tests still passing; no Windows PR depends on an item that hasn't landed on the Mac first. |
| W4 | **Bluetooth HFP narrowband system audio** during meetings on headsets. | High on BT headsets | Medium | Detect and warn (§4.4). Not fixable in-app; documented as the Windows equivalent of the Mac's built-in-speakers warning. |
| W5 | **Code-signing certificate cost and process.** | Certain | Medium | Budget for an OV certificate or Azure Trusted Signing before M3.5; unsigned builds are fine for M3.1–M3.4 dogfooding on the author's own machine. |
| W6 | **ARM64 Windows** has no `diarize` build. | Certain today | Low for the current audience | Out of scope (§1); x64 emulation runs the app. Re-check `sherpa-onnx-sys` prebuilt targets at each bump. |
| W7 | **Two UIs drift.** SwiftUI and React implementations of the same screens diverge in behavior. | High over time | Medium | All state derivation and data shaping lives in `core/api` after M3.0; the UIs are rendering only. TypeScript types are generated from the Rust structs, so a shape change breaks the build, not the runtime. |

**Open questions**

1. **Process loopback on new Teams.** New Teams runs as a packaged WebView2 app whose audio may come from a child or a broker process. Whether process-tree loopback captures it, or whether endpoint loopback is needed for Teams specifically, is a spike question.
2. **Where the store actor's request/response types live.** `core/api` (so Android's JNI layer can reuse the message shapes) or the Windows shell (simpler now). Leaning `core/api`; decide in M3.0 when the first coordinator moves.
3. **Whether the Mac adopts `keyring`-style naming** for Keychain items as part of fixing #33, so the settings-sync field names match across platforms without a mapping table.
4. **Crash-restart mechanism.** Scheduled Task with restart-on-failure vs. a 200-line supervisor parent. Pick in M3.2 after seeing how Tauri's own process behaves under `panic = "abort"`.

## 9. Milestones and exit gates

| Milestone | Scope | Exit gate |
|---|---|---|
| **M3.0 Core consolidation** | §3's `core/api` split; §5's table, one row per PR, Mac switched over in the same PR | Mac app builds and behaves identically (Swift test suite green, one dogfooding week); `core/api` tests cover every moved item; `apps/macos` Swift line count drops by ≥5,000 |
| **M3.1 Capture spike** (Rust CLI, no UI) | UIA extraction table from §4.1; OCR fallback; process loopback + mic → WAV → `core/transcribe` + `core/diarize` produce a labeled transcript of a real Teams and a real Zoom call; ambient CPU/RSS measured | The §4.1 table filled in on the real laptop; NFR-PRF-1 numbers recorded; go/no-go on UIA-from-Rust |
| **M3.2 Shell skeleton** | Tauri app, tray, autostart, store actor, Library + Today + Chat (streaming), Settings with providers/keys, sync enrollment against the live Worker, content sync | The roadmap's headline gate: a Windows laptop and a Mac on one account show one interleaved timeline with per-device attribution |
| **M3.3 Meetings** | Detection, alert panel, recording, transcription, diarization, meeting workspace (Before/During/After), calendar connectors, retention | A calendar-triggered and a heuristically detected meeting each produce a summary within NFR-PRF-5's 2 minutes; discard works |
| **M3.4 Automations** | Routines, approvals inbox, Gmail/Outlook write actions, MCP server sidecar, bundled Node | Daily Brief runs on schedule; an EOD recap email is approved and sent from Windows; `whimbrel-mcp-server` reachable from Claude Desktop on the same machine |
| **M3.5 Packaging** | Signed NSIS installer, updater, CI `windows-app` job, version-parity check, release script leg, release evidence doc | A signed installer updates itself in place; `docs/release-evidence/` has a Windows entry for the release that ships it |

Phase 3 closes when M3.5's gate is met **and** the roadmap's existing exit gate holds: Phase-1 parity on Windows 11, the mixed timeline, and `core/` under 5% platform-conditional code (which it already is; the gate now additionally means "no portable logic was added to `apps/windows` that should have gone to `core/api`", checked at review).
