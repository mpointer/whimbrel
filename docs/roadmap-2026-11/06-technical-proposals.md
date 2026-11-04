# 6. Technical proposals for the top features

Seven proposals, each at the level of "a builder could write the spec from this." Every
proposal names the files it touches and the design docs it must respect
(`docs/adr/*`, the sync design, the live-alerts and cheap-first specs). Nothing here
changes the Worker, the relay's view of content, or the five decisions in doc 16.

## 6.1 Entity and relationship memory (F2)

**Goal.** Turn `entity` and `entity_mention` into a small, bi-temporal personal graph that
every later feature queries, built by a local-first extraction pipeline, with every fact
carrying its source.

**Schema** (new tables in `memory/src/topics/schema.rs`, migrated like the rest):

```
relation   (id, subject_entity_id, predicate TEXT, object_entity_id NULL, object_text NULL,
            valid_from_unix_ms NULL, valid_to_unix_ms NULL, recorded_at_unix_ms,
            confidence REAL, extractor TEXT, source_kind TEXT, source_id, source_span TEXT,
            user_state TEXT DEFAULT 'auto')      -- auto | confirmed | rejected
commitment (id, relation_id, direction TEXT,    -- i_owe | they_owe
            due_unix_ms NULL, status TEXT,       -- open | done | dropped
            closed_by_source_kind NULL, closed_by_source_id NULL)
decision   (id, relation_id, decided_at_unix_ms, review_at_unix_ms NULL, outcome_text NULL)
```

Predicates start as a closed set: `works_at`, `reports_to`, `works_on`, `owns`,
`promised`, `was_promised`, `decided`, `attended`, `discussed`, `due_on`, `changed_to`.
A closed set is what makes grammar-constrained extraction on a 4B model viable (document 3
§3.3) and what makes the graph leg of retrieval a fixed query instead of a search.

`entity` gains `kind` values `project` and `decision_topic` next to the existing `person`,
`org` and `phrase`. Existing rows are untouched.

**Extraction pipeline** (a new job type `relation_extract`, enqueued by the topics indexer
per source, so re-extraction is local to one source):

1. Candidates without a model: email headers and attendees give `attended` and
   `discussed`; `meeting_action_item` rows give `promised` with the assignee from the
   item text when it names one; `meeting.summary_decisions` gives `decided`.
2. A GLiNER-class encoder (candle, 200M, downloaded like the embedding model) proposes
   entity spans and candidate relation pairs over transcript and email text (F10). Until
   it ships, step 1 alone is the extractor; the schema does not change.
3. Relation typing: one call on the light chain (`TaskKind::RelationType`, new) with a JSON
   schema listing the closed predicate set, the candidate pair, the sentence and the
   source date; the output includes `valid_from` when the text states one. Local-only
   mode uses the bundled `llama-server` with a GBNF grammar.
4. Entity resolution: blocking on `name_key`, email address and attendee lists first;
   embedding similarity on display names second; graph co-occurrence third. No model
   call. Ambiguity leaves two entities and a `maybe_same` relation the UI can merge (the
   existing topic merge code is the pattern).
5. Contradiction handling, Graphiti style: a new `works_at` for the same subject sets
   `valid_to` on the old one at the new fact's `valid_from`; nothing is deleted.

**Read model** (`memory/src/topics/read.rs` and `api/src/topics.rs`): `entity_page(id)`
returns facts grouped by predicate with validity, open commitments, last touch, next event,
and the citations for each. `facts_changed_since(ts)` returns the delta for F9's memory
diff. Both are plain SQL.

**Sync.** Three new row types in the content sync (relation, commitment, decision) travel
in the same E2E path; the relay learns only the row type names, which the sync design
already accepts. Entity rows sync today only if topics sync; this proposal makes `entity`
sync too, because a fact without its entity is useless on another device.

**Privacy and correction.** Every surfaced fact shows its source; a per-entity "forget this
person" deletes entity, relations and mentions and adds the key to a local suppression
list; `user_state` lets a rejected fact never return from re-extraction.

**Eval.** F3's seeded corpus gets 100 labelled relations across the predicate set;
precision and recall per predicate per extractor tier are the gate (document 5 F2 field 13).

**Effort.** Schema and heuristic extraction, 2 weeks; read model and sync, 2 weeks; model
typing and resolution, 2 weeks; UI (F6), 3 weeks.

## 6.2 Governed tool-use loop (F8) with reversibility tiers

**Goal.** Let a routine run a bounded plan, execute, verify loop over a declared tool set,
with each step tiered by reversibility, every step logged, a global kill switch, and no
irreversible action without a person.

**Tool layer** (`core/llm`): a `Tool { name, description, input_schema, tier }` type and a
`ToolCall`/`ToolResult` turn pair added to the chat request for the OpenAI, Anthropic,
Google and OpenRouter clients (all four support tool calling natively; Ollama and the local
server support it unevenly, so Local-only mode gets `tier <= ReadOnly` tools only and a
grammar-forced JSON fallback). `try_chain` is unchanged.

