# 13 — Mac UI parity with the Windows shell

**Date:** 2026-09-11. **Why:** after using both apps, the author's view is that the Windows UI (M3.2) is already better than the Mac's. This doc compares the screens both apps have, says why the Windows one reads better, and lists the Mac gaps as backlog items. It compares code, not screenshots, so anything purely visual (rendering, font metrics) is out of its reach.

**Scope:** shell and navigation, Today, Library (with Ask and Digest), item detail, Settings, and the visual system. Meetings and Automations are left out because Windows doesn't have them yet (M3.3, M3.4).

## Why the Windows UI reads better

It was designed as one system in one pass (docs/12 §6, the M3.2 plan), and the Mac grew screen by screen from the M1 debug window. That shows up in five ways:

1. **One place per thing.** Windows has one window, five tabs, and a detail view that replaces the screen with "← Back". The Mac spreads the same content over a shell window, four detail-window groups, a separate Settings window, a menu bar popover, and a fixed 280pt at-a-glance panel. The activity toggle and Start Meeting live in both the menu bar and the Library header; digest generation is written three times (Today, at-a-glance, Timeline); Routines and Pending Approvals appear in both Automations and Settings.
2. **Every list and state looks the same.** Windows uses one row card, one chip, one section label, and the same loading, empty and error treatment on every screen. The Mac mixes `List(.sidebar)`, plain `List`, and card stacks; four of the six Library modes have no loading state; digest failures on Today and in Library clear silently.
3. **No development residue.** The Mac still shows a "Stream Preview" button in Ask, a Diagnostics pane, requirement codes in titles ("AI & Routing (FR-CHAT-2/5)"), "capture spike" copy, and runs `DiagnosticsPrinter` and `CoreSelfTest` whenever Library appears.
4. **Consistent names.** On Windows the Library tab is titled Library. On the Mac the window is "Whimbrel", its id is `timeline`, the tab is "Library", the heading is "Timeline", and the menu item is "Timeline…".
5. **Settings as cards.** Windows Settings is five cards on one scrolling tab (Sync, AI & routing, Activity capture, Semantic search, About). The Mac's is fifteen sidebar panes in a separate window that opens on Permissions.

