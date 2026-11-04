# 1. Current-state architecture and feature inventory

Verified against the tree at `ba24e93b` (0.9.51, 2026-11-03), not against the README or
the older design docs. File references are to `core/` unless another root is named. Line
numbers are approximate and drift with every merge; the file and function names are the
stable part. Anything not confirmed in source is marked **unverified**.

## 1.1 Shape of the system

```
apps/macos (SwiftUI, 38k lines)   apps/windows (Tauri: Rust shell + React UI)   apps/mobile/ios (SwiftUI)
          |  C ABI (JSON strings)              |  direct Rust calls                    |  C ABI, --no-default-features
          +-------------------- core/ffi -------+----------------------------------------+
                                          core/api  (orchestration: ask, summary, digest, routine tick,
                                                     connectors, OAuth, write actions, import, local model)
   core/llm (providers, chain, RRF context, streaming, ledger, TypeSafe)   core/routines   core/agent (types only)
   core/memory (SQLite: FTS5 + sqlite-vec, episodes, threads, topics, jobs, outbox)
   core/transcribe (whisper.cpp)   core/diarize (sherpa-onnx)   core/ingest (dedupe, chunking)
   core/mcp-server (read tools over localhost)   core/mcp-host (runs bundled connectors)
   core/sync (X25519 + ChaCha20-Poly1305)   core/sync-engine (Turso push/pull, LWW settings)
   mcp-servers/ (Node: gmail, google-calendar, outlook365)
   server/ (Cloudflare Worker: auth, device registry, one Turso DB per account)   server-updates/ (update feed)
```

Rust: Rust 1.97 workspace of 14 crates, about 87k lines, 1,895 `#[test]` functions across the
core (memory 600, api 452, llm 233, ffi 190, sync-engine 124, skins 121, transcribe 55,
mcp-server 34, routines 33, sync 23, ingest 12, diarize 10, mcp-host 5, agent 3). Shell
tests: Mac 1,201 XCTests, Windows 668 Rust plus 415 vitest, iOS 126, relay 202.

## 1.2 Data model (`memory/src/lib.rs`, `memory/src/topics/schema.rs`)

| Table | Holds | Read by |
|---|---|---|
| `items` | Every captured or ingested row: kind in {capture, snapshot, email, calendar_event, transcript}; actors, refs, `thread_id`, `external_id`, `local_seq`. FTS5 (`items_fts`) and vec0 (`items_vec`, 384 dims) | Ask, timeline, topic indexing, sync, MCP |
| `capture_item` | Legacy screen-text table with its own FTS and vec tables | Reads now go through `items` |
| `meeting` and friends | Title, attendees, trigger (calendar, detected, manual, away), status, summary columns (`summary_tldr`, decisions, action items, open questions), away outcome; `transcript_segment` (FTS5 + vec), `meeting_snapshot` (slide image + OCR), `meeting_action_item`, `meeting_note_draft`, `meeting_followup`, `meeting_part`, `live_transcript_segment`, `meeting_prep` | Meetings UI, Ask, prep, routines |
| `episodes` | L2: same-app runs of captures (gap over 10 minutes starts a new one; runs under 60 s merge as alt-tab noise), with `summary` and `refs` | Ask (`recent_episodes(10)`), timeline, topic indexing |
| `threads`, `thread_priority` | Email threads with an incremental summary and rule-based status; TypeSafe scores | Ask (`threads_needing_attention(10)`), Reply Priorities |
| `conversation`, `conversation_exchange` | Ask history with `citations_json` and the model used | Ask UI (device-local; not synced) |
| `topic_source`, `entity`, `entity_mention`, `topic`, `topic_member`, `topic_stats`, `topic_idea`, `topic_dirty` | Connected Topics: entities (person, org, phrase), the bipartite source to entity mention index, clusters with cached centroids | **Nothing outside tests** (see 1.7) |
| `routine`, `routine_run` | Seven templates with schedule, `auto_approve_write_actions`, `model_pref`; runs with status, output and `pending_action_json` | Routine tick, approvals inbox, Today, sync |
| `usage_log`, `spend_cap_event` | One row per model call (provider, model, route, tokens, cents, trace id); cap trips | Usage screen, spend caps |
| `profile_suggestion` | Daily proposed profile facts; the profile itself is a settings string | Profile pane, prompts |
| `jobs` | Background queue (episode and thread summaries, thread priority, reindex) | `run_jobs_batch` |
| sync tables | `sync_outbox`, cursors, row maps, pending dependencies | sync-engine |
| `mcp_client_token`, `connector_account`, `kv_meta` | Scoped MCP tokens (salted hash), connected accounts, markers | MCP server, connectors |

Embeddings: candle running `all-MiniLM-L6-v2` (384 dims, CPU) in `memory/src/embedding.rs`;
a multilingual model is downloaded at first run per the 2026-10-30 localization design. Vec
tables rebuild when `EMBEDDING_MODEL_VERSION` changes. KNN top-k is capped at 4,096, and a
code comment notes hits can be missed once a corpus exceeds that.

