# 08 — Littlebird Parity & Beyond: Phased Requirements

**Status:** Draft for review, 2026-07-31.
**Sources:** Littlebird public documentation (`https://support.littlebird.ai/docs/…` — introduction, context, chat, routines, meeting-notes, meeting-workflow, meeting-notes-import, hummingbird, glance, projects, assistant-notes, command-bar, integrations, gmail, google-calendar, exclusions, privacy-security, usage, mcp, faq — fetched 2026-07-31), plus a full survey of this repo (code on `main`, the `worktree-encapsulated-chasing-candle` worktree, `docs/01`–`07`, and `docs/superpowers/`).

**Purpose:** a build-ready, phased requirements spec that takes Whimbrel to (1) functional parity with Littlebird's shipped feature set, then (2) past it. It extends [02-functional-requirements.md](02-functional-requirements.md) — existing `FR-*` IDs are referenced, not restated; net-new subsystems get new ID series (`FR-MCPS`, `FR-NOTES`, `FR-PRJ`, `FR-GLC`, `FR-OVR`, `FR-CMD`, `FR-WEB`). Extensions to existing series continue their numbering (e.g. `FR-RTN-8`).

---

## 1. What Littlebird ships today (condensed inventory)

From their docs, Littlebird is: Mac + Windows desktop app, iOS/Android companion apps (chat/review only, no phone capture), no web app, built on Anthropic's Claude Agent SDK, cloud-hosted (AWS US East, SOC 2, TLS + at-rest encryption — **not** E2E), credit-based usage with purchasable packs.

