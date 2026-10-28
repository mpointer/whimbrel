# Kestrel macOS UX Wireframes and Layout Specification

**Design objective.** Kestrel’s macOS experience should operate as a **calm, high-control meeting cockpit** rather than an accumulation of panels, model controls, and status messages. The redesign removes wasted permanent chrome, prevents model-driven overlap, isolates transient operations from editable work, and makes recording state undeniable.

> **Principle:** Essential state is always visible; secondary capability is one interaction away; diagnostics appear only when they can change a user’s next action.

The attached SVG wireframes are intentionally structural rather than brand-final. They establish content hierarchy, component behavior, and spatial constraints that can be transferred directly into SwiftUI. Their labels are representative product copy, not a requirement to preserve exact wording.

## 1. Design system foundation

### 1.1 Visual direction

Use a calm near-white/near-black native macOS material surface, a single Kestrel teal for active/successful listening, amber for delayed/attention, red only for Stop/failure and never as generic decoration. Typography should use SF Pro/Text/Mono semantic styles supplied by the platform. Corner radii, separators, and materials must be subtle enough to preserve scanning density.

| Token | Light value | Dark value | Use |
|---|---|---|---|
| `canvas` | `#FAFAF9` | `#171817` | Primary note/work area |
| `sidebar` | `#F2F3F1` | `#202220` | Navigation hierarchy |
| `surface` | `#FFFFFF` | `#262826` | Cards, composer, popovers |
| `text.primary` | `#171817` | `#F5F5F2` | Titles/body |
| `text.secondary` | `#6A706B` | `#AAB0AB` | Metadata and quiet status |
| `accent.listening` | `#0F766E` | `#2DD4BF` | Active capture and successful state |
| `attention` | `#B45309` | `#FBBF24` | Delayed sync/recoverable warning |
| `critical` | `#B91C1C` | `#F87171` | Stop/failure only |
| `divider` | `#E2E5E1` | `#353A36` | Low-emphasis group boundary |

| Token | Default | Rule |
|---|---:|---|
| App minimum width | 980 pt | Below this, transcript converts from inspector to overlay/segmented view; essential Stop remains visible |
| App optimum width | 1,280–1,600 pt | Displays sidebar, canvas, and transcript inspector simultaneously |
| Sidebar width | 232 pt | May collapse to 0; never shrink below 216 pt if visible |
| Transcript inspector | 360 pt | User-resizable from 320–460 pt; collapses before canvas becomes unusable |
| Context/header height | 84 pt maximum | Includes meeting title, state, privacy route, and action cluster; no provider chips allowed |
| Active state strip | 32 pt | Present only for capturing/paused/finalizing/attention; never an overlay |
| Composer height | 52 pt collapsed | May grow to 116 pt only while user input wraps; assistant response stays above, never inside field |
| Standard spacing | 8/12/16/24/32 pt | Use a strict spacing scale, not ad hoc offsets |

### 1.2 Component rules

| Component | Required behavior | Prohibited behavior |
|---|---|---|
| `CaptureStatusStrip` | Displays icon, textual state, elapsed time, source health, and canonical Stop while capturing | Hiding Stop in overflow; using only color/waveform to convey capture state |
| `MeetingHeader` | Holds meeting identity, privacy data-route, Brief, and More | Expanding with model/provider controls; more than two content lines |
| `NoteCanvas` | Primary and stable typing target; independent scroll/selection | Stealing focus for incoming transcript, AI response, or sync completion |
| `TranscriptInspector` | Optional, resizable, search/filterable, preserves manual scroll | Auto-opening on every new segment or covering editor at normal width |
| `AssistantComposer` | One prompt field, send, compact model picker, always-reachable inference Stop | Embedding per-model horizontal controls; becoming a multi-row toolbar |
| `ModelPicker` | Popover/sheet grouped by Local, Approved Cloud, Other permitted; search and capability labels | Displaying every provider in main shell or allowing disallowed policy choice |
| `SyncStatus` | Single quiet status dot normally; source-named actionable state on failure | Generic blocking spinner or toast flood |

## 2. App-shell information architecture

The default workspace uses a **two-column shell with an optional third inspector**. The sidebar is navigation only; the central canvas is the sole primary work surface. The inspector is never a permanent requirement. This removes vacant multi-panel space while retaining instant access to the transcript when it matters.

