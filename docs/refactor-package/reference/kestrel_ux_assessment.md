# Kestrel macOS UX Assessment and World-Class Redesign Specification

## Framing and confidence

This is a design- and behavior-level assessment. The present Kestrel screens and SwiftUI implementation were not available for inspection, so the documented overlap and wasted space are **reported issues**, not visual findings from a build. The proposed system is intentionally specific enough to guide a redesign and visual-regression test plan without inventing implementation facts.

## Competitive standard and opportunity

Granola positions the meeting as a note-first workspace: intentional start, visible recording state, a live transcript panel, direct stop/resume, AI-enhanced notes, and a bounded upcoming-meetings surface. [1] [2] Littlebird positions meetings as inputs to a broader work memory, combining automatic transcription with pre-meeting preparation from email, past meetings, and projects and later contextual recall. [3] Kestrel can be more trustworthy than both by fusing **Granola’s deliberate capture control** with **Littlebird’s contextual recall**, while differentiating through on-device/local-model routing, explicit data lineage, and control that never disappears.

> **Product thesis:** Kestrel should feel like an exceptionally calm meeting cockpit: it is immediately clear whether it is listening, what it knows, what it is doing, where data is going, and how to stop it.

| Dimension | Granola benchmark | Littlebird benchmark | Kestrel world-class resolution |
|---|---|---|---|
| Capture trust | Deliberate start and persistent active indicator; stop/resume inside transcript | Calendar-linked, bot-free computer-audio workflow | Deliberate start by default, clear audio-source health, command-level Stop in three reachable locations, and “Buffer & Confirm” at meeting boundaries |
| Live work | Transcript and note in a meeting-first canvas | In-meeting questions and a post-call summary | A stable three-mode canvas: notes, transcript, and assistant share a single meeting context without competing for the same pixels |
| Context | Calendar context and related meetings | Prep draws on email, past meetings, projects | “Brief” is grounded, source-cited, optionally local-only, and never blocks capture |
| Recall | Notes and linked recurrences | Cross-app memory and natural-language recall | Local full-text first, opt-in semantic recall second, source citations and one-click jump to precise transcript time/range |
| AI choice | Product-managed experience | Broad assistant/memory product | Provider-neutral model picker that reveals only decision-relevant capabilities, defaults to a recommended model, and never expands the primary toolbar |
| Operational clarity | Public calendar setup/troubleshooting guidance | Public privacy controls and data rights | Compact normal-state status; rich per-source Sync & Privacy Control Center only when attention is needed |

## What “wasted UI space” likely signals

Empty space is not inherently waste. Apple’s layout guidance asks designers to provide sufficient room for essential information, group related items, and preserve content/control distinction. [4] The reported concern instead suggests **low information utility per pixel**: decorative or permanently reserved regions dominate while the meeting, transcript, notes, and action controls require scrolling or compete with one another. The remedy is not maximal density; it is a clear hierarchy in which every persistent region earns its space.

### Primary information hierarchy

The app must answer, at a glance, in this order: **am I recording; what meeting is this; what do I need to do now; what was said; what can I ask or create; is anything at risk**. Sync details, model specifications, raw trace IDs, and configuration must remain available without occupying the ordinary working surface.

| Rank | Persistent information/control | Location and behavior | Must never be displaced by |
|---|---|---|---|
| 1 | Capture state, elapsed time, Stop | Global status strip and menu bar; red/amber/neutral semantics plus text | Model choice, sync banner, assistant output, or window resize |
| 2 | Meeting identity and lifecycle | Compact context header: title, calendar/account, attendee count, privacy routing, finalizing/completed state | Long titles; title truncates with accessible full value |
| 3 | Human note | Center canvas; always editable; keyboard focus stable | Transcript arrival, AI streaming, sync update |
| 4 | Live transcript | Optional resizable side panel; follows current turn only when user has not scrolled away | A chat panel or model selector |
| 5 | Assistant | Bottom composer / collapsible inspector; streamed response remains in a dedicated cell | The recording controls or editing canvas |
| 6 | Sync and diagnostics | Compact status dot normally; dedicated control center on demand | Everyday meeting workspace |

## Window architecture

### 1. App shell

Use the familiar macOS split-view grammar: a **collapsible sidebar**, a **single primary content canvas**, and one optional **inspector**. Do not create a permanent three-column layout merely because three data types exist. On smaller windows, only the selected working mode is visible beside the canvas; remaining context is available through a one-keystroke panel or adaptive segmented control.

