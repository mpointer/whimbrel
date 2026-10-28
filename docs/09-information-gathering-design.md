# 09 — Information Gathering & Context Distillation: Design Guidance

**Status:** Draft, 2026-08-06. Reconciled against [04-architecture.md](04-architecture.md) and [08-littlebird-parity-spec.md](08-littlebird-parity-spec.md).
**Scope:** How Whimbrel turns raw capture + connector data into on-target, usable context for routines (FR-RTN) and live chat (FR-CHAT) — the *memory pipeline* behind the feature backlog in 08. This doc does not restate 08's requirements; it proposes the architecture several of them (FR-RTN-2/9, FR-MCP-8, FR-MEM-5, FR-NOTES-4, LB0-1) will sit on.
**Sources:** Littlebird public docs (via 08's fetch), [TechCrunch on Littlebird](https://techcrunch.com/2026/03/23/littlebird-raises-11m-to-capture-context-from-your-computer-so-you-can-query-your-data/), [Screenpipe architecture docs](https://docs.screenpi.pe/architecture), [Screenpipe vs Littlebird](https://screenpipe.com/compare/littlebird).

---

## 1. Research findings (what the comparables actually do)

### Littlebird

- **Text-only, active-window, accessibility-first.** No screenshots, video, or keystrokes; frontmost window text every few seconds via accessibility APIs. Founder framing: text-only storage is *the* efficiency bet — Rewind and MS Recall both sank under screenshot storage weight. **Whimbrel already made the same bet** (04 §platform decision 3, `capture_item` schema) — validated, not a gap.
- **Cloud-processed** (AWS, encrypted at rest, *not* E2E) explicitly so they can run big models. Whimbrel's E2E/local-first divergence (08 §9.1) means Whimbrel must get equivalent distillation from **local batch inference** — which is exactly what §5 below designs.
- **Connectors fill what the screen misses.** Gmail/Calendar/Outlook via API; screen remains the primary sensor. Their triage/summarize/draft features imply background jobs over connector data, not just call-time tool use (relevant to §4's design tension).
- **Distillation is productized, not raw retrieval:** Routines (scheduled prompts over context), "prep for meeting" (calendar-spine join: attendees ↔ past meetings ↔ threads ↔ external web), pre-generated chat prompts that personalize over time (implies background profile-building). 08 already captures these as FR-RTN-8/9, FR-NOTES; this doc supplies the substrate they query.

### Screenpipe (source-available; the auditable reference implementation)

- **Event-driven capture, not fixed-interval polling:** triggers on app switch, window focus, click/scroll, debounced typing pause, clipboard copy; idle fallback ~5s. Accessibility tree primary, OCR fallback — same stance as Whimbrel's `ScreenTextCaptureService`. Worth comparing trigger sets: if Whimbrel's AX observer is interval-based, moving to event-driven triggers cuts redundant captures substantially (their published numbers: ~600 MB RAM, 5–10% CPU, 5–10 GB/month *including JPEG frames Whimbrel doesn't store* — text-only Whimbrel should be far under that).
- **Frame/text dedup before storage** (Whimbrel: simhash in `core/ingest` — already aligned).
- **Single SQLite + FTS5, local REST + MCP API, and "pipes"** — cron-scheduled markdown-prompt agents querying the API. Structurally identical to FR-RTN + FR-MCPS. Their architecture validates Whimbrel's; no change needed.

**Net:** Whimbrel's capture layer is already at or beyond the comparables architecturally. The actual gaps — confirmed by 08's parity matrix — are downstream: real embeddings (LB0-1), connector ingestion depth, and the distillation layers between raw capture and consumable context.

---

## 2. The core problem: one flat pile of chunks

Today's data model (04) is essentially `capture_item` + `transcript_segment` + a `summary` table keyed by meeting/day/week. Both routines and chat must query raw items and re-derive meaning per request. That produces noisy retrieval, repeated inference cost, and briefs that miss slow-moving state (open loops, commitments, waiting-on-them threads) that no single day's items reveal.

**Proposed: four explicit layers, each built from the one below by progressively more expensive inference.**

```
L0 RAW EVENTS          L1 ITEMS                L2 EPISODES/THREADS        L3 KNOWLEDGE
capture stream,        dedup'd typed chunks    session & thread           entities, projects,
connector deltas       + FTS5 + sqlite-vec     rolling summaries,         commitments, follow-ups,
(pruned in days)       (capture_item et al.,   structured status/asks     profile facts
                        kept ~forever)         (new tables)               (extends FR-MEM-5, FR-NOTES)
no inference           heuristics + local      local Ollama batch         Ollama batch + routed
                       embeddings (LB0-1)      (idle compute)             frontier calls (sparingly)
```

- **Live chat** reads L1 (hybrid retrieval) + L2/L3 (pre-distilled) — never L0.
- **Routines** read mostly L2/L3 and *cite* into L1. The daily brief becomes largely a **query** over structured rows, not an LLM crawl over yesterday's raw text.
- Every L2/L3 row carries provenance (source L1 IDs) so chat citations (FR-CHAT shipped behavior) drill down cleanly.
- Placement: L2/L3 tables live in `core/memory`'s SQLite alongside existing tables; builders run in a new job-queue worker (§5) in the Rust core. Sync classification (which layers sync vs stay device-local) is a decision point — see §7.

## 3. L2: episodes and threads

Two distinct L2 objects, because screen time and communication have different units of meaning:

**Episodes** — contiguous same-context activity windows derived from `capture_item` ("14:00–14:40, Xcode + docs, working on sync enrollment"). Built in batch: segment by app/window/URL continuity gaps, summarize with a local model into 2–3 sentences + extracted refs. These are what "what did I do yesterday" and FR-GLC/daily-brief consume, and they make timeline queries cheap.

**Threads** — the unit of meaning for mail/Slack/Teams is the thread, not the message. Per thread:

- **Rolling summary, incrementally updated:** on new messages, feed (old summary + new messages) to a local model → new summary. Never re-summarize long threads from scratch — this is what makes slow local models viable.
- **Structured fields, not just prose:** `status` (waiting_on_me | waiting_on_them | resolved | fyi), `asks_of_me[]`, `commitments_by_me[]` (with due-ish dates), `priority`, `last_activity`.
- **Two-stage priority:** (1) cheap heuristics at ingest — direct-to-me vs CC, mentions, sender rank learned from reply history, calendar proximity, thread velocity; (2) local-LLM classification only for items passing the heuristic bar. Never run a model over every email.
- **Follow-up detection as a rule, not inference:** last message is mine + contains request + no reply in N days ⇒ `waiting_on_them` candidate; inverse for `waiting_on_me`. These rows *are* the "what's quietly slipping?" answer (08's Chat Library item) and the backbone of FR-RTN-2's brief.

**Calendar is the join spine:** events link attendees ↔ threads ↔ transcript_segments ↔ episodes in the surrounding window. FR-RTN-9's pre-meeting brief is this join + summarization + optional web search — mostly SQL, one model call at the end.

## 4. Connector ingestion — a decision 08 doesn't force

04 declares **"MCP as the only integration mechanism."** As a *transport/auth* decision that's right and should stand. But there are two consumption models, and Whimbrel needs both:

1. **Call-time tools** (current model): chat/routine invokes `search_threads` etc. live. Right for actions (draft, send, create event) and rare lookups. Wrong for the brief/follow-up/priority workload — every routine run re-reads the same inbox, pays full latency/cost, and can't see cross-time patterns (thread velocity, aging follow-ups).
2. **Ingested + indexed** (Littlebird's model, proposed here): a sync loop pulls **incrementally with persisted cursors** (Gmail `historyId`, MS Graph delta tokens, Slack `oldest` ts) through the same MCP connectors, normalizes into L1, and lets L2 builders distill. Follow-up aging and sender-rank learning are only possible here.

Proposal: extend the FR-MCP connector shape with an optional **`sync` capability** (cursor in, items out) alongside tools. Connectors stay MCP servers — 04's decision holds — but gain a pull-based ingestion contract. Normalize everything to one L1 item shape (source-prefixed stable IDs; `kind`: email|chat_msg|event|screen|transcript; thread_id; actors; cleaned `body_text` with quoted history/signatures/join-leave noise stripped — cleaning alone roughly doubles downstream signal density).

Priority order matches 08 §6: Gmail + Calendar first (FR-MCP-7/8), Outlook (FR-MCP-9), Slack/Teams later (FR-MCP-10). Note for Teams: Graph delta requires tenant permissions that a corporate tenant may deny — if so, Teams context arrives via screen capture only, which the L1 schema already accommodates (`kind: screen` items carry app + window context).

## 5. Inference tiering — batch work on local Ollama

Whimbrel's E2E stance forbids Littlebird's answer (ship text to cloud, run big models). The replacement is deliberate tiering, aligned with 04's LLM strategy ("always local: embeddings; routable: synthesis; batched summarization") but made concrete:

| Tier | Work | When | Model |
|---|---|---|---|
| T0 heuristics | dedup, cleaning, flags, priority pre-score, follow-up rules | inline at ingest | none |
| T1 local batch | embeddings (LB0-1), episode summaries, thread rolling summaries, priority classification, entity extraction (FR-MEM-5) | idle compute, batched | Ollama: embed model + 7–14B instruct |
| T2 routed | daily-brief final synthesis, meeting prep, user-facing chat | on demand / scheduled | via `core/llm` router (frontier by default, local in sensitive/offline mode — user policy per 04) |

Queue mechanics (a `jobs` table + worker in the Rust core — no external queue infra):

- **Priority + deadline:** thread summaries for today's-meeting attendees outrank generic backfill. Jobs carry `not_before`, `priority`, `attempts`.
- **Batch by job type** so one Ollama model stays loaded — model-swap thrash destroys throughput on a single machine. Drain all embeddings, then all summaries.
- **Idempotent, content-hash-keyed jobs:** re-running is always safe. A slow local model falling behind just means L2 lags — acceptable by design; T2 consumers degrade gracefully to L1 retrieval.
- **Schema-constrained outputs:** T1 jobs emit JSON against a schema (Ollama structured outputs); validate, re-queue on failure. 7–14B models are reliable for constrained extraction/summarization; open-ended synthesis stays in T2.
- Nightly window: heavier backfill (re-embedding on model change — record model+version per vector row; weekly digests; entity-graph maintenance).
- This also gives FR-RTN-10's engagement-aware pause a natural lever: paused routine ⇒ its upstream T1 jobs drop priority.

## 6. `assemble_context` — one retrieval API for both consumers

One function in `core/memory`/`core/llm` boundary that chat, routines, and the overlay (FR-OVR-4) all call:

```
assemble_context(query|task, time_range, budget_tokens, sources[]) -> ContextPack (with provenance)
```

- **Hybrid retrieval:** FTS5 + sqlite-vec over L1, merged via reciprocal rank fusion (skip a reranker until proven necessary), plus **direct structured pulls** from L2/L3 (open follow-ups, today's events, relevant thread summaries).
- **Coarse-to-fine budget packing:** L3 facts → L2 summaries → L1 raw excerpts until the token budget is hit.
- **Recency-weighted scoring:** `relevance × exp(-age/τ)`, τ per kind (screen: hours–days; threads: weeks; L3 facts: no decay).
- This is also where FR-NOTES-4's proactive-vs-retrieved distinction lives: Assistant Notes/Custom Instructions are injected as a fixed proactive block; everything else competes for the retrieval budget.
- Routines then reduce to: trigger → `assemble_context` → prompt → delivery (FR-RTN-1's model, unchanged). Live chat: same call, user message as query, MCP tools available for gaps.

## 7. Sequencing and decision points

Sequencing — slots into 08's phases rather than replacing them:

1. **LB0-1 first** (real embeddings) — everything here assumes it. Choose the embedding model once (small local class per 04; via Ollama or candle/ONNX), version-stamp vectors.
2. **Job queue + T1 worker** (with LB0-1's re-index as its first customer) — LB-0/LB-1 boundary.
3. **Episodes + `assemble_context`** — immediately improves shipped chat; prerequisite polish for FR-RTN-2/8.
4. **Connector `sync` capability + thread distillation** — lands with LB-3 (FR-MCP-7/8), and is what makes FR-RTN-2's brief and "quietly slipping" queries real rather than per-run inbox crawls.
5. **L3 entity/profile layer** — extends FR-MEM-5 + FR-NOTES; last, because L2 provides most routine value.

Decisions needed from the product owner:

1. **Connector ingestion vs call-time-only** (§4) — this doc recommends ingestion via a `sync` capability on MCP connectors; it's a real amendment to 04's "MCP as the only integration mechanism" *consumption* assumption. Storage cost is trivial (text), but it widens the at-rest data footprint — privacy posture (exclusions, retention classes per FR-MEM) must extend to connector items.
2. **Sync classification for L2/L3:** thread summaries and follow-up state are exactly what mobile (FR-MOB) and web (FR-WEB) want; syncing them multiplies value of the distillation spend. Recommend: L2/L3 sync, L0/L1 screen items stay device-local (federated search via FR-SYNC-7/FR-MCPS-6 covers cross-device L1).
3. **Idle-compute policy:** T1 on a laptop competes with the user and battery. Options: run only on AC power + user-idle, fixed nightly window, or thermal/load-aware throttle. Recommend AC + idle-detection to start — simplest honest policy.
4. **Whether episodes are worth it before connectors:** if dogfooding shows chat-over-capture is already good enough, step 3 can swap with step 4. Follow-ups/briefs (connector-side) are the higher-leverage distillation; episodes mainly cheapen timeline/summary queries.

---

*Caveat: Littlebird is closed-source; internals are inferred from their public docs (per 08's fetch), founder statements, and third-party analysis. Screenpipe claims come from its published architecture documentation. Where this doc and 08 conflict, 08's parity facts (fetched from Littlebird's own support docs) win over the press/third-party sourcing used here.*