## 1.3 Retrieval (`llm/src/lib.rs`, `memory/src/lib.rs`)

- Keyword search is FTS5 (`search_keyword`); semantic search is vec0 KNN
  (`search_semantic`); both cover captures, transcripts (`*_with_meeting_ts`) and slide
  snapshots.
- `assemble_context` fuses three lists with reciprocal rank fusion (k=60): keyword hits,
  semantic hits, routine runs. Each hit is then down-weighted by age with a per-kind decay
  (captures 7 days, transcripts 3, episodes 3, threads 14). Episodes and threads are added as
  a second layer ranked by recency only.
- Context budget is about 1,500 tokens at 4 characters per token; direct hits pack first and
  overflow is dropped. There is no re-ranker; the query is deliberately unused in L2
  filtering ("reserved").
- Citations: `ContextItemRef` tagged by kind (Capture, Transcript, RoutineRun, Episode,
  Thread, Snapshot). Date-scoped questions use `detect_date_scope` and a time-window gather
  (`api/src/ask.rs`). Meeting Ask has its own transcript-only context.
- What Ask cannot see: emails only through the ten most urgent thread summaries; calendar
  items, topics and entities not at all.
- Entity extraction exists (`memory/src/topics/entities.rs`): people and organizations from
  email headers and attendees, phrases from titles and OCR, linked to sources through
  `entity_mention`. **There is no knowledge graph**: no entity-to-entity relations, no typed
  edges, no ontology, no decision or commitment entities, no traversal. A grep for graph,
  ontology and relation finds nothing relevant.

## 1.4 Model layer (`llm/`, `api/src/routing.rs`)

- Providers: Ollama, Anthropic, OpenAI, Google, OpenRouter, and Local (a bundled
  `llama-server` helper started on demand, `api/src/local_model.rs`).
- `build_chain` orders the providers the user holds keys for, preferred first; `try_chain`
  walks it on retryable failures. `TaskKind` (Chat, MeetingSummary, Digest, MeetingPrep,
  Routine, ConversationTitle, TopicName, ProfileSuggest) picks the light chain for small
  tasks when `cheap_first` is on; routines carry their own `model_pref`.
- Streaming adapters for Ollama, Google, Anthropic and OpenAI with cancellation; batch jobs
  and routines still use blocking calls.
- Spend: per-identity daily cap (default 200 cents) and an app-wide breaker (5,000 cents);
  a usage screen reads `usage_log`.
- TypeSafe (`llm/src/typesafe.rs`) scores emails for Reply Priorities on five judgments and
  checks meeting titles against summaries; opt-in, never in Local-only mode.
- Prompts: six fixed localized starters plus a keyword urgency heuristic that sends
  non-urgent chat to the local model.
- Skins: a validated JSON theme manifest, made and refined by chat, synced across devices.

## 1.5 Automation and "agents" (`agent/`, `routines/`, `api/src/routine_runs.rs`)

What "agent" means in the tree today: a draft of one write action, either create a calendar
event or send an email, parked in a `routine_run` row as `pending_action_json` until a person
approves it (or a per-routine standing authorization does). `write_action::plan` claims the
run and builds the connector call, the shell makes the MCP call, `settle` records the
outcome. The agent crate's own header says it is not the guardrail system (no kill switch,
no audit log).

There is **no planning loop, no tool-use loop, no LLM function calling** anywhere in the
tree (no `tools` or `tool_use` in any request), and no agent state beyond `routine_run`,
`meeting_followup`, `meeting_prep` and `kv_meta` markers.

Routine templates and what each does:

| Template | Trigger | Does |
|---|---|---|
| Daily Brief | time of day | yesterday's digest, open loops, today's events |
| Focus Time | time of day | a fixed calendar-event draft, no model call |
| EOD Recap, Weekly Digest | time of day | a digest, plus an optional email draft |
| Reply Priorities | time of day | ranks waiting threads by TypeSafe score |
| Meeting Prep | lead window before a meeting | a brief whose every claim must cite a given source (`llm/src/meeting_prep.rs`) |
| Away Follow-up | Away Mode could not attend | drafts a "please send the recording" email |

One primary device runs routines; a secondary runs Daily Brief or Reply Priorities only when
they are three hours overdue. Away Mode (Mac) joins a Zoom meeting muted and records it. Live
question alerts (2026-11-02) judge each transcript line for a direct question or assignment
to the user and draft an answer from memory with citations, delivered as a synced alert row
and an email to the user's own address.

## 1.6 Connectors, MCP, sync

- Connectors are bundled Node MCP servers (`mcp-servers/`): Gmail (`send_email`,
  `search_threads`, `get_thread`, `create_draft`, `sync_messages`), Google Calendar
  (`list_events`, `create_event`, `sync_events`), Outlook 365 (read-only mail and events; Mac
  only). OAuth is authorization code plus PKCE over a loopback redirect; tokens live in the
  Keychain or Credential Manager and reach connector processes through environment
  variables. Only two write tools exist, both approval-gated.