Where the Mac is ahead, and should stay ahead: the meeting workspace and Meetings tab, the at-a-glance schedule with Join buttons, the menu bar popover, transcript search results, and multi-window detail (a Mac convention the Windows app doesn't need).

## Screen by screen

| Area | Windows (`apps/windows/ui/src`) | Mac (`apps/macos/Whimbrel/UI`) |
|---|---|---|
| Shell | 200px sidebar tablist; Settings is a tab; Meetings and Automations show "Coming in M3.x" | `NavigationSplitView` sidebar; Settings tab only says "opens in its own window"; update banner above |
| Today | Four cards: daily recap (Refresh), open loops, latest meeting (opens Detail), "Across your devices today" | Meeting workspace first (active, next, or last meeting); then an approvals count with no link, recap text, and open loops as plain text; no device summary |
| Library list | Six modes; rows grouped under Today / Yesterday / date headings; every row has a device chip | Six modes; flat list; device labels on capture, meeting and episode rows, not on transcript hits or any detail view |
| Library header | Mode picker plus per-mode controls | "Timeline" title, a capture status card with the activity toggle, Settings…, Start/End Meeting and an in-meeting notes editor, then the mode picker |
| States | Loading, empty (per-mode message) and error (`role=alert`) on every screen | Recent, Keyword, Semantic and Episodes have no loading state; "No results yet." for all; digest failures are silent |
| Ask | Conversation list always visible (collapses under 720px), hover actions to star, rename and delete, citations as chips that open Detail, stream status lines, spend line | Conversation list behind a Show/Hide button; citations as "• (label) text" lines; Anthropic and OpenAI don't stream; a failed stream silently retries without streaming; empty state says "answered locally via Ollama" even with a hosted provider selected |
| Detail | Inline, max 820px, device chip on every variant | Separate windows with a floating close button, no device label |
| Settings | Five cards on one tab; masked keys "Saved · …1234" | Fifteen panes; Diagnostics pane; requirement codes in titles; opens on Permissions |
| Visual system | Tokens ported from `DesignSystem.swift`; one button style; two unused tokens; no type-scale tokens | Same palette and spacing; components like `WhimbrelEmptyState` used in two places; raw `.orange`, `.red`, `.blue`, `.callout` and literal radii in several views |
| Keyboard, a11y | Enter to send; focus rings; ARIA roles on tabs, groups, alerts, live regions | No `.keyboardShortcut` anywhere; no accessibility modifiers in Timeline, Today or Settings |

## Backlog: Mac gaps, cheapest first

Effort: S = under a day, M = one or two days.

| # | Item | Effort | Where |
|---|---|---|---|
| 1 | **Done.** Remove development residue: the Stream Preview button, requirement codes in titles, "capture spike" copy; move Diagnostics behind a hidden toggle; stop running `DiagnosticsPrinter` and `CoreSelfTest` on Library appear | S | `TimelineView.swift:235-238, 444, 516-531`, `SettingsView.swift:124, 220, 432, 1082-1103` |
| 2 | **Done.** One name: "Library" for the heading and settings copy; the menu item becomes "Open Whimbrel", matching the Windows tray, rather than switching tabs (that would mean lifting the shell's tab selection into shared state) | S | `TimelineView.swift`, `MenuBarContentView.swift`, `MeetingAlertView.swift`, `SettingsView.swift` |
| 3 | **Done.** Honest states: loading indicators for the four list modes, a message when a digest fails (Today and Library), and an Ask empty state that names the selected provider | S | `TimelineView.swift:476-481, 876-881, 1177-1179`, `TodayView.swift:263` |
| 4 | **Done.** Drop the Library header's Settings… button (the shell has a Settings tab and ⌘,). The activity toggle, Start/End Meeting and the notes editor stay: the menu bar item was unreliable on a real machine (Control Center hosting, overflow), and Settings has no capture toggle yet | S | `TimelineView.swift` header |
| 5 | **Done.** Day headings in Library lists (Today, Yesterday, date), as Windows' `rows.ts` does | S | `TimelineView.swift` list builders |
| 6 | **Done.** One device chip component, shown on transcript hits and in every detail view | S | `TimelineView.swift:638-660, 766-778`, detail views |
| 7 | **Done.** Open loops open their thread; the approvals count has a Review button that switches to Automations (the shell's tab selection moved to `AppState`); an "Across your devices today" card lists each device's captures today, This Mac first. Counts come from a new core query, `Store::capture_activity_by_device` (FFI `whimbrel_memory_capture_activity_by_device`), which Windows can use too | M | `TodayView.swift`, `WhimbrelShellView.swift`, `AppState.swift`, core/memory, core/ffi |
| 8 | **Done.** The conversation list shows by default (Hide still collapses it) and refreshes on entering Ask; citations are source chips under "Sources" that open the cited item, with the full text as a tooltip; the answering model shows under each answer. Saved conversations still reload without citations or model, a shared core gap (below) | M | `TimelineView.swift`, `TimelineView+Ask.swift`, `DesignSystem.swift` |
| 9 | **Done.** Fifteen panes become seven sections (eight in Debug builds), each a stack of cards built from the existing panes: General (versions and permissions), Capture & Privacy (capture and retention), Meetings (meetings and transcription), AI (routing and prompts), Sync & Devices, Connectors (connectors and MCP clients), Automations (routine settings), and Diagnostics in Debug builds. Settings opens on General. The routines inbox and pending approvals are dropped here because the Automations tab has them | M | `SettingsView.swift` |
| 10 | **Done.** Replace ad-hoc colors, fonts and radii with tokens; add warning, error and link tokens | S | `ProviderModelPicker.swift:120-136`, `MenuBarContentView.swift:47`, `MeetingDetailView.swift:615, 626`, `TimelineView.swift:593, 746-749, 838, 869` |
| 11 | **Done.** A Go menu with ⌘1–⌘5 for the tabs, ⌘F (Search Library: switches to Keyword unless a search mode is already on, and focuses the field) and ⌘N (New Conversation in Ask); all three bring the main window forward. VoiceOver: Library rows and conversation-list rows are activatable buttons with a combined label, the star button and detail windows' close buttons have labels, and Today's device rows read as one line | M | `WhimbrelApp.swift`, `AppState.swift`, `TimelineView*.swift`, `TodayView.swift`, detail views |
| 12 | **Done.** `TimelineView.swift` (1,650 lines) is now 608 lines plus `TimelineView+Ask`, `+Rows`, `+Header` and `+Digest`; `SettingsView.swift` (1,749) is now 119 lines plus one file per pane in `UI/Settings/`. Members moved verbatim; `TimelineView`'s members and four helper types lost `private` so the extensions can reach them | M | same files |

All twelve items are done (#139–#153).

## Shared gaps (fix once in core, both platforms benefit)

- ~~Saved conversations lose their citations on both platforms.~~ **Done:** each app saves the entries an answer cited (label, tooltip text, where it opens) in `citations_json`, and core stores the model that answered. Conversations don't sync, so the JSON shape is each app's own.
- Neither platform has type-scale tokens, and Windows defines two tokens it never uses (`--radius-surface`, `--color-accent-highlight`).
- ~~Anthropic and OpenAI answers don't stream on the Mac.~~ **Done:** core has Anthropic and OpenAI streaming adapters, and both apps stream a keyed Anthropic or OpenAI question.

## What this says about the three options

The gaps are about information architecture, consistency and leftover development UI. None of them is a SwiftUI limit. That argues for option 1 (port the patterns in SwiftUI) now and option 2 (treat the Windows layout as the reference design for new screens) going forward. Nothing here makes the case for option 3 (running the Tauri/React UI on the Mac); that would need its own design doc, weighing one UI codebase against the menu bar, multi-window and native-control conventions the Mac would lose.