**Tiers**:

| Tier | Examples | Policy |
|---|---|---|
| ReadOnly | the seven `mcp-server` tools, `get_thread`, `list_events`, `entity_page` | auto-run, logged |
| Reversible | `create_draft`, `remember_fact`, create a calendar hold the user owns | auto-run when the routine has a standing authorization for that tool, else queued |
| Irreversible | `send_email`, `create_event` with attendees, anything that leaves the device | always queued for a person; never auto-approved, whatever the routine says |
| Forbidden | delete, payment, credentials | not exposed to the model at all |

This replaces the single `auto_approve_write_actions` bit on `routine` with a per-tool
grant list, which is also what doc 17's Cloud runner needs for its per-routine grants.

**Run state** (`core/memory`): `agent_run (id, routine_run_id, status, step_budget,
spend_budget_cents, started, ended, killed_by)` and `agent_step (id, run_id, idx, kind,
tool, input_json, output_json, tier, approval_state, trace_id)`. `pending_action_json` on
`routine_run` stays for compatibility and is written from the queued step.

**Loop** (`core/api/src/agent_loop.rs`, new): plan (one model turn producing a short step
list), execute (each tool call through `McpHost` or an internal function), verify (one
model turn that checks the result against the routine's stated goal and may add one
corrective step), stop on budget, on a queued irreversible step, or on verify saying done.
Hard caps: 12 steps, the routine's spend budget, 5 minutes wall clock. MAST's third
failure class (no verification) is why the verify turn is mandatory, not optional.

**Kill switch.** A settings flag that sets every `agent_run` to `killed` and refuses new
runs; synced as a settings field so one device can stop all. FR-AGT-2 as written.

**Audit log.** `agent_step` is the log; a view in both shells lists runs, steps, tools,
inputs and outputs, and lets the user revoke a tool grant from the row. The ledger gets
`parent_trace_id` so cost per run is one query.

**Injection firewall (O-33).** Screen text, email bodies and transcript lines enter prompts
as data. Before any tool-using run, the text that will be in context is scored by a local
classifier (a small fine-tuned encoder; until it exists, a rule set for instruction-shaped
spans such as "ignore previous", "send", "forward to", URLs with credentials) and flagged
spans are wrapped and labelled as untrusted in the prompt, and any tool call whose
arguments contain a flagged span is downgraded to Irreversible (queued). The rate of
flagged spans is reported in the audit view. Document 3 §3.4 gives the baseline: 23.6%
injection success without mitigations in a browser agent.

**Eval.** A pass^k suite in `core/eval`: 20 routine goals over the seeded corpus, each run
4 times; pass^4 above 0.7 is the ship gate.

**Effort.** 6 weeks.

## 6.3 Condition-triggered routines (F7)

**Goal.** Routines fire on events, not only on the clock.

**Trigger vocabulary** (a JSON value stored in a new `trigger_json` column on `routine`;
`schedule_hhmm` stays for the clock case):

```
{ "kind": "time", "hhmm": "08:00" }
{ "kind": "meeting_ended", "with_entity": <id> | null, "title_matches": <regex> | null }
{ "kind": "meeting_starting", "lead_minutes": 30, ... }
{ "kind": "email_received", "from_domain": "...", "thread_entity": <id> | null }
{ "kind": "commitment_due", "within_hours": 24, "direction": "i_owe" | "they_owe" | "any" }
{ "kind": "fact_changed", "predicate": "...", "entity": <id> | null }
{ "kind": "idle", "minutes": 30 }      -- "when I stop for a while"
```

**Evaluator.** The tick in `api/src/routine_runs.rs` already runs on a timer on the primary
device; it gains an event cursor (`kv_meta` key `routine_event_cursor`) over `items.local_seq`
and the meeting lifecycle table, and evaluates each enabled routine's trigger against the
events since the cursor. Meeting end, email ingest and commitment due are all already
observable from the store; `fact_changed` needs F2. Each routine gets a rate limit (default
one run per 10 minutes, 20 a day) and a dedupe key (trigger kind plus source id) so the
same meeting cannot fire a routine twice across devices after sync.

**Shells.** The routine editor gains a trigger picker; the sync fields gain `trigger_json`.
Templates get sensible defaults (Meeting Prep already uses a lead window; it becomes a
`meeting_starting` trigger with no behaviour change).

**Effort.** 3 to 4 weeks.

## 6.4 Surfacing Connected Topics (F1)

**Goal.** Make the dormant stack visible with the least new code.

1. Remove `topic_index` and `topic_cluster` from the exclusion in `run_jobs_batch` and
   enqueue `topic_index` from the ingest path (the indexer's `topic_dirty` table already
   tracks what needs work). Run `topic_cluster` nightly from the tick.
