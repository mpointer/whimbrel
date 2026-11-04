# 4. Opportunity backlog (scored)

Thirty-four opportunities, each tied to a gap in document 1 (architecture) or document 2
(market), with the evidence class marked. Ten are flagged **unconventional**: concepts no
competitor in document 2 offers and no requirement in `docs/02` names.

Doc 17's Releases 1 to 5 and doc 16's five decisions are taken as given. Where an item
here overlaps a doc 17 release, the column says so and this backlog only re-scores it
against the new capability work; it does not reopen the release.

## 4.1 Scoring model

Each criterion is scored 1 (weak) to 5 (strong). Weights sum to 100.

| Criterion | Weight | What 5 means |
|---|---|---|
| User value | 20 | Solves a problem users state (doc 17 A and R gaps) or document 2's unmet needs |
| Differentiation | 15 | No competitor in document 2 does it, and most cannot because of their architecture |
| Retention | 15 | Raises the chance a user returns in week 2 and keeps the capture running |
| Leverage | 15 | Mostly wiring of code that exists in `core/` today |
| Strategic fit | 10 | Advances doc 16's model (local core free, Cloud, Team, Support) and doc 17's releases |
| Monetization | 10 | Creates or strengthens a paid line without breaking "cheap by default" |
| Defensibility | 10 | Compounds with data, architecture or community in a way a well-funded entrant cannot copy in a quarter |
| Risk (inverted) | 5 | 5 means low privacy, reliability and reputational risk |

Weighted score = sum(score × weight) / 100, so the range is 1.0 to 5.0. Evidence class:
**F** verified fact in the tree, **M** market evidence (document 2 or 3), **I** inference,
**H** hypothesis to test, **S** speculation.

## 4.2 The backlog, ranked