### 2.1 Sidebar

The sidebar is organized by intent, not by data type: **Today**, **Meetings**, **Search**, **Spaces**, then **Settings**. It may show a concise “Up next” card with one meeting and an affordance to expand, but it should not replicate the full calendar. Keep sidebar metadata compact; date, title, and state are enough for scanability.

| Sidebar state | Width | Contents | Trigger |
|---|---:|---|---|
| Standard | 232 pt | Primary navigation, one compact Up Next group, pinned spaces | Default ≥1,100 pt |
| Collapsed | 0 pt | Toolbar/sidebar toggle remains | User choice or constrained width |
| Search mode | 296 pt | Search field, filters, result facets | Explicit Search command; replace navigation temporarily |
| Narrow width | 0 pt or overlay | Sidebar is not simultaneously shown with transcript inspector | Width <980 pt |

### 2.2 Central context header

The header gives immediate context without creating an oversized hero. It is a 16 pt top/side-padded region, with title/metadata on the left and a **fixed action cluster** on the right. Horizontal overflow never pushes an action onto the note canvas; metadata truncates and moves to a Details popover.

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ Weekly client discovery – Acme                              [Brief] [•••]    │
│ Today, 10:00–10:30 · 4 attendees · Local only                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 2.3 Primary canvas

The canvas defaults to a human editable note, because users need a stable surface throughout the meeting. The note uses a comfortable content column of 640–800 pt maximum within the available workspace, preserving readability while not wasting a fixed blank right pane. At wider widths the note column is left-aligned inside the canvas and supports contextual blocks such as Agenda, Decisions, and Action Items; those blocks are in the document, not hard-coded chrome.

## 3. Wireframe A — Today / ready-to-capture workspace

See **`Kestrel_Wireframe_A_Today.svg`**. This is the calm, non-recording home state. The highest-value content is the next meeting and the last few notes, but the design avoids a large empty dashboard. A small active-day summary is useful; cards serve a navigation purpose rather than decoration.

| Callout | Requirement | Rationale |
|---:|---|---|
| 1 | Sidebar begins with Today/Meetings/Search, then a small Up Next context group | Eliminates dashboard sprawl and keeps current work reachable |
| 2 | “Start ad-hoc meeting” is a compact secondary action, not a hero panel | Meeting start is important but does not deserve most of the screen when idle |
| 3 | Upcoming meeting row carries title, time, people count, and privacy/policy indicator | Enables a fast pre-capture decision without opening details |
| 4 | Recent notes use a dense list, not large masonry cards | Preserves information utility per pixel |
| 5 | Status footer says “All sources up to date” or compact actionable state | Trust is present but does not interrupt normal planning |

## 4. Wireframe B — live recording cockpit

See **`Kestrel_Wireframe_B_Recording.svg`**. This is the most important screen. A user must be able to stop capture instantly, distinguish capture from finalization, and write notes without a transcript or model control taking over the canvas.

| Callout | Requirement | Rationale |
|---:|---|---|
| 1 | 32 pt dedicated capture strip shows “Recording,” elapsed time, mic/system health, and Stop | State is unmissable and canonical Stop is always reachable |
| 2 | Meeting header is compact; data-route badge describes Local Only or approved provider | Consent and identity appear before any AI/provider control |
| 3 | Note remains central and editable | The product augments attention; it does not displace human note-taking |
| 4 | Transcript is an optional 360 pt inspector with its own search and source filters | Allows detailed review without permanently reserving unused blank space |
| 5 | Composer has exactly one model picker and one inference lifecycle location | Solves Google-model overlap at the structural level |

The strip must use text, icon, and color. A waveform alone is not sufficient for accessibility or user trust. The Stop button should be 84–96 pt wide with an icon and label; it is not merely an unlabelled square.

## 5. Wireframe C — slow/streaming/cancellable inference

See **`Kestrel_Wireframe_C_Streaming_Inference.svg`**. Inference progress occurs inside a dedicated assistant result block above the composer. The note remains fully editable. A streamed answer is rendered append-only, with source chips linking back to a transcript range or prior meeting. The model picker is not open by default.

