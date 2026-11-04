# 5. Roadmap by horizon

This roadmap slots the backlog around doc 17's Releases 1 to 5 and doc 16's five decisions
without reopening either. Doc 17's releases carry the adoption and retention fixes; this
document carries the capability work that turns a memory into knowledge and a routine into
an agent. The two interleave, and one engineer plus coding agents is the assumed capacity,
so each horizon has one spine and a few satellites, not a list of everything desirable.

Horizons: **H1** 0 to 3 months (Nov 2026 to Jan 2027), **H2** 3 to 6 (Feb to Apr 2027),
**H3** 6 to 12 (May to Oct 2027), **H4** 12 plus.

## 5.1 Horizon summary

| Horizon | Spine | Satellites | Doc 17 work alongside |
|---|---|---|---|
| H1 | Surface Connected Topics and wire the entity schema (O-02, O-01 phase 1, O-10) | Eval harness and spans (O-03); trust surface (O-14); legible proactivity (O-13); Limitless import (O-21) | Release 1 remainder, Release 2 (Windows parity), the cheap-first on-ramp follow-ups |
| H2 | Condition-triggered routines and the governed tool loop (O-05, O-04, O-33) | Commitments ledger (O-08); memory diff on return (O-29); local extraction tier (O-11); graph leg in retrieval (O-06); plain-language routine authoring (O-18); embedding upgrade (O-12) | Release 3 (mobile approvals and push) |
| H3 | Cloud routine runner (O-15) and speaker identity (O-07) | Decision log (O-09); pre-mortem brief (O-28); relationship health (O-26); Meeting Ask over the graph (O-24); user-registered MCP servers (O-17); MCP write tools (O-16); Outlook (O-23) | Release 4 (Cloud) |
| H4 | Team spaces (O-22) | Contradiction alerts (O-27); counterfactual recall (O-32); signed routine files (O-30); device-local A/B (O-34); local fine-tune research (O-31) | Release 5 (Team) |

## 5.2 Feature records

Each roadmap feature carries the same eighteen fields.

### F1. Connected Topics, surfaced (O-02) · H1

1. **Problem**: 5,000 lines of entity and topic code produce nothing a user can see.
2. **Users**: everyone with more than two weeks of capture.
3. **User value**: a people and projects view, Obsidian export, and the first visible answer
   to "what does Whimbrel know about X."
4. **Evidence**: F (document 1 §1.7); doc 17 lists it as shipped, which it is not.
5. **Builds on**: `memory/src/topics/*`, `run_jobs_batch`, Mac settings sync, Windows
   schema generator.
6. **New surfaces**: FFI entries (`whimbrel_topics_*`), Tauri commands, a Topics view on Mac
   and Windows, a job-type allowlist change.
7. **Platforms**: Mac, Windows; iOS read-only later.
8. **Model needs**: topic naming only (already a `TaskKind`); clustering is local math.
9. **Privacy**: no new data leaves the device; export is user-initiated.
10. **Dependencies**: none.
11. **Effort**: 3 to 4 weeks.
12. **Risk**: naming quality on small corpora; mitigated by the eval harness and a
    "needs more data" state.
13. **Success metric**: 40% of active users open the Topics view in week 1 of release; a
    named topic is renamed or merged by 20% (they care enough to correct it).
14. **Pricing tier**: Free.
15. **Owner area**: core memory, Mac, Windows.
16. **Decision needed**: whether the topics tables sync (today they do not).
17. **Kill criterion**: under 10% open rate after two releases, hide behind a setting.
18. **Status**: not started; design in `docs/superpowers/specs` needed.

### F2. Entity and relationship memory (O-01) · H1 schema, H2 extraction, H3 views

1. **Problem**: memory is rows; there is no structure for who, what project, what was
   decided, what was promised, or when any of it stopped being true.
2. **Users**: everyone; most valuable to managers and anyone with many threads.
3. **User value**: every later feature in H2 and H3 (commitments, memory diff, decision
   log, relationship health) is a view over this.
4. **Evidence**: M (document 3 §3.2 bi-temporal model; §3.1 entity-passage links), F
   (existing `entity`, `entity_mention`, structured meeting summaries).
5. **Builds on**: `topics/entities.rs`, `meeting_action_item`, `meeting.summary_*`,
   `items.actors`, the topics indexer.
6. **New surfaces**: `relation` table (typed edges, `valid_from`, `valid_to`,
   `recorded_at`, source refs), `fact` table, an extraction job, graph read model.
