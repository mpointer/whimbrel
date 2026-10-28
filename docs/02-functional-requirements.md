# 02 — Functional Requirements

Requirements are numbered `FR-<SUBSYSTEM>-<n>`, marked **[P1]**–**[P4]** for roadmap phase (see [05-roadmap.md](05-roadmap.md)), and written to be individually testable.

Priority: **MUST** (MVP-blocking for its phase), **SHOULD** (target for phase, may slip one), **MAY** (opportunistic).

---

## FR-CAP — Screen context capture

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-CAP-1 | P1 | MUST | Capture the text content of the active window via the macOS Accessibility API, sampling on focus change and on content-change notifications (debounced). |
| FR-CAP-2 | P1 | MUST | For apps that don't expose text via Accessibility (canvas-rendered Electron apps, images, remote desktops, video), fall back to Vision-framework OCR of the active window only. |
| FR-CAP-3 | P1 | MUST | Persist captured text with metadata: app bundle ID, window title, browser URL (when available), and timestamp. Screenshots are **not** persisted by default. |
| FR-CAP-4 | P1 | MUST | Support per-app and per-URL-pattern exclusion lists, pre-seeded with defaults: password managers, private-browsing windows, banking/health domains, system credential dialogs. |
| FR-CAP-5 | P1 | MUST | Provide a global capture pause/resume via menu-bar toggle and configurable hotkey; capture state is always visible in the menu bar. |
| FR-CAP-6 | P1 | MUST | Deduplicate near-identical successive captures (e.g., simhash over normalized text) before persistence. |
| FR-CAP-7 | P1 | SHOULD | Detect screen lock / screensaver / display sleep and suspend capture. |
| FR-CAP-8 | P2 | SHOULD | Incognito interval: "pause for 15/60 min / until tomorrow" one-click options. |
| FR-CAP-9 | P2 | MAY | Optional periodic low-res screenshot capture as an opt-in (off by default), stored encrypted with its own retention policy. |

## FR-MTG — Meeting capture & transcription

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-MTG-1 | P1 | MUST | **Calendar trigger:** read calendar events via EventKit (local/iCloud/Exchange/Google accounts configured in macOS), plus Google Calendar and Outlook 365 via MCP connectors. For events containing a conferencing link or ≥1 other attendee, prompt N minutes (configurable, default 1) before start to begin capture; optional per-calendar auto-start. |
| FR-MTG-2 | P1 | MUST | **Audio/app trigger:** detect an active meeting with no matching calendar entry via heuristics — meeting app running (Zoom, Teams, Meet in browser, Slack huddle, Webex, FaceTime) AND sustained bidirectional audio (mic in use + system audio out) — and surface a one-click "Record this meeting?" prompt within 30 s of meeting start. |
| FR-MTG-3 | P1 | MUST | Capture **system audio + microphone** locally via ScreenCaptureKit (or Core Audio process taps on macOS 14.4+). No bot ever joins a call; no other participant is required to install anything. |
| FR-MTG-4 | P1 | MUST | Transcribe on-device (whisper.cpp or Apple SpeechAnalyzer) supporting ≥10 languages, with channel-based diarization at minimum (me = mic channel, others = system channel). |
| FR-MTG-5 | P1 | MUST | Provide a lightweight in-meeting notes pad; on meeting end, blend the user's rough notes with the transcript (Granola-style) into a structured summary: TL;DR, decisions, action items (owner + due date when stated), open questions. |
| FR-MTG-6 | P1 | MUST | Consent affordances: persistent visible recording indicator; one-click discard (audio, transcript, and summary) during or after any meeting; configurable auto-announcement template for two-party-consent jurisdictions. |
| FR-MTG-7 | P1 | SHOULD | Speaker-name attribution by cross-referencing calendar attendees and platform participant names when available. |
| FR-MTG-8 | P2 | SHOULD | Optional screen recording of the shared-content window during meetings (off by default), linked to the transcript timeline. |
| FR-MTG-9 | P2 | SHOULD | Post-meeting follow-up drafts (recap email, task creation) routed through Routines/MCP with user approval. |
| FR-MTG-10 | P1 | MUST | Detect and gracefully handle the macOS single-system-audio-capturer constraint: if another process holds the audio tap, degrade to mic-only capture and warn the user. |
| FR-MTG-11 | P2 | SHOULD | **Meeting snapshots (Granola-style):** during a recorded meeting, take a still image of the shared screen or meeting window every N seconds and keep only frames that differ meaningfully from the last kept one, so each slide or shared visual is stored once. Snapshots attach to the meeting at their timestamp, show in meeting detail alongside the transcript, have their text OCR'd into search, and can feed the summary. On by default only for meetings already being recorded; follows capture exclusions, the discard action and meeting retention. Lighter than FR-MTG-8 (stills, not video). |