2. FFI: expose `api::topics` (list, detail, rename, merge, split, name_dirty) and
   `api::topics_export` through `core/ffi` as JSON string functions, like every other
   entry; add the header declarations.
3. Tauri: the Windows DTO generator (`scratchpad/schemagen` pattern, now in the repo's
   `schema.rs` command) adds the topic types; commands wrap the same API.
4. UI: one Topics view per desktop with the list, a detail pane showing members and the
   "where I left off" line, rename and merge, and an Export to Obsidian button.
5. Sync: topics and entities join the content sync as row types (needed by F2 anyway).

**Effort.** 3 to 4 weeks, most of it the two UIs.

## 6.5 Evaluation harness and local spans (F3)

**Goal.** A number for answer quality and for agent reliability, run on every relevant
change.

- `core/eval` crate (dev-dependency only): a seeded corpus generator that writes a
  synthetic but realistic month (40 meetings with transcripts, 300 emails in threads, 2,000
  captures, a calendar) into a temp `Store`; question sets as JSON: 60 single-hop, 25
  multi-hop and date-scoped, 15 "not in memory" (the right answer is "I don't know"); a
  LongMemEval-style set of 30 questions whose answer changed over the month; the pass^k
  routine suite from §6.2.
- Graders: citation validity is checked locally with the `meeting_prep.rs` contract
  (every claim cites a given source); answer correctness is graded by a model through a
  team key, with the rubric and the grader model version pinned in the result file.
- Spans: `usage_log` gains `parent_trace_id` and `span_kind` (`ask`, `routine`,
  `agent_run`, `agent_step`, `extract`, `embed`), mirroring the OTel GenAI agent span
  names so an exporter later is a mapping, not a redesign. Local by default; nothing is
  sent anywhere.
- CI: a job that runs the retrieval sets with the local embedding model and no network,
  and posts recall at 10, citation precision and the "I don't know" rate; the graded sets
  run on demand.

**Effort.** 2 to 3 weeks, then ongoing.

## 6.6 Speaker identity across meetings (F12)

**Goal.** Names on transcript lines across meetings, locally, with consent.

- `core/diarize` already segments speakers per meeting with sherpa-onnx; add a speaker
  embedding model (sherpa-onnx ships several) and store one embedding per diarized cluster
  per meeting in `speaker_cluster (meeting_id, cluster, embedding, line_count)`.
- Identity is proposed, never assumed: after a meeting, clusters are matched against
  `speaker_identity (entity_id, embedding, confirmed_at)` by cosine distance; a match above
  threshold is shown as "probably Sam" and becomes a name only when the user confirms. The
  first enrollment of a person comes from attendee lists: a two-person meeting with one
  known attendee is the easy case.
- The user's own voice is enrolled from the mic channel with no confirmation needed.
- Storage: embeddings are encrypted at rest with the sync group key and sync as a row type
  only if the user turns on "share speaker identities across my devices"; default off.
- Consent: a settings note states what is stored; a per-person delete removes identity and
  cluster embeddings; nothing is inferred about anyone who is not in the user's attendee
  lists. A written policy for two-party-consent jurisdictions precedes release.
- Live alerts use it for the asker name, which the live-alerts spec left open.

**Effort.** 6 weeks, Mac first.

## 6.7 Local extraction tier and embedding upgrade (F10)

- Encoder: GLiNER2.5 Small (or the GLiNER2 base) run through candle or ONNX, downloaded
  on first use like the multilingual embedding model; it proposes spans and relation
  candidates for §6.1 step 2. If the candle port is costly, ONNX Runtime through
  `ort` is the fallback.
- Typing model: Qwen3.5-4B-Instruct or Gemma 4 E4B on the bundled `llama-server` in
  non-reasoning mode with a GBNF grammar from the predicate schema; `TaskKind::RelationType`
  routes to it when `use_local_model` or Local-only is set, else to the light chain.
- Embeddings: EmbeddingGemma 308M or Qwen3-Embedding-0.6B at 256 dims (Matryoshka);
  `EMBEDDING_MODEL_VERSION` bump triggers the existing vec rebuild; the rebuild runs as a
  low-priority job with progress in settings, and search falls back to FTS until it ends.
  The 4,096 KNN cap is raised by partitioning the vec table by month, which also addresses
  the `ponytail:` note in the code.
- Gate: F3 numbers. If recall at 10 does not move, the embedding upgrade is dropped and the
  encoder alone ships.

**Effort.** 4 weeks.

## 6.8 Cross-cutting rules for all of the above

- Every model call goes through `build_chain_for` or `build_chain_for_routine`, records to
  `usage_log`, and respects the spend caps. No new call paths.
- Every new row type is listed in the sync design's metadata table before it ships.
- Every new user-facing string passes the humanizer rule and is localized through the
  catalog flow.
- No proposal here touches `server/` or `server-updates/`.