| State | Result block presentation | User controls | Core projection needed |
|---|---|---|---|
| Connecting | “Connecting to Gemini…” + elapsed time | Stop | `Connecting` |
| Local warm-up | “Loading local model · 00:14” + calm progress | Stop; switch after stop | `WarmingLocalModel` |
| Streaming | Incremental output, source references, cursor | Stop; copy; collapse reasoning if policy allows | `Streaming` + `TextDelta` |
| Cancelling | “Stopping…” remains visible; partial output held | No duplicate action; later Keep/Discard | `Cancelling` |
| Cancelled | “Stopped — partial response kept as draft” | Keep/Discard, Retry | `Cancelled` |
| Failed | Human-readable reason, trace behind Details | Retry, Change model, View status | `Failed(class)` |
| Complete | Final result with source provenance | Apply to notes, copy, regenerate | `Completed` |

The user can stop an inference in every in-flight state. SwiftUI must move to `Cancelling` on core receipt, then `Cancelled`/`Failed`/`Completed` only on authoritative core projection. No late chunk may render after the `Cancelling` transition.

## 6. Wireframe D — model picker and sync/control center

See **`Kestrel_Wireframe_D_Control_Center.svg`**. The design uses compact overlays for depth—not permanent panels. The model picker opens from the composer and has a predictable width; it never causes a layout reflow. The sync/control center opens only from an attention state or explicit menu action and shows named sources, last good synchronization, pending local work, and a safe recovery action.

| Overlay | Anchor/size | Contents | Rules |
|---|---|---|---|
| Model picker | Composer anchor; 360–420 pt wide | Search, Recommended, Local, Approved Cloud, unavailable states, concise capability/locality labels | No modal requirement; selection updates only model label after close |
| Privacy route sheet | Header route badge; 420–520 pt | Per-meeting Local Only/approved route explanation and current policy | Policy enforcement is in Rust core, not this sheet |
| Transcript inspector | Trailing edge; 320–460 pt | Search, source filter, live/paused transcript, export/copy action | Convert to overlay/segmented view below minimum width |
| Sync control center | Status-dot/Settings; 480–600 pt | Per-connector status, last success, backlog age, Retry/Reconnect/Diagnostics | No generic “Sync now” without identifying source/scope |

## 7. Responsive layout and constraint specification

### 7.1 Width classes

| Width class | Range | Layout | Behavior |
|---|---:|---|---|
| Expanded | ≥1,280 pt | Sidebar + canvas + transcript inspector | Inspector visible if user preference is on; header actions displayed inline |
| Standard | 980–1,279 pt | Sidebar + canvas; transcript collapses by default | Transcript opens as trailing overlay or segmented content mode; composer unchanged |
| Compact | 760–979 pt | Canvas only by default | Sidebar and transcript are mutually exclusive overlays; capture strip/header/composer always retained |
| Below supported minimum | <760 pt | Do not optimize as a primary state | Enforce minimum app width or use a deliberately constrained single-pane fallback if product permits |

### 7.2 SwiftUI layout prescription

Use `NavigationSplitView` or equivalent split-view composition with `HSplitView`/resizable inspector at expanded width. Use `ViewThatFits`, `Layout`, size-class-like geometry preference, and intrinsic content sizing rather than absolute x/y placement. Avoid nested `GeometryReader`-driven fixed offsets for controls.

```swift
// Structural intent, not drop-in code.
NavigationSplitView(columnVisibility: $sidebarVisibility) {
    Sidebar()
        .frame(minWidth: 216, idealWidth: 232, maxWidth: 264)
} detail: {
    WorkspaceShell {
        CaptureStatusStrip()
        MeetingHeader()
        HStack(spacing: 0) {
            NoteCanvas()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            if layoutMode.showsTranscriptInspector {
                TranscriptInspector()
                    .frame(minWidth: 320, idealWidth: 360, maxWidth: 460)
            }
        }
        AssistantComposer()
            .frame(minHeight: 52, maxHeight: 116)
    }
}
```

| Constraint | Requirement |
|---|---|
| Stop control | `layoutPriority` highest; minimum 84 pt width; may not move to menu/overflow during any horizontal compression |
| Title/metadata | Truncate title before reducing essential control hit targets; metadata line may move into Details at compact width |
| Model picker | Fixed overlay/popover width; does not participate in header/composer width negotiation beyond its trigger |
| Transcript | Lower layout priority than primary note; collapses before note falls below 520 pt readable width |
| Composer | Pinned to safe-area bottom of workspace; AI result list scrolls above it, never beneath it |
| Sidebar | User collapsible; never squeezes into an icon-only unlabelled state that harms discoverability |
| Source badges | Wrap inside their own document/result card; never wrap header action cluster to an additional permanent row |