## FR-MEM — Memory, timeline & search

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-MEM-1 | P1 | MUST | Maintain a unified local timeline of screen-context items, meetings (transcript + summary), and (later) agent/routine activity, browsable by day. |
| FR-MEM-2 | P1 | MUST | Chunk and embed all captured text locally; provide hybrid semantic + keyword search with filters for time range, app, meeting, and (P2+) device. |
| FR-MEM-3 | P1 | MUST | Generate an automatic end-of-day digest: what was worked on, meetings held, decisions, open loops. |
| FR-MEM-4 | P1 | MUST | Retention policies configurable per data class (default: raw screen text 90 days, transcripts 1 year, summaries/digests indefinite) and hard-delete by time range, app, or individual item — deletion propagates to search indexes and synced replicas. |
| FR-MEM-5 | P2 | SHOULD | Entity extraction (people, projects, companies) to power filtered views ("everything about Project X"). |
| FR-MEM-6 | P2 | MAY | Weekly digest and per-project rollups. |

## FR-CHAT — Context-grounded assistant

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-CHAT-1 | P1 | MUST | Chat interface whose answers are grounded in memory via retrieval, with citations linking each claim to timeline items (screen capture, transcript segment). |
| FR-CHAT-2 | P1 | MUST | Model-agnostic backend: hosted APIs (Anthropic, OpenAI) and local models via Ollama, selectable globally and per conversation. |
| FR-CHAT-3 | P1 | SHOULD | "Current context" awareness: the assistant can be asked about the currently active window/meeting without the user pasting anything. |
| FR-CHAT-4 | P2 | SHOULD | Drafting actions (email reply, doc summary, follow-up) that hand off to MCP tools with approval. |
| FR-CHAT-5 | P2 | SHOULD | Sensitivity-based routing policy: user can require that memory-derived prompts only go to designated models (e.g., local-only mode). |

**2026-07-26: LLM governance core landed (feature-completeness sub-project 1 of 4).**
User's [`llm-governance-gateway`](https://github.com/mpointer/llm-governance-gateway)
(their own TypeScript library) architectural question — port natively into Rust vs. run
the real npm package behind a Node sidecar — was resolved in favor of a native port: "a
Node runtime inside a signed/notarized macOS app is disproportionate weight for what's
mostly the library's *decisions*, not irreplaceable code." A deliberately-scoped subset
("the load-bearing quarter" — usage ledger, spend caps, provider-chain failover,
retry/backoff classification) is now real, native Rust in `core/memory`/`core/llm`, with
test cases extracted directly from the TypeScript library's own vitest suite as shared
JSON fixtures rather than reinvented. `RoutingPolicy` no longer always resolves to
Ollama — `core/ffi`'s two LLM call sites now check a spend cap, walk a real
Ollama→Anthropic→OpenAI chain (failing over on retryable errors), and log real
token-usage-derived cost to a `usage_log` table. Default out-of-the-box behavior (no env
vars set) is unchanged — Ollama-only, byte-identical to before this landed.