- `mcp-server` exposes seven read tools (`search_memory`, `list_meetings`, `get_meeting`,
  `get_transcript`, `get_timeline`, `get_daily_digest`, `assemble_context`) over streamable
  HTTP on localhost, each gated by a scoped bearer token. No write tools.
- `mcp-host` runs only Whimbrel's own bundled connectors. There is no user-registered
  external server list, and because there is no tool-use loop the model cannot call any MCP
  tool.
- Sync: X25519 device keys wrap a shared ChaCha20-Poly1305 group key; enrollment is
  approved from an existing device with comparison codes. The Worker authenticates with a
  Google ID token, keeps a device registry and a waitlist, creates one Turso database per
  account and issues per-device tokens. Content never passes through the Worker. Settings
  sync is field-level last-writer-wins (opt-in API keys included); content sync carries
  items, meetings, routine runs, meeting prep, profile suggestions, live alerts and skins.
  Conversations and note edits do not sync. The relay sees device ids, row types,
  timestamps, entity tags and ciphertext sizes, and settings field names; never content.

## 1.7 Dormant, partial, and missing

- **Connected Topics is built and unwired.** About 5,000 lines in `memory/src/topics/`
  (indexing, entity extraction, clustering with no model calls, rename/merge/split, model
  naming, Obsidian Markdown export) and their API layer (`api/src/topics.rs`, whose header
  calls it "the read surface the shells call", and `api/src/topics_export.rs`) have no FFI
  entry, no Tauri command, no UI on any platform, and `run_jobs_batch` explicitly excludes
  `topic_index` and `topic_cluster`; nothing enqueues them. Doc 17 lists Connected Topics as shipped; the tree says it is
  complete in core and invisible to users. This is the single largest piece of leverage in
  the repository and the correction this document makes to doc 17.
- Outlook: Windows deferred; Mac `read_mail` not ingested into `items` (unverified).
- iOS: pull-only replica with Ask and citations; no write approvals (FR-MOB-2), no push, no
  recording.
- Windows: no Away Mode, no live question alerts, no title check, live transcription runs
  but is not displayed; the Reply Priorities capability flag is hard-coded false although
  a Gmail poller exists.
- Telemetry: close to none, not none. The update feed records weekly unique desktops with a
  random per-install, per-week id plus platform and version (`server-updates/src/index.ts`).
  No crash reporting.
- Requirements not implemented: FR-AGT-1 to 7 (computer use, kill switch, audit log, skill
  format, library), FR-RTN-1's condition-based heartbeat (only time-of-day schedules),
  FR-MEM-5's entity views (the data exists, the view does not), FR-SYNC-7 federated search
  (partly met by replication), FR-MOB-2 and 3.
- Known shortcuts marked `ponytail:` in code: the 4,096 KNN cap, peer-synced transcripts not
  topic-indexed, an outbox stale-row leak, episode sampling not token-aware.

## 1.8 Architectural leverage map

| Potential capability | What exists to build on | What is missing |
|---|---|---|
| Entity and relationship memory (people, projects, decisions, commitments) | `entity`, `entity_mention`, `topic_*` tables; extraction in `topics/entities.rs`; meeting decisions and action items already structured in `meeting.summary_*` and `meeting_action_item`; attendees and email actors on `items` | Edge tables with types and validity intervals; decision and commitment entities; extraction prompts; traversal in `topics/read.rs`; FFI and Tauri exposure; sync of the new rows |
| Graph-aware retrieval | `assemble_context` RRF pipeline with a documented slot for a third leg; citations by kind | A graph leg (entity match, one or two hops) and a re-rank stage |
| Multi-step agents with persistent state | governed chain and ledger; `routine_run` approval state machine; `write_action::plan/settle`; `McpHost` as tool runtime; the `jobs` queue; `mcp-server` read tools as a ready tool set | A tool-use loop in `llm` (tool schemas, message turns); `agent_run`/`agent_step` tables; a wider `WriteActionPayload`; condition triggers in the tick; approval views on three shells |
| Decision support | citation-validated briefs (`meeting_prep.rs`); TypeSafe structured judgments; topic status and "where I left off" read model; date-scope machinery | A decision entity with provenance; outcome tracking; a re-rank stage |
| Proactive surfaces | Today on three platforms; live alerts as synced rows; routines with a primary device | Push (doc 15 M4.3); event triggers; an engagement policy |
| Cloud runner | sync-engine pulls an account's rows into any peer; `tick` is pure Rust over `Store` | The runner device, per-routine grants, the WASM or container build (doc 17 §5.2) |

The pattern across every row: the data and the governed call path exist; the structure that
turns rows into knowledge, and the loop that turns a call into an agent, do not.