| # | Opportunity | Gap | Val | Diff | Ret | Lev | Fit | Mon | Def | Risk | Score | Evidence |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| O-01 | **Entity and relationship memory** (people, projects, decisions, commitments as first-class, with bi-temporal edges), wired from Connected Topics | 1.7, 2.6 item 4 | 5 | 5 | 4 | 4 | 5 | 3 | 5 | 4 | **4.45** | F, M, I |
| O-02 | **Surface Connected Topics** (FFI, Tauri, Topics view, Obsidian export, jobs enabled) | 1.7 | 4 | 4 | 4 | 5 | 4 | 2 | 3 | 4 | **3.85** | F |
| O-03 | **Evaluation harness and local spans** (seeded corpus, 50 to 100 questions, LongMemEval-style multi-session set, pass^k routine suite, OTel-shaped spans in the ledger) | 3.9 | 3 | 2 | 3 | 4 | 5 | 2 | 4 | 5 | **3.30** | M, I |
| O-04 | **Governed tool-use loop** in `core/llm` and `core/api` (tool schemas, agent runs and steps, reversibility-tiered approvals, audit log, kill switch) | 1.5, FR-AGT | 4 | 4 | 4 | 3 | 5 | 4 | 4 | 3 | **3.90** | F, M |
| O-05 | **Condition-triggered routines** (FR-RTN-1 heartbeat: "when an email from X arrives", "after any meeting with Y", "when a commitment is due") | 1.5 | 5 | 4 | 5 | 3 | 5 | 3 | 3 | 4 | **4.10** | F, M |
| O-06 | **Graph leg in retrieval** (entity match plus one hop as a third RRF list; HippoRAG-style PageRank later) | 1.3, 3.1 | 4 | 4 | 3 | 3 | 4 | 2 | 4 | 4 | **3.50** | M, H |
| O-07 | **Speaker identity across meetings** (voice embeddings enrolled per known person, local only, user-confirmed) | 2.6 item 5 | 5 | 5 | 4 | 2 | 4 | 3 | 5 | 3 | **4.00** | F, M |
| O-08 | **Commitments ledger** ("you said you would", "they said they would", due dates, nudges, closure from later evidence) | 1.8, 3.7 | 5 | 4 | 5 | 3 | 5 | 3 | 4 | 4 | **4.20** | F, I |
| O-09 | **Decision log with provenance** (what was decided, when, by whom, with the sources; optional review date and calibration) | 3.7 | 4 | 5 | 3 | 3 | 4 | 3 | 4 | 4 | **3.75** | M, H |
| O-10 | **Where-I-left-off per entity** (person and project pages: last touch, open loops, next event; the topics read model made visible) | 1.7, FR-MEM-5 | 5 | 3 | 5 | 4 | 4 | 2 | 3 | 5 | **3.95** | F |
| O-11 | **Local-model extraction tier** (GLiNER-class encoder plus 4B grammar-constrained typing; frontier model through the user's key optional) | 3.3 | 3 | 5 | 3 | 2 | 5 | 2 | 5 | 4 | **3.50** | M, H |
| O-12 | **Embedding upgrade** (EmbeddingGemma or Qwen3-Embedding-0.6B, Matryoshka 256 dims, versioned rebuild) | 3.6 | 3 | 1 | 3 | 4 | 3 | 1 | 2 | 4 | **2.60** | M |
| O-13 | **Proactivity controls that are legible** (per-source opt-in, "why this surfaced" on every Today item, intensity slider, offer-not-act default) | 3.8 | 4 | 3 | 4 | 4 | 4 | 1 | 2 | 5 | **3.40** | M |
| O-14 | **Trust surface** (capture indicator, per-app exclusions on both desktops, export and delete everything, what-the-relay-sees page) | 3.8, 2.6 item 7 | 4 | 4 | 3 | 4 | 5 | 2 | 3 | 5 | **3.70** | M |
| O-15 | **Cloud routine runner** (doc 17 §5.2; the paid line) | doc 17 R4 | 4 | 4 | 4 | 3 | 5 | 5 | 4 | 3 | **4.00** | M, I |
| O-16 | **MCP write tools with elicitation** (expose `create_event`, `create_draft`, `remember_fact` to external clients behind the same approval queue) | 1.6, 3.5 | 3 | 4 | 2 | 4 | 4 | 2 | 3 | 3 | **3.15** | F, M |
| O-17 | **User-registered MCP servers** (`mcp-host` runs any stdio or HTTP server the user adds; tools become available to routines through O-04) | 1.6 | 4 | 3 | 3 | 3 | 4 | 3 | 3 | 2 | **3.25** | F, M |
| O-18 | **Routine authoring in plain language** (describe a routine; the model fills the schedule, sources, model preference and output; the user edits a form, not a prompt) | doc 17 A-gaps | 4 | 2 | 4 | 3 | 4 | 2 | 2 | 4 | **3.15** | M |
| O-19 | **Mobile write approvals and push** (FR-MOB-2; doc 17 Release 3.1) | 1.7 | 4 | 2 | 4 | 2 | 4 | 3 | 2 | 4 | **3.10** | F |
| O-20 | **Windows parity** (Away Mode, live alerts, Reply Priorities flag, live transcript display) | 1.7 | 4 | 2 | 4 | 3 | 4 | 2 | 1 | 4 | **3.05** | F |
| O-21 | **Limitless and Rewind import** (their export formats into `items`, with the import pipeline that exists) | 2.1 | 3 | 3 | 2 | 4 | 3 | 2 | 1 | 5 | **2.80** | M |
| O-22 | **Team spaces** (doc 17 R5: shared project graph with per-entity grants, same E2E model) | doc 17 | 4 | 4 | 3 | 2 | 4 | 5 | 4 | 2 | **3.55** | M, I |
| O-23 | **Outlook ingestion and Windows Outlook** (read mail into `items`; Windows connector) | 1.7 | 4 | 1 | 3 | 3 | 3 | 3 | 1 | 4 | **2.75** | F |
| O-24 | **Meeting Ask over the whole graph** (meeting view can ask across related meetings, people and threads, not only the current transcript) | 1.3 | 4 | 3 | 4 | 4 | 3 | 1 | 2 | 5 | **3.30** | F |
| O-25 | **Retrieval re-rank stage** (a small cross-encoder, local, over the top 40 RRF hits) | 1.3 | 3 | 1 | 3 | 3 | 3 | 1 | 2 | 5 | **2.50** | M, H |
| O-26 | (unconventional) **Relationship health** (per-person cadence, reply latency both ways, open promises; a quiet "you have not spoken to X in 6 weeks, you owe them a reply") | 1.8 | 4 | 5 | 4 | 3 | 3 | 2 | 4 | 3 | **3.65** | I, H |
| O-27 | (unconventional) **Contradiction and drift alerts** (the graph notices a fact that changed: a date moved, a price changed, a person's role changed; bi-temporal edges make this a query) | 3.2 | 4 | 5 | 3 | 2 | 3 | 2 | 5 | 3 | **3.45** | M, H |
| O-28 | (unconventional) **Pre-mortem brief** (before a decision meeting, a routine lists what was said last time, what was promised and not done, and the open questions, each cited; the Meeting Prep citation contract extended to the graph) | 3.7 | 4 | 4 | 4 | 4 | 4 | 2 | 3 | 4 | **3.70** | F, I |
| O-29 | (unconventional) **Memory diff on return** ("since you were away: 3 decisions, 2 promises to you, 1 changed date", generated from graph deltas, not from a prompt over everything) | 1.8 | 5 | 4 | 5 | 3 | 4 | 2 | 4 | 4 | **4.00** | I, H |
| O-30 | (unconventional) **Skill sharing as signed routine files** (a routine plus its prompts, sources and model preference as a signed JSON file; a community gallery; FR-AGT-5 and 6 without an app store) | FR-AGT | 3 | 3 | 3 | 3 | 5 | 3 | 4 | 3 | **3.30** | F, M |
| O-31 | (unconventional) **Local-model fine-tune on your own memory** (LoRA on the user's corpus for extraction and style, trained on the machine, never leaves it; "your model, trained on you") | 3.3 | 3 | 5 | 3 | 1 | 4 | 3 | 5 | 2 | **3.25** | S |
| O-32 | (unconventional) **Counterfactual recall** ("what did I know on the day I decided X?": point-in-time queries over bi-temporal edges) | 3.2 | 3 | 5 | 2 | 2 | 3 | 1 | 4 | 4 | **2.95** | M, H |
| O-33 | (unconventional) **Ambient prompt injection firewall** (screen and email content enters prompts as data; a local classifier flags instruction-shaped text before it reaches a tool-using agent; publishes a rate) | 3.4 | 3 | 4 | 2 | 3 | 5 | 1 | 4 | 5 | **3.20** | M |
| O-34 | (unconventional) **Device-local A/B of your own routines** (run two prompts or models on the same routine for a week, the user picks the better, the ledger shows cost and the eval scores; pass^k for one person) | 3.9 | 2 | 4 | 3 | 4 | 4 | 2 | 3 | 5 | **3.20** | I |

Worked example for the top row: O-01 = (5×20 + 5×15 + 4×15 + 4×15 + 5×10 + 3×10 + 5×10 +
4×5) / 100 = (100 + 75 + 60 + 60 + 50 + 30 + 50 + 20) / 100 = 4.45.

## 4.3 Ranked order and horizon

| Rank | Item | Score | Horizon |
|---|---|---|---|
| 1 | O-01 Entity and relationship memory | 4.45 | 0 to 6 months (schema and extraction first) |
| 2 | O-08 Commitments ledger | 4.20 | 3 to 6 |
| 3 | O-05 Condition-triggered routines | 4.10 | 3 to 6 |
| 4 | O-07 Speaker identity across meetings | 4.00 | 6 to 12 |
| 4 | O-15 Cloud routine runner | 4.00 | 6 to 12 (doc 17 R4) |
| 4 | O-29 Memory diff on return | 4.00 | 3 to 6 |
| 7 | O-10 Where-I-left-off per entity | 3.95 | 0 to 3 |
| 8 | O-04 Governed tool-use loop | 3.90 | 3 to 6 |
| 9 | O-02 Surface Connected Topics | 3.85 | 0 to 3 |
| 10 | O-09 Decision log with provenance | 3.75 | 6 to 12 |
| 11 | O-14 Trust surface | 3.70 | 0 to 3 |
| 11 | O-28 Pre-mortem brief | 3.70 | 6 to 12 |
| 13 | O-26 Relationship health | 3.65 | 6 to 12 |
| 14 | O-22 Team spaces | 3.55 | 12 plus (doc 17 R5) |
| 15 | O-06 Graph leg in retrieval | 3.50 | 3 to 6 |
| 15 | O-11 Local-model extraction tier | 3.50 | 3 to 6 |
| 17 | O-27 Contradiction and drift alerts | 3.45 | 12 plus |
| 18 | O-13 Legible proactivity controls | 3.40 | 0 to 3 |
| 19 | O-03 Evaluation harness and spans | 3.30 | 0 to 3 |
| 19 | O-24 Meeting Ask over the graph | 3.30 | 6 to 12 |
| 19 | O-30 Signed routine files | 3.30 | 12 plus |
| 22 | O-17 User-registered MCP servers | 3.25 | 6 to 12 |
| 22 | O-31 Local fine-tune | 3.25 | 12 plus (research) |
| 24 | O-33 Injection firewall | 3.20 | 3 to 6 (with O-04) |
| 24 | O-34 Device-local routine A/B | 3.20 | 12 plus |
| 26 | O-16 MCP write tools | 3.15 | 6 to 12 |
| 26 | O-18 Plain-language routine authoring | 3.15 | 3 to 6 |
| 28 | O-19 Mobile approvals and push | 3.10 | doc 17 R3 |
| 29 | O-20 Windows parity | 3.05 | 0 to 6 (doc 17 R2) |
| 30 | O-32 Counterfactual recall | 2.95 | 12 plus |
| 31 | O-21 Limitless and Rewind import | 2.80 | 0 to 3 (small) |
| 32 | O-23 Outlook ingestion | 2.75 | 6 to 12 |
| 33 | O-12 Embedding upgrade | 2.60 | 3 to 6 (with the eval harness) |
| 34 | O-25 Re-rank stage | 2.50 | after the eval harness says so |

O-03 (eval harness) scores modestly on its own and is nonetheless scheduled first, because
O-06, O-11, O-12 and O-25 cannot be judged without it and the trends evidence in document
3 says graph retrieval sometimes makes small corpora worse.

## 4.4 Notes on individual rows

- **O-01 is the spine.** Eleven other rows (O-06, O-08, O-09, O-10, O-24, O-26, O-27,
  O-28, O-29, O-32, O-22) are queries or views over the graph it creates. Its leverage score
  is 4, not 5, because the existing `entity` and `entity_mention` tables lack typed
  entity-to-entity edges and validity intervals, which is the new work.
- **O-05 is the agent feature users will feel first.** A routine that runs "after any
  meeting with my manager" is the difference between a scheduled report and an assistant,
  and it needs no tool-use loop, only a trigger evaluator in the tick.
- **O-07 scored high on value and differentiation, low on leverage.** The diarizer labels
  speakers per meeting; cross-meeting identity needs speaker embeddings, an enrollment UI,
  and a consent posture (the user confirms every identity; nothing is inferred about
  people who are not users). Document 2 shows every competitor is weak here.
- **O-15 and O-22 are doc 17's.** They are re-scored here only to place them against the
  capability work; their design and pricing are doc 17's and unchanged.
- **O-31 is speculation** and stays in the backlog as a research item because it is the
  one capability a cloud product structurally cannot offer.
- **O-33 moves with O-04.** A tool-using agent fed screen text and email is the textbook
  prompt-injection target (document 3 §3.4); shipping the loop without the firewall is
  the single riskiest thing on this list.

## 4.5 What was considered and left out

- Vertical agents (sales, recruiting) as Otter and Fireflies offer: against the product
  vision and the single-user model.
- Computer-use automation (FR-AGT-1): OSWorld 2.0 completion rates around 20% (document 3)
  and the Copilot Actions posture (separate session, off by default) say the reliability
  and sandboxing are not there for an app that holds a user's screen memory. Revisit in
  12 months.
- Managed inference of any kind: decision 4 in doc 16.
- A Whimbrel bot that joins meetings: bot-free capture is a stated differentiator and the
  Vexa model (per bot-hour) is a different business.
- A hosted MCP server: contradicts E2E sync; the Cloud runner (O-15) is the governed route
  to "reach Whimbrel from elsewhere."