7. **Platforms**: core first; views in F6, F9.
8. **Model needs**: entity candidates from heuristics and a GLiNER-class encoder (F10);
   relation typing with grammar-constrained JSON on the light chain.
9. **Privacy**: facts about third parties are derived from the user's own data and never
   leave the device except through E2E sync; a per-entity "forget" is mandatory.
10. **Dependencies**: F1 (the jobs run), F3 (the eval set judges extraction quality).
11. **Effort**: 6 to 8 weeks across H1 and H2.
12. **Risk**: wrong facts with confident provenance; mitigated by the citation contract
    (every fact carries its source), extraction-per-source, and user correction.
13. **Success metric**: precision of extracted commitments and decisions above 0.8 on the
    seeded corpus; under 5% of surfaced facts corrected or deleted by users.
14. **Pricing tier**: Free.
15. **Owner area**: core memory, core api.
16. **Decision needed**: sync the graph rows (yes, same E2E path; row types added).
17. **Kill criterion**: extraction precision under 0.6 with the local tier and under 0.8
    with a frontier key after two iterations; then ship entity pages without relations.
18. **Status**: not started; technical proposal in document 6 §6.1.

### F3. Evaluation harness and local spans (O-03) · H1

1. **Problem**: no way to tell whether a retrieval, prompt or model change helped.
2. **Users**: the team; indirectly every user.
3. **User value**: fewer regressions; a visible "answer quality" number in release notes.
4. **Evidence**: M (document 3 §3.1, §3.9).
5. **Builds on**: `usage_log` (trace id), `assemble_context`, the citation checker in
   `meeting_prep.rs`, existing test fixtures.
6. **New surfaces**: `core/eval` crate with a seeded corpus, question sets, graders; a
   `parent_trace_id` and span kind on `usage_log`; a CI job.
7. **Platforms**: CI and developer machines.
8. **Model needs**: a grader model through a team key; local graders for citation checks.
9. **Privacy**: the seeded corpus is synthetic; no user data.
10. **Dependencies**: none.
11. **Effort**: 2 to 3 weeks initial; ongoing.
12. **Risk**: a synthetic corpus that does not resemble real use; mitigated by an opt-in
    "contribute anonymized questions" later, never by default.
13. **Success metric**: every change to `routing.rs`, `assemble_context`, prompts or the
    embedding model runs the suite; a baseline number exists by the end of H1.
14. **Pricing tier**: n/a.
15. **Owner area**: core llm, core api.
16. **Decision needed**: which grader model and who pays (team key, a few dollars a run).
17. **Kill criterion**: none; this is infrastructure.
18. **Status**: not started; proposal in document 6 §6.5.

### F4. Trust surface (O-14) · H1

1. **Problem**: the privacy architecture is real and mostly invisible (document 3 §3.8).
2. **Users**: prospective users deciding whether to run an ambient capture tool.
3. **User value**: a capture indicator, per-app and per-site exclusions on both desktops,
   export everything, delete everything, and one page showing what the relay sees.