### 7.3 Overflow behavior

No action is removed merely because a model label is long. Instead, model display truncates at 160 pt with accessible full text; the picker retains full searchable name. Header metadata moves to a popover. The More menu contains low-frequency actions only: export, rename, duplicate, report a problem, and debug details. **Stop, privacy route, composer, and visible capture state never live in More.**

## 8. Interaction, focus, and accessibility specifications

| Interaction | Required behavior |
|---|---|
| Start recording | Core acknowledges capture transition, then strip changes to Recording; no active animation before actual acknowledgment |
| Stop recording | All Stop affordances call the same core command; immediate `finalizing` projection; transcript/AI work may continue separately |
| Toggle transcript | `⌘⇧T` opens/closes trailing inspector; closing returns focus to the exact prior note/composer element |
| Ask Kestrel | `⌘⏎` submits prompt; composer never loses typed text when provider unavailable or model choice changes |
| Cancel inference | `Esc` when active response has focus and a documented global command; cancellation never clears user prompt automatically |
| Switch model | Picker opens with current choice focused; changing model affects next request only unless user explicitly stops/retries current run |
| Incoming transcript | If user is at live edge, scroll gently; if user scrolled away, show “12 new segments” pill rather than force-scroll |
| Sync warning | Announce once with source and remedy; do not interrupt typing or open a panel automatically |
| VoiceOver | Announce state transitions succinctly: “Kestrel: recording started,” “recording stopped, finalizing,” “Gemini response streaming,” “response stopped” |

Keyboard flow must be complete: `⌘N` starts ad-hoc meeting; `⌘⇧R` starts/stops capture after a contextual safety confirmation where needed; `⌘⇧T` toggles transcript; `⌘L` focuses assistant; `⌘F` searches current transcript/meeting; `⌘K` opens command palette; `⌘,` opens settings. Final shortcut selection must avoid conflicts with existing app and macOS commands during source review.

## 9. Visual-regression matrix

A view that looks correct once is not a correct macOS layout. Add SwiftUI previews plus screenshot/automation coverage for the following matrix before enabling new model providers or panels.

| Scenario | Required pass condition |
|---|---|
| Expanded width with transcript open | Sidebar/canvas/inspector align; notes remain ≥520 pt; composer and Stop visible |
| Standard width with long title and 10 models | No header/composer overlap; title truncates; picker scrolls/searches independently |
| Compact width during active recording | Capture strip and Stop remain visible; sidebar/transcript are overlays, never simultaneous persistent panes |
| Maximum accessibility text | Controls reflow vertically within their region; no clipped/overlapping text; action hit targets preserved |
| Long localized labels | Data-route/model labels truncate or move to overlay; canonical actions retain full accessible names |
| Dark mode/increased contrast/reduce motion | State remains distinguishable by text/icon; waveform respects motion preference |
| Streaming and note typing | New delta does not alter text-selection/focus or cause content jump |
| Cancel race | After core accepts cancel, output never changes; composer remains enabled per intended state |
| Sync failure | Source-named status and remedy appear without covering Stop or composer |
| Back-to-back meeting transition | Buffer & Confirm is modal/attached sheet with explicit Continue/Start New, not a vague toast |

## 10. Design acceptance checklist

The UI redesign passes only when user tasks can be completed without guessing: start/stop capture, type a note while transcript updates, ask and cancel a local/cloud model, inspect which data route applies, recover a named sync issue, and switch between adjacent meetings without accidental data contamination. A “world-class” look is insufficient if any of these actions becomes hidden, delayed, or ambiguous.

## References

[1]: https://developer.apple.com/design/human-interface-guidelines/layout "Apple — Layout, Human Interface Guidelines"
[2]: https://docs.granola.ai/help-center/taking-notes/transcription "Granola — How transcription works"
[3]: https://littlebird.ai/features/meeting-notes "Littlebird — AI Meeting Notes, Bot-Free"
[4]: https://docs.ollama.com/capabilities/streaming "Ollama — Streaming"
[5]: https://ai.google.dev/gemini-api/docs/streaming "Google AI for Developers — Streaming interactions"