| # | Capability | Detail |
|---|-----------|--------|
| L1 | Context awareness | Text/elements of the **active window** every few seconds via accessibility APIs (no screenshots, video, or keystrokes; background windows and secondary monitors ignored). Builds a private encrypted activity index. Password fields ignored; credit cards/API keys auto-redacted; password managers auto-excluded. |
| L2 | Context controls | Pause (5m/15m/30m/1h/until relaunch) from menu bar/tray; per-app + per-domain exclusions; **category exclusions** (Banking, Social, Shopping, Entertainment, Health; Adult always off); delete last hour / last day / full reset. |
| L3 | Chat | Grounded in captured context + integrations. Threads, recents, starring, doc+image attachments, mic dictation, thumbs up/down, "Max mode" (deeper multi-step reasoning), image generation, download-as-markdown. Chat Library of prebuilt prompts ("What's quietly slipping?", "Morning brief", "Prepare me for a hard conversation"). Sharing: private / team / public link (snapshot semantics). |
| L4 | Routines | Scheduled prompts — daily (exact time) / weekly / monthly — from templates or custom. Delivery: dashboard + push + optional email. Output history retained and queryable from Chat. **Auto-pause when outputs are consistently ignored** (credit economics). |
| L5 | Meeting Notes | Calendar-triggered record prompt minutes before events + manual start. System-audio capture, no bot. **Live transcript view during the meeting** + side-by-side manual notes. Structured post-meeting summary: Executive summary / "For you" personalized actions / Topics w/ key quotes / Decisions / Action items (You vs Others) / Risks & open questions. **Every section editable in place.** Shareable link. Import of external meeting notes as first-class searchable records. Pre-meeting briefing workflow (Call Briefing 1-Pager template). iOS in-person transcription. |
| L6 | Hummingbird | macOS overlay on double-tap Option: sees current visible screen (exclusions deliberately don't apply to this on-demand view) + live meeting transcript. Quick tasks: summarize live meeting, draft a reply, explain what's on screen, rephrase text. |
| L7 | Glance | Collapsible home-page dashboard (Cmd+G): Schedule, today's Routines w/ completion, yesterday's meeting summaries, Continue-last-chat, Connect-more shortcuts. Widgets show/hide/reorder, layout persisted. |
| L8 | Projects | Folders grouping chats + meetings. Per-project standing Instructions. Per-project file uploads shared across the project's chats. Context isolated per project by default, with an opt-in toggle to reference outside conversations. |
| L9 | Assistant Notes + Custom Instructions | Custom Instructions: static user-only standing rules. Assistant Notes: AI-editable durable memory ("remember that …" → add/update/remove via natural language) applied proactively to every conversation. |
| L10 | Command Bar | Cmd+K: search chats/meetings/settings and jump; contextual actions (download chat, copy link); "Reset Local Database" (hard refresh of the local cache from cloud). |
| L11 | Integrations | Gmail: read/search/summarize/triage, draft, **send with review**, multi-account. Google Calendar: read (free slots, shared availability) + **write** (create events with agendas/links/deep-work blocks), multi-account. Outlook mail+calendar. Slack "coming soon". |
| L12 | MCP server | `https://mcp.littlebird.ai/mcp` — remote MCP over OAuth exposing the user's Littlebird data/tools to 20+ external AI clients (Claude Code/Desktop, Cursor, ChatGPT, …) with account-level permissions. |
| L13 | Usage UX | Settings → Usage: single real-time usage bar + reset date; non-expiring usage packs; guidance on cheap usage patterns. |
| L14 | Mobile | iOS/Android companions: chat over desktop-collected context, routine delivery, time-sensitive questions. No phone screen capture. |

## 2. Parity matrix

Disposition codes: **HAVE** (shipped, verified) · **PART** (partially built) · **GAP** (missing) · **DIVERGE** (deliberately different — keep the divergence).

| Littlebird capability | Whimbrel status | Disposition |
|---|---|---|
| L1 active-window text capture, no screenshots | `ScreenTextCaptureService` (AX walk) + `OCRFallback` (Vision); FR-CAP-1..3 | **HAVE** — same architectural stance |
| L1 secret auto-redaction, password-manager auto-exclusion | Exclusion lists exist (FR-CAP-4) but no content-level redaction of card numbers/API keys, no password-manager auto-detection | **PART** → FR-CAP-10/11 |
| L2 timed pause options | FR-CAP-8 (P2, "incognito interval") specced, unbuilt | **PART** → build FR-CAP-8 |
| L2 category exclusions, delete-last-hour/day | Retention + per-item delete exist (FR-MEM-4); no category exclusions, no time-window quick delete | **PART** → FR-CAP-12, FR-MEM-7 |
| L3 grounded chat with citations, model choice | Shipped incl. citations, Ollama/Anthropic/OpenAI routing, spend caps, local-only mode | **HAVE** (better: BYO-model, local-only) |
| L3 prompt library + suggested chips | Shipped 2026-08-1x: `PromptLibrary`/`QueryUrgencyClassifier` (M1.4 sub-projects 3 & 4), chips in Ask mode, keyword-based Ollama routing substitution — commits `2f446f6`/`616ed07` | **HAVE** |
| L3 attachments, dictation, starring, feedback, share links, markdown export | None of these | **GAP** → FR-CHAT-6..10 |
| L3 Max mode | Per-conversation model picker exists; no "effort" toggle | **PART** → FR-CHAT-11 |
| L3 image generation | Absent | **GAP** (MAY) → FR-CHAT-12 |
| L4 Routines | `core/routines` is an empty stub | **GAP** → FR-RTN-1..7 + new FR-RTN-8..10 |
| L5 calendar-trigger + manual + heuristic detection | Shipped, three detectors (heuristic detection is **beyond** Littlebird) | **HAVE** (Meet-in-browser + end-detection bugs noted in §3) |
| L5 live in-meeting transcript | Explicitly out of scope today (batch post-meeting only) | **GAP** → FR-MTG-11 (revisit the exclusion) |
| L5 structured summary w/ For-you, quotes, risks; editable sections | Editable sections shipped 2026-08-19 (see `docs/10-meeting-workspace-v0.4.0.md`): `MeetingSummary { tldr, decisions, action_items, open_questions }` now stored in `meeting_note_draft` + `meeting_action_item` tables, with replace/keep/compare regeneration semantics. "For-you" personalization per diarization still unbuilt (relates to speaker attribution, needs a profile/diarization-speaker-identity model). | **PART** → FR-MTG-12/13 (editable sections now HAVE, "For-you" remains GAP) |
| L5 share meeting summary link | None | **GAP** → FR-WEB-4 |
| L5 import external notes | None | **GAP** → FR-MEM-8 |
| L5 pre-meeting briefing | Brief view for `.before` state (attendees, prior meetings, open action items, digest) shipped 2026-08-19; matches FR-RTN-9's intent for meeting-context-aware pre-meeting preps, though FR-RTN-9 is distinct (automated routine execution, not just UI display) | **PART** → FR-RTN-9 (pre-meeting UI now HAVE via meeting-workspace Before state, routine automation still GAP) |
| L6 overlay (Hummingbird) | None | **GAP** → FR-OVR-1..4 |
| L7 Glance dashboard | Menu-bar popover has quick-glance basics | **PART** → FR-GLC-1..3 |
| L8 Projects | None | **GAP** → FR-PRJ-1..4 |
| L9 Assistant Notes / Custom Instructions | None (no persistent user-memory layer) | **GAP** → FR-NOTES-1..4 |
| L10 Command bar | None | **GAP** → FR-CMD-1..3 |
| L11 Gmail / Calendar-write / Outlook integrations | Google Calendar **read-only** `list_events` connector working on the unmerged worktree only; nothing else | **PART/GAP** → FR-MCP-2 subset, phased |
| L12 MCP server exposing own data | **Nothing.** `core/mcp-host` is a *client* host; zero tools served | **GAP — largest single parity item** → FR-MCPS-1..7 |
| L13 usage bar | `usage_log` + spend caps exist; no UI, and cap-hit is indistinguishable from provider failure | **PART** → FR-CHAT-13 |
| L14 mobile companion | README stub (P4) | **GAP** — existing FR-MOB covers it |
| Cloud-hosted, non-E2E, vendor-locked model | Local-first, E2E-encrypted sync, BYO keys/models | **DIVERGE — keep.** This is Whimbrel's moat, not a gap. |

## 3. Phase LB-0 — Foundations (parity prerequisites)

Debt already documented in [05-roadmap.md](05-roadmap.md) that parity features would sit on top of. Building parity surface on these unfixed is building on sand.

| ID | Pri | Requirement |
|----|-----|-------------|
| LB0-1 | MUST | Replace the hashing-trick placeholder embedding in `core/memory` with a real local embedding model (candle or ONNX; e.g. all-MiniLM/bge-small class). Every "find what I saw" parity claim (L1) depends on real semantic recall. Re-index existing rows on upgrade. |
| LB0-2 | MUST | Merge the Google Calendar MCP path from `worktree-encapsulated-chasing-candle` (Connectors settings pane, `CalendarConnectorState`, bundled connector) into `main`. It is the only working connector and the template for all of FR-MCP-2. |
| LB0-3 | MUST | Fix meeting end detection (validated silence threshold + scheduled-end + app-quit paths against real meetings) and Google-Meet-in-browser detection (AX-tree walk independent of capture state). |
| LB0-4 | MUST | Recording-file lifecycle: extend `applyRetention` to delete orphaned `.caf`/WAV files per retention class; one-time cleanup pass for the accumulated multi-GB backlog. (Currently "out of scope" in 05 — parity with a commercial product makes unbounded disk growth untenable.) |
| LB0-5 | MUST | Distinguish spend-cap-exceeded from provider failure at the FFI boundary (typed error, surfaced in chat UI). Prerequisite for the usage bar (FR-CHAT-13). |
| LB0-6 | SHOULD | Backfill `transcript_segment` ingestion for pre-feature meetings so search covers all history. |
| LB0-7 | SHOULD | CI on the three NFR-PRT-3 targets (core tests), so the parity build-out doesn't regress silently. |
| LB0-8 | SHOULD | Diarization quality pass: speaker-count hint for heuristic meetings (open question 8), revisit fallback-to-"Others" rate, session-level time-correlation for bleed filtering. |

**Exit gate:** semantic search returns model-based results; calendar connector usable from `main`; a full meeting auto-ends correctly; disk usage bounded; CI green.

## 4. Phase LB-1 — The automation core (MCP server, routines, memory)

The three pillars Littlebird has that Whimbrel lacks entirely. These are backend-heavy and unlock everything in LB-2/LB-3.

### FR-MCPS — Whimbrel MCP server (net-new subsystem)

Littlebird's L12 is a *hosted* MCP endpoint; a hosted endpoint that can read your data is incompatible with Whimbrel's E2E stance (the server holds only ciphertext). Whimbrel therefore serves MCP **from the device**, which is strictly more private and works offline. Remote access is a later, opt-in bridge.

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-MCPS-1 | P2 | MUST | A local MCP server (new `core/mcp-server` crate, reusing `rmcp`) exposing Whimbrel's own data over stdio and streamable-HTTP-on-localhost, launchable as `whimbrel mcp serve` and advertised in the app's Settings with copy-paste client config for Claude Code/Desktop, Cursor, etc. |
| FR-MCPS-2 | P2 | MUST | Read tools at launch: `search_memory` (hybrid semantic+keyword, time/app/device filters), `list_meetings`, `get_meeting` (summary + metadata), `get_transcript` (paginated segments w/ speakers), `get_timeline` (day view), `get_daily_digest`. |
| FR-MCPS-3 | P2 | MUST | Every tool result respects exclusions, retention, and (FR-MCPS-4) scope grants; excluded-app content never leaves via MCP. |
| FR-MCPS-4 | P2 | MUST | Consent model: first connection from a client requires in-app approval; per-client scopes (memory / meetings / transcripts / write) persisted and revocable in Settings. Local clients authenticate via a per-client token minted at approval. |
| FR-MCPS-5 | P2 | SHOULD | Write tools behind explicit scope: `create_note`, `add_assistant_note` (FR-NOTES), `trigger_routine`. Write calls route through FR-AGT-2 approval gates. |
| FR-MCPS-6 | P3 | SHOULD | Serve the **merged cross-device** picture (post FR-SYNC-7), so any enrolled desktop's MCP server answers for the whole account. |
| FR-MCPS-7 | P3 | MAY | Opt-in remote bridge for mobile/other machines (device-to-device, E2E — e.g. through the existing Worker as a dumb ciphertext relay), never a plaintext cloud endpoint. |

### FR-RTN — Routines (activate the stub)

FR-RTN-1..7 in docs/02 already cover Littlebird's L4 and more (heartbeat/condition triggers, write-action gates). Build them as written. Additions from Littlebird's docs worth copying:

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-RTN-8 | P2 | MUST | Routine output history is stored as timeline items, browsable per routine, and **queryable from chat** ("what did my morning brief say?"). |
| FR-RTN-9 | P2 | MUST | Pre-meeting briefing template: N minutes before a calendar meeting, compile attendees, prior meetings/threads with them, open action items, and deliver as notification + inbox item. (Makes FR-RTN-4's "pre-meeting prep" concrete; Littlebird only offers this as a manual chat template — auto-run beats them.) |
| FR-RTN-10 | P2 | SHOULD | Engagement-aware auto-pause: if a routine's outputs go unopened for K consecutive runs, pause it and say so; one-click resume. (Littlebird's credit-saving trick; for Whimbrel it saves the user's own API spend.) |

### FR-NOTES — Assistant notes & custom instructions (net-new subsystem)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-NOTES-1 | P2 | MUST | Custom Instructions: a user-edited standing-rules text applied to every chat/digest/summary prompt. Synced via settings sync. |
| FR-NOTES-2 | P2 | MUST | Assistant Notes: a structured, durable memory store (new table, synced) the assistant can add/update/remove entries in via natural language ("remember that …"), with every mutation shown to the user and undoable. |
| FR-NOTES-3 | P2 | MUST | Notes are injected into chat/routine/summary prompts as proactive context; a Settings pane lists and edits them directly. |
| FR-NOTES-4 | P2 | SHOULD | Distinguish proactive memory (Notes) from on-demand retrieval (FR-MEM search) in the prompt structure, per Littlebird's own guidance, to keep the prompt budget bounded. |

**Exit gate:** an external Claude Code session can, via Whimbrel's MCP server, answer "what did I discuss with X on Tuesday" from real meeting data; a daily-brief routine fires on schedule and lands in inbox + notification; "remember that I prefer bullet points" persists and shapes the next conversation.

## 5. Phase LB-2 — Product surface parity

### Meetings (extends FR-MTG)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-MTG-11 | P2 | MUST | Live in-meeting transcript view (streaming or near-real-time chunked transcription into the meeting window). Reverses the "batch-only" exclusion in 05-roadmap — it is table stakes in Littlebird, Granola, and every competitor. |
| FR-MTG-12 | P2 | MUST | Upgrade `MeetingSummary` to Littlebird's structure: executive summary; **"For you"** (actions attributed to the user, using diarization's "Me" channel — an attribution Littlebird has to infer, Whimbrel *knows*); topics with key quotes; decisions; action items You/Others; risks & open questions. |
| FR-MTG-13 | P2 | MUST | Summary sections editable in place; edits persist and sync, and are marked user-authored vs generated. |
| FR-MTG-14 | P2 | MUST | Manual meeting start is an explicit "Start Meeting" action, independent of activity-capture state in both directions — starting a meeting never touches activity capture, and pausing/resuming activity capture never touches an in-progress meeting. |
| FR-MEM-8 | P2 | SHOULD | Import external meeting notes (markdown/text/VTT-SRT transcripts to start) as first-class searchable meeting records. |

### Chat (extends FR-CHAT)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-CHAT-6 | P2 | MUST | Multi-thread conversations: named threads, recents list, starring. (Today chat is a single Ask surface.) |
| FR-CHAT-7 | P2 | MUST | Finish M1.4 sub-projects 3 & 4: prompt library (Chat-Library-style templates incl. Littlebird's best — "what's quietly slipping?", "which recurring meetings can I kill?") and suggested-prompt chips. |
| FR-CHAT-8 | P2 | SHOULD | Attachments (files, images) into a conversation's context; mic dictation via on-device speech APIs. |
| FR-CHAT-9 | P2 | SHOULD | Per-answer feedback (thumbs), logged locally for prompt iteration. |
| FR-CHAT-10 | P2 | SHOULD | Export chat as markdown; copy deep link to a chat (in-app link; public share is FR-WEB-4). |
| FR-CHAT-11 | P2 | SHOULD | "Max mode" analog: per-message effort toggle mapping to bigger model / extended thinking in the routing layer. |
| FR-CHAT-12 | P3 | MAY | Image generation in chat via a configurable provider. |
| FR-CHAT-13 | P2 | MUST | Usage pane: real-time spend bar per provider from `usage_log`, cap proximity warnings, and a distinct in-chat "you hit your spend cap" state (needs LB0-5). |

### Capture controls (extends FR-CAP / FR-MEM)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-CAP-10 | P2 | MUST | Content-level auto-redaction before persistence: credit-card numbers (Luhn), API-key/token patterns, SSN-like strings. |
| FR-CAP-11 | P2 | MUST | Auto-detect and exclude password managers by bundle ID (1Password, Bitwarden, LastPass, KeePassXC, …) without user setup. |
| FR-CAP-12 | P2 | SHOULD | Category-based domain exclusions (Banking, Health, Social, Shopping, Entertainment; Adult always on) shipped as maintained lists. |
| FR-CAP-13 | P2 | MUST | Activity capture (continuous screen-text logging) runs independently of meeting state, and starts automatically once Accessibility permission is granted (including if the user enabled it before permission was granted) — no separate manual step required. |
| FR-MEM-7 | P2 | SHOULD | Quick delete of last hour / last day of captures from the menu bar, propagating to indexes and sync (uses FR-MEM-4 machinery). |
| — | P2 | MUST | Build FR-CAP-8 (timed pause: 5m/15m/30m/1h/until relaunch) as already specced. |

### FR-GLC — Glance-style home (net-new)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-GLC-1 | P2 | MUST | A home/dashboard surface: today's schedule (EventKit + connectors), today's routines w/ status, yesterday's meeting summaries, continue-last-chat. |
| FR-GLC-2 | P2 | SHOULD | Widgets show/hide/reorder; layout persisted (settings sync). |
| FR-GLC-3 | P3 | MAY | Proactive alert widgets (Littlebird lists this as *their* roadmap — an overtake opportunity: e.g. "meeting in 10 min, no prep brief read", "action item from Tuesday still open"). |

### FR-OVR — Overlay (Hummingbird analog, net-new)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-OVR-1 | P2 | MUST | Global-hotkey floating panel (default double-tap Option, rebindable) over any app, closable by hotkey/click-away. |
| FR-OVR-2 | P2 | MUST | On-demand context: current active-window text (fresh AX/OCR grab at ask time — bypasses exclusions for this explicit request, matching Littlebird's documented behavior) and the live meeting transcript when one is recording. |
| FR-OVR-3 | P2 | MUST | Quick actions: answer about screen, summarize meeting so far, draft/rephrase text with copy-back. |
| FR-OVR-4 | P2 | SHOULD | Full routing parity (provider picker, local-only mode) — the overlay is a thin client over the same core. |

### FR-PRJ — Projects (net-new)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-PRJ-1 | P2 | MUST | Project containers grouping chats + meetings; create/star; bulk assign; synced. |
| FR-PRJ-2 | P2 | MUST | Per-project instructions injected into that project's conversations. |
| FR-PRJ-3 | P2 | SHOULD | Per-project files available to all chats in the project. |
| FR-PRJ-4 | P2 | SHOULD | Context scoping: project chats retrieve only project content by default, with a per-project toggle to include global memory. |
| FR-PRJ-5 | P3 | MAY | Auto-suggest project assignment from entity extraction (FR-MEM-5) — beyond Littlebird's manual-only assignment. |

### FR-CMD — Command bar (net-new)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-CMD-1 | P2 | MUST | Cmd+K palette: search chats, meetings, timeline items, settings panes; keyboard-first; jump on select. |
| FR-CMD-2 | P2 | SHOULD | Contextual actions per surface (export chat, copy link, start meeting note, pause capture). |
| FR-CMD-3 | P2 | MAY | Maintenance actions (re-index search, sync now) — Whimbrel's analog of "Reset Local Database". |

**Exit gate:** a day's work is drivable Littlebird-style: home dashboard morning check → auto pre-meeting brief → recorded meeting w/ live transcript → editable structured summary → overlay quick-asks during work → Cmd+K to find anything — with capture controls a privacy reviewer would sign off on.

## 6. Phase LB-3 — Integrations parity (extends FR-MCP)

Priority order from Littlebird's catalog, all as MCP connectors per FR-MCP-2..6, write actions gated per FR-MCP-3/FR-AGT-2:

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-MCP-7 | P2 | MUST | Google Calendar **write**: create events with title/time/attendees/rich descriptions (agendas, links). Extends the existing read-only connector. |
| FR-MCP-8 | P2 | MUST | Gmail: search/read/summarize threads, inbox triage, draft replies; **send** only behind explicit per-send approval. Multi-account. |
| FR-MCP-9 | P2 | SHOULD | Outlook 365 mail + calendar (same shape as FR-MCP-7/8). |
| FR-MCP-10 | P3 | SHOULD | Slack read (channels/DMs summaries) — beat Littlebird's "coming soon". |
| FR-MCP-11 | P2 | MUST | Connector health surface in Settings (FR-MCP-5): auth status, last success, re-auth flow — generalizing the worktree's Connectors pane. |

**Exit gate:** the FR-RTN-2 daily brief runs against real calendar + real inbox; "email Sarah the meeting recap" produces a gated, reviewable draft-then-send.

## 7. Phase LB-4 — Web review + mobile companions

### FR-WEB — Web viewer (net-new; supersedes the unbuilt design in docs/07 — do not implement from 07's Fly.io/WebSocket/R2 architecture)

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-WEB-1 | P3 | MUST | Read-only web app (Next.js/Vercel per house stack, or Worker-hosted) for reviewing meetings, summaries, routine outputs, and timeline. E2E constraint: decryption happens **client-side in the browser** (WASM of the sync crypto or a JS port); the account Turso DB and Worker continue to see ciphertext only. |
| FR-WEB-2 | P3 | MUST | Web enrolls as a device (existing X25519 enrollment + approval flow) — sessions are revocable like any device. |
| FR-WEB-3 | P3 | SHOULD | Chat over synced memory from the web, routed via the user's configured providers. |
| FR-WEB-4 | P3 | SHOULD | Share links for a meeting summary or chat: explicit per-item publish of a **snapshot** (re-encrypted to a link key or served plaintext-by-choice), revocable. Matches Littlebird's private/team/public sharing without breaking default E2E. |

### Mobile

FR-MOB-1..4 already specify parity (chat over synced memory, routine push, approvals surface, in-person recording via phone mic, no phone screen capture — identical stance to Littlebird). One addition:

| ID | Phase | Pri | Requirement |
|----|-------|-----|-------------|
| FR-MOB-5 | P4 | SHOULD | Push delivery of routine outputs and meeting-summary-ready notifications (APNs/FCM via the Worker; payloads E2E — notification is a poke, content decrypts on device). |

**Exit gate:** review a meeting summary from a phone browser or companion app minutes after the meeting ends, with the server never able to read it.

## 8. Phase LB-5 — Beyond Littlebird

Where Whimbrel doesn't just match but wins. Ordered by leverage.

| ID | Pri | Requirement |
|----|-----|-------------|
| BL-1 | MUST | **Privacy as the headline**: E2E-encrypted multi-device memory (already built) vs Littlebird's server-readable AWS store; local-only mode; BYO models incl. fully-offline Ollama. Document and market it; add a "what leaves this device" live audit view (extends FR-ONB-3). |
| BL-2 | MUST | **True multi-device capture**: Littlebird's mobile is a window onto one desktop's context. Whimbrel already merges capture from multiple Macs (FR-SYNC-3) and will add Windows (P3) — one consolidated timeline across every machine. Finish FR-SYNC-7 federated search and confirm cross-device meeting sync. |
| BL-3 | MUST | **Agentic desktop automation** (FR-AGT-1..7): guarded computer-use with approval gates, kill switch, audit log, portable skills. Littlebird has nothing here; combined with routines this is "notices AND acts". |
| BL-4 | SHOULD | **Event-triggered routines**: FR-RTN-1's heartbeat enables "when a meeting with X ends, draft the recap", "when I close the laptop, post my EOD" — Littlebird is clock-only. |
| BL-5 | SHOULD | **Open MCP in both directions**: Whimbrel is an MCP *host* (any third-party server, FR-MCP-4) and an MCP *server* (FR-MCPS). Littlebird serves its data but users can't plug arbitrary connectors in. |
| BL-6 | SHOULD | **Cross-meeting speaker identity**: persist speaker embeddings (already computed by `core/diarize`) into a local voiceprint library so recurring participants get stable names across meetings, seeded from calendar attendees. No competitor does this on-device. |
| BL-7 | SHOULD | **Data portability**: documented SQLite schema, full export (markdown + JSON + audio), no lock-in — the credible answer to "what if this app dies". |
| BL-8 | MAY | Proactive nudges (FR-GLC-3) and "resume where I left off" session restoration from the timeline ("you were editing X, waiting on Y, meeting at 2"). |
| BL-9 | MAY | Team features (shared project spaces, meeting-summary handoff) — only after single-user is excellent; Littlebird's team sharing is shallow (links). |

## 9. Explicit divergences from Littlebird (decided, not gaps)

1. **No plaintext cloud.** Littlebird's hosted MCP, server-side routines, and share links all rest on a server that reads user data. Whimbrel keeps E2E: MCP serves from-device (FR-MCPS), routines run on-device, sharing is explicit per-item publish (FR-WEB-4). Consequence to accept: routines don't fire while every desktop is asleep — mitigated later by FR-MCPS-7/relay wakeups, never by giving the server keys.
2. **BYO keys instead of credits.** No usage-pack economy; the usage bar (FR-CHAT-13) tracks the user's own provider spend against their own caps.
3. **Ollama-down ≠ silent failover to hosted** (documented decision in docs/02) — stays.
4. **Heuristic meeting detection stays** — Whimbrel detects unscheduled meetings; Littlebird is calendar+manual only.

## 10. Open questions for the product owner

1. ~~**FR-MTG-11 (live transcription)** reverses a documented out-of-scope decision. Confirm the reversal — it's the single most user-visible parity gap in meetings.~~ **Resolved 2026-07-31: reversal confirmed.** Live transcription is in scope; update the out-of-scope list in 05-roadmap when FR-MTG-11 work begins.
2. ~~**FR-OVR-2** copies Littlebird's "exclusions don't apply to on-demand overlay asks". Comfortable with that, or should overlay respect exclusions (stricter than Littlebird)?~~ **Resolved 2026-07-31: overlay bypasses exclusions for explicit on-demand asks, matching Littlebird.**
3. **Sharing (FR-WEB-4)** inevitably relaxes E2E per shared item. Acceptable as explicit opt-in per item, or defer sharing entirely?
4. **Phase ordering**: LB-1 puts the MCP server and routines before UI parity (LB-2). If day-to-day dogfooding pain matters more than automation, swap them — dependencies allow it except FR-GLC needs FR-RTN for its routine widget.
5. **FR-CHAT-12 (image gen)**: worth carrying at all?
6. The worker allowlist (`ALLOWED_GOOGLE_EMAILS`) makes Whimbrel personal-use today. Multi-account/tenant hardening is deliberately absent from all phases here — confirm.

## 11. Sequencing summary

```
LB-0 foundations ──► LB-1 automation core ──► LB-2 surface parity ──► LB-3 integrations ──► LB-4 web+mobile ──► LB-5 beyond
(embeddings, bugs,    (FR-MCPS, FR-RTN,        (meetings, chat, capture   (calendar-write,      (FR-WEB, FR-MOB)     (BL-1..9; BL-1/2/3
 worktree merge,       FR-NOTES)                controls, glance, overlay,  gmail, outlook,                            can start any time
 file cleanup)                                  projects, cmd bar)          health UI)                                 after LB-1)
```

Within LB-1/LB-2, items are largely independent and suitable for the repo's per-feature SDD flow (`docs/superpowers/specs/` + `plans/` pairs); this document is the backlog they should be cut from. Roadmap phase tags P2/P3/P4 above slot into the existing [05-roadmap.md](05-roadmap.md) structure: LB-0→M1.5/M2.x hardening, LB-1/2/3→Phase 2, LB-4→Phases 3–4.