4. **Evidence**: M (Recall's history; Littlebird's facts page as the competitor's answer).
5. **Builds on**: the sync design's metadata list, existing settings panes, the export
   path for Obsidian.
6. **New surfaces**: an indicator, an exclusions list, a "your data" pane, a website page.
7. **Platforms**: Mac, Windows, website.
8. **Model needs**: none.
9. **Privacy**: this is the privacy feature.
10. **Dependencies**: none.
11. **Effort**: 2 to 3 weeks.
12. **Risk**: low.
13. **Success metric**: the website page is the second most visited after the home page
    among sign-ups (a proxy for "privacy drove the decision").
14. **Pricing tier**: Free.
15. **Owner area**: Mac, Windows, website.
16. **Decision needed**: whether exclusions sync (recommended: yes, as settings).
17. **Kill criterion**: none.
18. **Status**: parts exist (exclusions on Mac, unverified on Windows).

### F5. Legible proactivity controls (O-13) · H1

1. **Problem**: proactive output without explanation reads as surveillance (document 3
   §3.8); Pulse died on it.
2. **Users**: everyone who sees Today.
3. **User value**: every item on Today says why it is there and from which source; an
   intensity setting; per-source opt-in; "offer, don't act" as the default posture.
4. **Evidence**: M (arXiv 2509.09309; CHI 2026 study).
5. **Builds on**: Today on three platforms, citations, routine settings.
6. **New surfaces**: a "why" affordance on Today cards, a proactivity level setting.
7. **Platforms**: Mac, Windows, iOS.
8. **Model needs**: none beyond existing.
9. **Privacy**: reduces perceived threat.
10. **Dependencies**: none.
11. **Effort**: 1 to 2 weeks.
12. **Risk**: low.
13. **Success metric**: Today dismissal rate falls; "why" is opened by 15% of viewers.
14. **Pricing tier**: Free.
15. **Owner area**: shells.
16. **Decision needed**: none.
17. **Kill criterion**: none.
18. **Status**: not started.

### F6. Where-I-left-off per entity (O-10) · H1 to H2

1. **Problem**: "what is the state of things with X" needs a question today; it should be
   a page.
2. **Users**: everyone.
3. **User value**: a person or project page: last touch, open loops, promised and owed,
   next event, recent decisions, with citations.
4. **Evidence**: F (the topics read model has "where I left off"); doc 17 R-gaps.
5. **Builds on**: `topics/read.rs`, threads status, calendar items, F2 facts.
6. **New surfaces**: entity pages on Mac and Windows; search that lands on them.
7. **Platforms**: Mac, Windows; iOS read-only in H3.
8. **Model needs**: none for the page; a one-paragraph summary uses the light chain.
9. **Privacy**: local.
10. **Dependencies**: F1; richer with F2.
11. **Effort**: 3 weeks.
12. **Risk**: low.
13. **Success metric**: entity pages account for 25% of in-app navigation within a month.
14. **Pricing tier**: Free.
15. **Owner area**: shells, core api.
16. **Decision needed**: none.
17. **Kill criterion**: none.
18. **Status**: not started.

### F7. Condition-triggered routines (O-05) · H2

1. **Problem**: routines run on the clock only (FR-RTN-1 unmet).
2. **Users**: anyone whose work is event-driven, which is everyone.
3. **User value**: "after any meeting with my manager, draft the follow-up"; "when an email
   from a customer domain arrives, prep a reply brief"; "when a commitment is due tomorrow,
   remind me."
4. **Evidence**: F (the tick, the templates); M (scheduled agents are table stakes;
   conditions are not).
5. **Builds on**: `routines/`, the tick in `routine_runs.rs`, items ingestion, meeting
   lifecycle events, F2 commitments.
6. **New surfaces**: a `trigger` field on `routine` (schedule or condition), a condition
   evaluator in the tick, trigger pickers in both shells and in sync fields.
7. **Platforms**: core, Mac, Windows; iOS shows runs.
8. **Model needs**: none for triggers; the routine's own model preference for output.
9. **Privacy**: local.
10. **Dependencies**: F2 for commitment triggers; none for meeting and email triggers.
11. **Effort**: 3 to 4 weeks.
12. **Risk**: runaway triggers; mitigated by a per-routine rate limit and the spend cap.
13. **Success metric**: 30% of routine runs come from conditions within two months.
14. **Pricing tier**: Free locally; Cloud runs them when the laptop is closed (F11).
15. **Owner area**: core routines, shells.
16. **Decision needed**: the trigger vocabulary (document 6 §6.3 proposes one).
17. **Kill criterion**: none.
18. **Status**: not started.

### F8. Governed tool-use loop and injection firewall (O-04, O-33) · H2

1. **Problem**: "agent" means one gated draft; there is no loop, no audit log, no kill
   switch (FR-AGT-2, 3).
2. **Users**: routine authors; later, Cloud.
3. **User value**: routines that can look things up, call connector tools and propose
   multi-step actions, each step tiered by reversibility.
4. **Evidence**: F; M (document 3 §3.4 on approval fatigue and pass^k).
5. **Builds on**: `try_chain`, the OpenAI, Anthropic and Google clients, `McpHost`,
   `write_action::plan/settle`, the seven `mcp-server` read tools as the initial tool set.
6. **New surfaces**: tool schemas in `core/llm`, `agent_run` and `agent_step` tables, a
   reversibility tier per tool, an audit log view, a global kill switch, an injection
   classifier on ingested text, an approval inbox that shows a diff-style preview.
7. **Platforms**: core, Mac, Windows; iOS approvals in Release 3.
8. **Model needs**: tool calling; the light chain for read-only steps, the preferred model
   for planning. Local 4B tool calling is uneven (document 3 §3.6), so Local-only mode gets
   a reduced tool set.
9. **Privacy**: tool calls logged locally; nothing new leaves the device.
10. **Dependencies**: F3 (pass^k suite), F7 (triggers make it useful).
11. **Effort**: 6 weeks.
12. **Risk**: the highest on the roadmap; mitigated by read-only default, caps on steps and
    spend, the firewall, and never auto-approving an irreversible action.
13. **Success metric**: pass^4 above 0.7 on the routine suite; zero unapproved irreversible
    actions, ever.
14. **Pricing tier**: Free.
15. **Owner area**: core llm, core api, core agent.
16. **Decision needed**: the reversibility tiers (document 6 §6.2).
17. **Kill criterion**: pass^4 under 0.5 after two iterations; then keep single-step
    actions only.
18. **Status**: not started.

### F9. Commitments ledger and memory diff (O-08, O-29) · H2

1. **Problem**: promises made in meetings and email are the most valuable thing captured
   and the least visible.
2. **Users**: everyone.
3. **User value**: "you said you would", "they said they would", due dates, nudges, closure
   when later evidence shows it was done; and on return from time away, a diff of what
   changed, generated from graph deltas rather than a prompt over everything.
4. **Evidence**: F (action items exist per meeting); I.
5. **Builds on**: F2 facts with `recorded_at`, `meeting_action_item`, Today, routines.
6. **New surfaces**: a Commitments view, "since you were away" on Today, a weekly
   routine template.
7. **Platforms**: Mac, Windows, iOS read.
8. **Model needs**: extraction (F2); closure detection on the light chain.
9. **Privacy**: local.
10. **Dependencies**: F2.
11. **Effort**: 4 weeks.
12. **Risk**: nagging; mitigated by F5 controls.
13. **Success metric**: week-2 return rate up 10 points for users with the view enabled.
14. **Pricing tier**: Free.
15. **Owner area**: core api, shells.
16. **Decision needed**: none.
17. **Kill criterion**: dismissal over 60% of nudges.
18. **Status**: not started.

### F10. Local-model extraction tier and embedding upgrade (O-11, O-12) · H2

1. **Problem**: every memory product assumes a frontier model for extraction; Whimbrel's
   promise is that memory works without one.
2. **Users**: Local-only and cheap-first users.
3. **User value**: entity and relation extraction that runs on the machine.
4. **Evidence**: M (GLiNER2, LightKGG, EmbeddingGemma).
5. **Builds on**: candle in `embedding.rs`, the bundled `llama-server`, the light chain.
6. **New surfaces**: an encoder model download, grammar-constrained prompts, a versioned
   embedding rebuild.
7. **Platforms**: core.
8. **Model needs**: a 200M encoder, a 4B instruct model, a 300M to 600M embedding model.
9. **Privacy**: strengthens it.
10. **Dependencies**: F3 to prove it helps.
11. **Effort**: 4 weeks.
12. **Risk**: disk and battery cost; the rebuild of vec tables on upgrade.
13. **Success metric**: extraction precision within 10 points of the frontier path on the
    seeded corpus; retrieval recall at 10 up measurably on the eval set.
14. **Pricing tier**: Free.
15. **Owner area**: core memory, core llm.
16. **Decision needed**: models and licences.
17. **Kill criterion**: no measurable gain on the eval set; keep the current models.
18. **Status**: not started.

### F11. Cloud routine runner (O-15) · H3 · doc 17 Release 4

Doc 17 §5.2 is the record. Added here: the runner executes F7 triggers and F8 loops with
the same approval tiers, and the per-routine grants decide which tools it may call. No
other change.

### F12. Speaker identity across meetings (O-07) · H3

1. **Problem**: diarization labels reset every meeting; "who said that" is unanswerable
   across time.
2. **Users**: anyone in recurring meetings.
3. **User value**: names on transcript lines, "what has Sam said about X across meetings",
   asker names on live alerts.
4. **Evidence**: F (sherpa-onnx diarization); M (every competitor is weak here).
5. **Builds on**: `core/diarize`, meeting attendees, F2 person entities.
6. **New surfaces**: a speaker-embedding store, an enrollment and confirmation flow, a
   per-person consent note.
7. **Platforms**: Mac first, Windows second.
8. **Model needs**: a speaker-embedding model (local).
9. **Privacy**: voiceprints of other people are sensitive; stored locally, encrypted,
   user-confirmed, deletable, never synced to the relay in clear, and off by default.
10. **Dependencies**: F2.
11. **Effort**: 6 weeks.
12. **Risk**: legal posture in two-party-consent jurisdictions; needs a written policy.
13. **Success metric**: 80% of recurring-meeting lines attributed after three meetings.
14. **Pricing tier**: Free.
15. **Owner area**: core diarize, Mac.
16. **Decision needed**: the consent and default posture.
17. **Kill criterion**: accuracy under 70% on a test set.
18. **Status**: not started.

### F13. Decision log and pre-mortem brief (O-09, O-28) · H3

1. **Problem**: decisions are in summaries, not in a structure with provenance.
2. **Users**: managers, founders, anyone who revisits decisions.
3. **User value**: a decision entity (what, when, who, inputs, sources), an optional review
   date, and a pre-meeting brief that lists what was said and promised last time, each
   claim cited (the Meeting Prep citation contract extended to the graph).
4. **Evidence**: M (document 3 §3.7); F (`meeting.summary_decisions`, `meeting_prep.rs`).
5. **Builds on**: F2, `meeting_prep.rs`, routines.
6. **New surfaces**: a Decisions view, a review reminder, an export.
7. **Platforms**: Mac, Windows, iOS read.
8. **Model needs**: light chain.
9. **Privacy**: local.
10. **Dependencies**: F2, F7.
11. **Effort**: 4 weeks.
12. **Risk**: overclaiming "better decisions"; the copy claims recall and provenance only.
13. **Success metric**: 30% of users with 10 or more decisions open the view weekly.
14. **Pricing tier**: Free.
15. **Owner area**: core api, shells.
16. **Decision needed**: whether to ship calibration scoring (recommended: later).
17. **Kill criterion**: none.
18. **Status**: not started.

### F14. Relationship health (O-26) · H3

Same fields as F9 in shape; it is a query over F2 plus thread latency. Default off,
per F5. Not detailed further until F9 ships and its dismissal rate is known.

### F15. External MCP: user servers and write tools with elicitation (O-17, O-16) · H3

1. **Problem**: routines can only use bundled connectors; external agents can only read.
2. **Users**: developers and power users; Cloud routines.
3. **User value**: add any MCP server to `mcp-host`; expose `create_event`, `create_draft`
   and `remember_fact` to external clients behind the same approval queue, with elicitation
   (spec 2025-06-18) for "ask before a write."
4. **Evidence**: F; M (document 3 §3.5).
5. **Builds on**: `mcp-host`, `mcp-server`, scoped tokens, F8.
6. **New surfaces**: a servers list, tool allowlists per routine, write tool handlers.
7. **Platforms**: core, Mac, Windows.
8. **Model needs**: F8.
9. **Privacy**: every call logged; tokens scoped; writes gated.
10. **Dependencies**: F8.
11. **Effort**: 4 weeks.
12. **Risk**: arbitrary servers are arbitrary code; run them as the user does today with a
    clear warning, no sandbox promise.
13. **Success metric**: 10% of routine authors add a server.
14. **Pricing tier**: Free.
15. **Owner area**: core mcp.
16. **Decision needed**: none.
17. **Kill criterion**: none.
18. **Status**: not started.

### F16. Team spaces (O-22) · H4 · doc 17 Release 5

Doc 17's design holds. Added here: the shared object is a subgraph (project entity and its
edges), not a folder of notes, which is what makes it different from Granola's folders.

### F17. H4 research items (O-27, O-30, O-31, O-32, O-34)

Not scheduled. Each is a two-week spike after F2 and F8 have shipped, decided one at a
time on the eval numbers and on user requests.

## 5.3 Dependency graph

```
F3 eval ──────────────┐
F1 topics ─► F2 graph ─┼─► F6 entity pages ─► F9 commitments, memory diff ─► F13 decisions ─► F14 relationships
                       │                                                     └─► F16 team (subgraph sharing)
F10 local tier ────────┘
F5 controls ─► (every proactive surface)
F7 triggers ─► F8 tool loop (+ O-33 firewall) ─► F11 cloud runner ─► F15 external MCP
F12 speakers ─► F13 (who decided), live alert asker names
F4 trust surface (independent; ships first)
```

## 5.4 What this roadmap does not change

- Doc 16 decisions 1 to 5, including no managed inference of any kind.
- Doc 17's release order, pricing ($12 to $15 Cloud; Team per seat; Support), and the
  Google verification and relay cap work.
- The open-source licence and the "local core is free" line, which document 3 §3.10 shows
  is also the market's line.