| Region | Purpose | Width/visibility rule | Interaction contract |
|---|---|---|---|
| Sidebar | Today, Meetings, Search, Spaces/Folders, Settings | Default 220–264 pt; collapsible; remembers user preference | Sidebar rows are navigation, not actions; current location is always obvious |
| Context header | Meeting title, lifecycle, participants, preparation, privacy mode | One/two lines max; content compresses before controls move | Long metadata moves to overflow/details; Stop never goes into overflow |
| Canvas | Notes and meeting output | Takes all remaining primary width | Content uses a readable line length and independent vertical scroll |
| Transcript inspector | Live source text, search, speaker/source distinction | 320–420 pt; user-toggleable/resizable; becomes sheet/segmented view at narrow size | New chunks do not steal focus or force scroll if user is reviewing history |
| Assistant composer | Ask, generate, model choice, Stop inference | Anchored bottom; grows vertically only to a capped height | Model picker appears in a popover/sheet; streamed reply is not inside the text field |
| Status strip | Recording, sync attention, finalization/inference state | Thin, semantic, transient unless active/failed | Always readable by VoiceOver and never overlays typing controls |

### 2. The recording cockpit

While capturing, this must be the most disciplined screen in the product. The user should be able to look for less than a second and know capture has started, which sources are healthy, how long the session has run, and how to terminate it. The app window, a floating return-to-meeting pill, and the menu-bar item must all execute the **same** Stop command rather than duplicating logic.

```text
[● Recording  18:42  Mic healthy · System Audio healthy]                 [Stop ⏹]
[Weekly client discovery — Acme] [Private/local-only ▼] [Brief] [More]
────────────────────────────────────────────────────────────────────────────────
[Editable notes canvas                                                      ]
[• Customer wants Q3 implementation; pricing needs VP approval.           ]
[                                                                        ]
────────────────────────────────────────────────────────────────────────────────
[Live transcript ▸] [ Ask Kestrel about this meeting… ] [Model: Recommended ▼]
```

The design must distinguish **Capturing**, **Paused**, **Finalizing**, and **AI working** with text, color, icon, and accessible announcement. “Finalizing transcript” must not look like continued recording. Pressing Stop changes the capture state immediately; it may retain a small, non-blocking finalization indicator for the independent downstream work.

### 3. AI without UI sprawl

The Google-model regression indicates an ownership problem: provider-specific controls are likely being inserted into a global horizontal control row. Replace that approach with a single **Model** affordance. The default label should be “Recommended” or the user’s chosen default, with a provider/model name in a secondary line only when it helps. The picker belongs in a popover, command palette, or settings sheet. It can show capability badges, expected locality, and availability without expanding the active meeting layout.

| User need | Primary UI | Secondary detail | Never do |
|---|---|---|---|
| Choose a model | One compact menu in assistant composer | Searchable popover grouped by Local / Private Cloud / Cloud; model availability and streaming badge | Render one button or chip for each provider/model in the main window |
| Understand a wait | Inline run status: “Loading local model · 00:14” | Reveal timing, queue, and technical trace under “Details” | Replace user controls with an indeterminate spinner |
| Stop work | Text-and-icon Stop located in active response cell and shared command palette | “Keep partial response” after cancellation | Hide cancel in an overflow menu or disable it during stream setup |
| Change model during run | Disabled with explanation; “Stop and retry with…” action | Preserve input and partial response separately | Let late output overwrite the new model’s response |
| Know data path | Small “Local only” / provider badge adjacent to send | Policy sheet with exact provider/data categories | Infer consent from model brand or bury routing in settings |

## Core experience flows

### A. Scheduled meeting

The user opens an event from a concise **Up next** list or a notification. Kestrel shows a brief only if one is already available, with recording idle until the user confirms start. At the scheduled start time, it may surface a focused “Ready to capture?” control but should not begin unexpectedly. This mirrors Granola’s emphasis on deliberate start while avoiding an accidental-recording perception. [2]

Once started, capture state becomes persistent. The transcript appears only if the user opens it; notes retain focus. The assistant can receive an in-meeting question, but response streaming uses its own cell and never shifts focus away from notes. At Stop, the user immediately sees “Recording stopped”; background tasks move to finalizing. If the next scheduled meeting is imminent or a new call is detected, Kestrel enters **Buffer & Confirm**, asking whether to continue the current session or start a new one. This boundary is mandatory for client-confidential workflows.

### B. Ad-hoc meeting

The global new-meeting action opens a minimal identity sheet: title, optional calendar association, capture sources, and data routing policy. A retained default enables rapid start, but the screen never auto-submits. Kestrel should remember the user’s choice of screen/system audio only with visible prior consent and source health.

### C. Slow or failed AI run

After submit, the UI changes in under one interaction frame to “Connecting” or “Loading local model,” with a Stop action. Partial output streams into a discrete response block. If no first token arrives, elapsed time and a concise recovery action appear; the user can stop, retry, or change model without losing the prompt. On cancellation, Kestrel stops showing new output and asks whether to keep the partial result as a draft. This is especially important for Ollama, whose documented streaming support makes a blocking all-at-once response unnecessary. [5]

### D. Sync attention

Normal sync creates no toast churn. A small status indicator reports “Up to date,” “Syncing changes,” or “Needs attention.” Only actionable or durable problems expand into a banner; the banner identifies the exact source, scope, last successful sync, local safety posture, and next action. A user who is offline should see “Saved on this Mac; will sync when online,” never a scary generic failure.

## Functional gaps and design resolutions