Explicitly deferred from the source library (per the user's own steer): the model-graded
judge, batch processing, the prompt-store override layer, and ZDR (zero-data-retention)
routing constraints — these wait until Whimbrel's own usage actually demands them. A
future account-level/cross-device spend cap could run the actual npm package at the
`server/` sync relay (Node-friendly, zero packaging cost) whenever that phase arrives —
noted, not built.

**2026-07-27: Settings UI landed (sub-project 2 of 4) — FR-CHAT-2/FR-CHAT-5 are now
done.** The *backend capability* (real Anthropic/OpenAI clients, real chain failover,
real sensitivity-based local-only forcing via `RoutingPolicy.choose(sensitive)`) already
existed and worked — verified with real API calls, both Ollama-only and hosted-provider
paths. What was missing is now built: (a) a Keychain-backed `KeychainService` stores
Anthropic/OpenAI API keys (never in `UserDefaults` or a plaintext file), and
`SettingsView.swift`'s `AIProvidersSettingsPane` gives the user a real pane to set those
keys, the default provider/model, per-identity and global spend caps, and local-only
mode; (b) `TimelineView.swift` now has a real per-conversation provider+model picker, and
`RoutingSettings.localOnlyMode` reaches `sensitive: true` on every `askIfNeeded`/
`generateDigestIfNeeded` call, so FR-CHAT-5's local-only mode is user-reachable, not just
capability-only. The `ANTHROPIC_API_KEY`/`OPENAI_API_KEY`/`WHIMBREL_ALLOW_HOSTED_LLM=1`
env-var stopgap (added as a safety gate before Settings UI existed) has been fully
retired, not merely supplemented — Keychain-backed keys and the Settings pane are now the
only way to enable hosted providers.

Two known follow-ups, not gating: (a) a stopped/not-running Ollama does **not** trigger
failover to a configured hosted provider — only an HTTP 429/5xx from a *running* Ollama
does, since the ported retry classification (faithfully matching the source library's
own behavior) only treats HTTP-status errors as retryable. Explicitly decided to keep
this as-is: falling back to a paid, non-local API just because the local model isn't
running is a real privacy/cost decision that deserves to stay a deliberate choice, not an
automatic one. (b) Cap-exceeded and provider-failure are still indistinguishable at the
FFI boundary (both return `null`) — Settings UI (sub-project 2, now landed) did not add
that richer signal, so a legitimately-exceeded spend cap still fails silently rather than
telling a user "you hit your spend cap" apart from "the provider is down." Carried
forward as a known gap, not gating.

Sequencing note: sub-project 2 (Settings UI for keys/routing/caps) is done; sub-project 3
(prompt library + task-based routing) is next, then 4 (Littlebird-style suggested-prompt chips,
which needs 3's prompt library to have something real to suggest from).

## FR-AGT — Agentic desktop automation

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-AGT-1 | P2 | MUST | Computer-use driver that prefers semantic Accessibility-API actions (press button by role/label, set field value) and falls back to coordinate-based click/type with screenshot grounding. |
| FR-AGT-2 | P2 | MUST | Layered guardrails (Hermes model): (a) approval gate before the agent types into any application; (b) approval gate before sensitive/destructive actions — send, delete, submit, purchase, credential entry; (c) per-skill permission manifest declaring the apps and tools it may touch, enforced at runtime. |
| FR-AGT-3 | P2 | MUST | Global kill switch: one hotkey immediately halts all agent activity and releases input control. |
| FR-AGT-4 | P2 | MUST | Complete audit log of every agent action: timestamp, skill, action, target, approval decision, before/after evidence. |
| FR-AGT-5 | P2 | MUST | Portable skill format (OpenClaw tradition): a skill is a directory with a manifest (name, description, triggers, permission requirements) plus markdown instructions; installable, editable, and shareable as files. |
| FR-AGT-6 | P2 | SHOULD | Skill library UI: browse, enable/disable, inspect permissions, view run history per skill. |
| FR-AGT-7 | P3 | MAY | Self-improvement: agent may propose new skills from observed repetitive workflows; proposals require explicit user adoption. |

## FR-RTN — Routines (scheduled proactive automation)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-RTN-1 | P2 | MUST | Cron-style scheduler with timezone handling plus an OpenClaw-style heartbeat that wakes the routine engine at a configurable interval for condition-based routines ("when X happens"). |
| FR-RTN-2 | P2 | MUST | A routine composes: memory queries + MCP tool calls + LLM synthesis + delivery. Example: 8:30 daily brief = yesterday's timeline digest + today's calendar (Outlook/Google via MCP) + flagged unread email (Gmail/Outlook via MCP) → synthesized brief. |
| FR-RTN-3 | P2 | MUST | Delivery channels: desktop notification, in-app inbox, mobile push (P4), email. Routine outputs are stored in the timeline. |
| FR-RTN-4 | P2 | MUST | Built-in templates: daily brief, end-of-day recap, weekly digest, pre-meeting prep (auto-runs N minutes before meetings with context on attendees and prior interactions). |
| FR-RTN-5 | P2 | SHOULD | Natural-language routine authoring ("every Friday at 4, summarize what I shipped this week and email it to me") compiled into an editable routine definition. |
| FR-RTN-6 | P2 | MUST | Routines that perform write actions (send email, create tasks) are subject to FR-AGT approval gates unless the user grants standing per-routine authorization. |
| FR-RTN-7 | P2 | SHOULD | Failure handling: retries with backoff, and a visible run history with per-run status and output. |

## FR-MCP — MCP tool host

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-MCP-1 | P2 | MUST | Embedded MCP client host supporting stdio servers and remote servers (streamable HTTP) with OAuth 2.1 flows. |
| FR-MCP-2 | P2 | MUST | First-party connector catalog at launch: Outlook 365 (mail + calendar), Gmail, Google Calendar, Google Drive, Slack, Microsoft Teams chat, Linear, Todoist, Notion, GitHub, Zoom recordings. |
| FR-MCP-3 | P2 | MUST | Per-connector permission scoping surfaced at connect time (read-only vs. read-write); write-capable tools route through FR-AGT approval gates. |
| FR-MCP-4 | P2 | MUST | Users can register arbitrary third-party MCP servers (command or URL) with the same permission model. |
| FR-MCP-5 | P2 | SHOULD | Connector health dashboard: auth status, last successful call, error surfacing, re-auth flow. |
| FR-MCP-6 | P2 | SHOULD | Credentials stored in the OS keychain; never in config files or the sync payload plaintext. |

## FR-SYNC — Multi-device & multi-instance aggregation

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-SYNC-1 | P2 | MUST | End-to-end-encrypted sync of memory items, transcripts, summaries, settings, skills, and routine definitions across all of a user's devices. The relay server stores ciphertext only and cannot read user data. |
| FR-SYNC-2 | P2 | MUST | Device registry: each install gets its own keypair; enrollment via existing-device approval; per-device revocation and remote wipe of synced data. |
| FR-SYNC-3 | P2 | MUST | Consolidated cross-device timeline: activity from multiple desktops and mobile devices interleaves chronologically into one picture, preserving per-device/per-instance attribution. |
| FR-SYNC-4 | P2 | MUST | Offline-first with eventual consistency; conflict resolution via CRDTs for mutable objects (settings, routines) and append-only merge for timeline data. |
| FR-SYNC-5 | P2 | MUST | Multiple client instances on one machine coordinate through a local lock so exactly one captures audio/screen at a time (macOS permits a single system-audio capturer). |
| FR-SYNC-6 | P2 | SHOULD | Selective sync: user may exclude data classes (e.g., raw screen text) from leaving a given device. |
| FR-SYNC-7 | P3 | SHOULD | Search federates over all synced data regardless of which device captured it. |

## FR-MOB — Mobile companion apps

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-MOB-1 | P4 | MUST | iPhone app (first), then Android: chat over synced memory, briefing/routine delivery, push notifications. Available on Free and Pro. Design: [15-mobile-app-design.md](15-mobile-app-design.md). |
| FR-MOB-2 | P4 | MUST | Mobile approval surface: pending agent/routine write actions can be approved or denied from the phone. |
| FR-MOB-3 | P4 | SHOULD | In-person meeting recording via phone microphone, feeding the same transcription/summary pipeline and consolidated timeline. |
| FR-MOB-4 | P4 | MUST | No ambient screen capture on mobile in v1 (platform restrictions and battery); mobile contributes meetings, chat, and approvals only. |

## FR-ONB — Onboarding & permissions (cross-cutting)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-ONB-1 | P1 | MUST | Staged TCC permission onboarding: request Screen & System Audio Recording, Microphone, Accessibility, Calendar, and Notifications each at the moment its value is demonstrated, with a progress checklist and deep links to System Settings. |
| FR-ONB-2 | P1 | MUST | The app functions with partial permissions (e.g., meetings-only if screen capture is declined) and clearly shows which capabilities are inactive and why. |
| FR-ONB-3 | P1 | SHOULD | First-run privacy explainer: what is captured, where it is stored, what leaves the device, with links to controls. |

## FR-UI — Visual design & UI polish (cross-cutting)

Reference material: 17 screenshots of the real Littlebird macOS app, `~/Desktop/Screenshots/Littlebird UI/`. Reviewed once during the M1.1 settings-window restructuring (that work adopted the *structural* split — config in Settings, live status/actions in the debug window — that the screenshots showed), but the *visual* language below was never itself turned into tracked work and hasn't started. Functionally complete screens (Settings panes, timeline, chat) currently use bare default SwiftUI styling, not a deliberate visual design.

**FR-UI-1 is done.** Custom Dock/About icon and menu-bar glyph landed: the whimbrel's hover-hunt pose (wings raised, tail fanned) rebuilt from flat angular facets, palette drawn from the bird's own plumage (rust, slate-blue, ochre) rather than generic app-blue. Vector source in `apps/macos/design/`, compiled assets in `Whimbrel/Assets.xcassets/` (`AppIcon.appiconset`, `StatusGlyph.imageset`); `StatusIcon.swift` composites the template glyph with a small color badge (red = recording, amber = on but degraded/missing a permission) instead of the old four-SF-Symbol switch.

**FR-UI-5 is done.** Shared design system landed in `apps/macos/Whimbrel/UI/DesignSystem.swift`: 8 adaptive color tokens, serif/sans type pairing (`Font.whimbrelDisplay`/`whimbrelBody`), a 4pt spacing scale, `WhimbrelCardStyle`/`.whimbrelCard()`, `WhimbrelPillButtonStyle`, and `AccentBarRow`. Applied to Settings, the debug window, and (via FR-UI-2 below) Timeline.

**FR-UI-2 is done.** Timeline's plain `List` replaced with `whimbrelCard`/`AccentBarRow` rows (slate accent = screen-text capture, rust accent = meeting), and previously write-only meeting records (`CoreMemoryStore.recentMeetings()`) now surface in "Recent" mode, interleaved with captures by timestamp. Digest ended up **not** merged as a per-item row — a digest is generated on demand for a date range, not an individually-listed historical item, so it stays its own tab (see `docs/superpowers/specs/2026-07-26-timeline-visual-pass-design.md`). Two known follow-ups, not gating: (a) `AccentBarRow`'s bar sits inset within `.whimbrelCard()`'s padding rather than flush to the card edge — neither FR-UI-3 (bubbles, no accent bar) nor FR-UI-4 (deliberately doesn't consume cards at all) turned out to be a natural home for this fix, so it's now un-scheduled: fold a `DesignSystem.swift` flush-bar variant into whenever that file is next touched, rather than a specific future FR-UI pass; (b) "Recent" merges `recent(limit: 50)` captures with `recentMeetings(limit: 20)` independently before sorting, so at high capture volume the list can read as "recent captures, then older meetings" rather than fully interleaved — a shared recency-windowed retrieval across both tables is real future work, not a bug in this pass.

**FR-UI-3 is done.** Ask mode's plain stacked text (bold query, plain answer, flat citation list, `Divider()` between exchanges) replaced with iMessage-style bubbles: right-aligned rust (`whimbrelAccentPrimary`) query bubbles, left-aligned `.whimbrelCard()` answer bubbles with citations (capped at 2 lines, matching Timeline's own citation-field precedent) tucked inside. Digest mode and suggested-prompt chips stay out of scope (see `docs/superpowers/specs/2026-07-26-chat-visual-pass-design.md`). Known follow-ups, not gating: (a) the dark-mode query bubble's white-on-rust contrast (~3.26:1) is below WCAG AA for body text — root cause is `DesignSystem.swift`'s dark rust token itself, worth darkening whenever that file is next touched; (b) no auto-scroll to the newest exchange, more noticeable now that bubbles are taller than the old stacked text — `.defaultScrollAnchor(.bottom)` whenever someone picks it up; (c) no shared `Radius`/bubble-width token exists yet in `DesignSystem.swift` — four corner-radius/width literals now live across three files (`DesignSystem.swift`'s card, `TimelineView.swift`'s bubbles, and both `ControlWindowView.swift` and `MenuBarContentView.swift`'s meeting-prompt blocks) — fold into the next design-system touch.

**FR-UI-4 is done.** `MenuBarContentView.swift` restyled with the shared design system, plus two quick-glance actions that already existed elsewhere but weren't reachable from the popover: a "Timeline…" button (`openWindow(id: "timeline")`, the same call `ControlWindowView` already makes) and accept/dismiss controls for a pending meeting-detection prompt (`MeetingTriggerService.acceptPrompt()`/`.dismissPrompt()`, same calls and identical `Color.orange.opacity(0.15)` visual treatment as `ControlWindowView`'s `meetingsSection`, deliberately not `.whimbrelCard()` — same underlying state should read identically across both surfaces). Verified against a real running app, including a genuinely heuristic-triggered meeting prompt (see `docs/superpowers/specs/2026-07-26-menubar-popover-design.md`). This closes out the four-part visual-polish sequence (FR-UI-2 through FR-UI-5). One follow-up, not gating: the reentrancy-safety deferral (`Task { @MainActor in ... }` around `acceptPrompt()`/`toggleCapture()`) now has two independent copies (`ControlWindowView.swift`, `MenuBarContentView.swift`) that must stay in sync by hand if the underlying SIGSEGV pattern (`Whimbrel-2026-07-25-152222.ips`) is ever revisited — the more robust fix is moving the deferral inside `MeetingTriggerService.acceptPrompt()`/`AppState.toggleCapture()` themselves so no call site can miss it, letting both current copies drop their wrappers entirely. Not done here since both of those files were out of scope for this pass.

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-UI-1 | P1 | SHOULD | ~~Custom app icon (Dock + About panel) and a purpose-drawn menu-bar status icon with distinct on/off/recording states — today both are Xcode/SF Symbols placeholders.~~ Done. |
| FR-UI-2 | P1 | SHOULD | ~~Timeline view visual pass: card/list styling, spacing, typography, and per-item-type visual treatment (screen text vs. meeting) informed by the reference screenshots, replacing the current plain `List`.~~ Done. Timeline capture rows now also open a detail window on tap (full untruncated text), and the capture pipeline records/exposes whether each snippet came from the Accessibility API or OCR fallback. |
| FR-UI-3 | P1 | SHOULD | ~~Chat view visual pass: message bubbles, citation affordance styling, consistent with the timeline's visual language rather than a bare `List`/`Text` stack.~~ Done. |
| FR-UI-4 | P1 | MAY | ~~Menu-bar popover redesign matching the reference screenshots' quick-glance layout (currently `MenuBarContentView.swift` is minimal/functional only), plus quick-glance access to Timeline and a pending meeting prompt.~~ Done. |
| FR-UI-5 | P1 | MAY | ~~A shared design pass (color palette, spacing scale, type scale) applied consistently across Settings, debug window, timeline, and chat, instead of each screen's ad hoc SwiftUI defaults.~~ Done. |

Deliberately out of scope for this pass: this is visual polish only, not new functional surface — no new screens, no changed information architecture beyond what FR-UI-2…4 call out. Not blocking for M1's dogfooding exit gate (per `05-roadmap.md`), since it's cosmetic rather than a correctness/reliability gap.