| Reported gap | UX diagnosis | Resolution | Objective gate |
|---|---|---|---|
| Wasted UI space | Insufficient hierarchy and persistent low-value chrome | Rebuild around a collapsible sidebar, primary note canvas, optional inspector, and anchored composer; default to one working surface | A representative meeting lets users view/edit notes, see recording state, and stop capture without scrolling or resizing at the minimum supported window size |
| Overlap after Google models | Model/provider controls own layout rather than using a stable interaction contract | One capability-driven model picker inside composer; no provider-specific global layout elements | Zero collisions/occlusions with 10+ descriptors, minimum width, large accessibility text, localization expansion, and dark/light appearances |
| No streamed output | Inference treated as a blocking request | Normalize all providers into partial deltas; update a dedicated response block | Visible started/progress state within 500 ms; first token rendered as soon as the provider emits it |
| Cannot stop LLM | User lacks authority over provider work | Unified cancellation command, current-run token, stopped state, late-chunk suppression | Stop is reachable by pointer and keyboard and reliably prevents new visible content after confirmation |
| Cannot reliably stop recording | Capture lifetime conflated with finalization or hidden by layout | Persisted state machine; global Stop owned in domain core; immediate screen/menu/pill state transition | Stop produces a `finalizing`/`idle` visual transition under 250 ms even if cloud/model work is slow |
| Intermittent sync | State is opaque and likely not independently recoverable | Local-first outbox, connector-specific status, actionable control center, redacted diagnostic export | Offline stop/edit/create actions survive forced termination and replay exactly once on relaunch |
| Weak long-term value | Isolated summaries rather than a grounded memory system | Transcript full-text index first; optional semantic index; source-linked recall and related-meeting graph | Answers cite meeting/date/segment and jump to the original source; user can restrict which spaces are indexed |

## Accessibility, cognitive load, and macOS quality requirements

A world-class standard means the UI behaves predictably during dynamic events. Transcript updates, sync notices, and streamed model deltas must not move keyboard focus, steal VoiceOver focus, or cause primary controls to move. The Mac user expects keyboard-first operation, durable window preferences, correct focus rings, native menu command discoverability, and responsive layouts under resizing.

| Requirement | Implementation expectation | Validation |
|---|---|---|
| Keyboard-first control | Documented shortcuts for start/stop capture, pause, new meeting, focus note/transcript/assistant, search, and model picker | Complete a live meeting flow without pointer interaction |
| Focus stability | New chunks and status updates use non-interrupting accessibility announcements; never refocus composer/note | Type in notes while transcript and AI stream; cursor and focus do not move |
| Color independence | All state has icon/text shape in addition to color; error contrast meets system expectations | Grayscale and increased-contrast review |
| Text and layout adaptation | Semantic Dynamic Type-compatible fonts, intrinsic size, constraints, scroll/overflow at narrow widths | Visual snapshot grid at minimum/window-resized widths and large accessibility text |
| Motion/reduced distraction | Respect Reduce Motion; optional subtle waveform animation; no repetitive success toasts | Review with Reduce Motion and long meetings |
| Error recovery | Plain language with a single primary remedy, no raw provider jargon in normal flow | Simulate permission, offline, token-revoked, full-disk, and model-unavailable states |

## UX release gates

No “world-class” claim should be made until Kestrel is verified through a recorded end-to-end evaluation. The following are release gates, not aspirational items.

| Gate | Required test | Pass criterion |
|---|---|---|
| Capture sovereignty | Start, pause, stop from note, menu bar, and keyboard while model streaming and network is offline | All routes invoke one state change; capture ends immediately; local data survives restart |
| Boundary safety | Back-to-back events, call app remains open, meeting runs over, and manual stop | No automatic cross-meeting merge without Buffer & Confirm; user can explicitly resolve identity |
| Streaming/cancel race | Cancel Ollama and Gemini at connect, first delta, midstream, tool step, and after model switch | No late text, no blocked composer, no orphan process/socket, partial content state is coherent |
| Sync resilience | Airplane/offline, 429, token expiry, server 5xx, crash mid-push/pull, duplicate inbound event | No lost/duplicated domain record; exact recovery action and last-good state exposed |
| Configuration explosion | 10+ models, unavailable/local/remote descriptors, smallest window, large text, long localized labels | No overlap, clipped controls, hidden Stop, or excessive permanent empty space |
| Recall trust | Ask a question spanning old meetings; change or delete a source | Answer identifies source ranges; regenerated output reflects current allowed corpus; deletion propagates visibly |

## References

[1]: https://docs.granola.ai/help-center/getting-started/syncing-your-calendars "Granola — Syncing your calendars"
[2]: https://docs.granola.ai/help-center/taking-notes/transcription "Granola — How transcription works"
[3]: https://littlebird.ai/features/meeting-notes "Littlebird — AI Meeting Notes, Bot-Free"
[4]: https://developer.apple.com/design/human-interface-guidelines/layout "Apple — Layout, Human Interface Guidelines"
[5]: https://docs.ollama.com/capabilities/streaming "Ollama — Streaming"
