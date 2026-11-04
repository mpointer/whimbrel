# 0. Executive assessment (2026-11-03)

This folder is the strategic roadmap requested on 2026-11-03: an evidence-driven assessment
of Whimbrel against the current market and technology landscape, with a scored backlog, a
horizon roadmap, technical proposals, an agentic strategy and a 90-day plan. It is grounded
in the tree at 0.9.51 (document 1), in checked competitor and technology facts with dates
and sources (documents 2 and 3), and in the strategy already decided in
`docs/16-open-core-commercial-model.md` and `docs/17-competitive-gaps-roadmap-monetization.md`,
which it does not reopen.

Evidence discipline throughout: **verified fact** (in the tree), **market evidence**
(sourced and dated), **inference**, **hypothesis** (testable), **recommendation**, and
**speculation**, marked as such.

## Where the product stands

Whimbrel is a working local-first ambient memory with bot-free meeting capture, on-device
transcription and diarization, screen memory, a hybrid retrieval pipeline with citations,
seven scheduled routines, two approval-gated write actions, end-to-end encrypted sync
across Mac, Windows and iOS, own-model routing with a cheap-first on-ramp, a local MCP
server, and an Apache licence. About 87,000 lines of Rust in 14 crates, with 1,895 core
tests, plus three shells, a relay and a website.

Three findings from the inventory change the picture painted by the existing strategy
docs:

1. **Connected Topics is built and invisible.** About 5,000 lines of entity extraction,
   clustering, naming and export, plus their API layer, have no FFI entry, no Tauri
   command, no UI, and their job types are excluded from the queue. Doc 17 lists it as
   shipped. It is the largest piece of leverage in the repository.
2. **There is no agent.** "Agent" in the tree is one gated draft parked in a routine run.
   No tool-use loop, no planning, no persistent agent state, no audit log, no kill switch.
   The governed call path, the approval state machine and the MCP host are all there to
   build one on.
3. **There is no knowledge graph.** Entities and mentions exist; typed relations, validity
   intervals, decisions and commitments as structure do not. Nothing in the end-user market
   has them either (document 2 §2.6).

## Where the market stands

Granola ($1.5B), Littlebird ($11M seed, the closest feature match, cloud memory, resold
inference), anarlog (MIT, meetings only), Screenpipe (relicensed away from open source),
and the suites (Notion, Microsoft, Google, Zoom) all converge on bot-free capture,
summaries, chat and scheduled routines. Credits are the new meter and the new complaint.
Limitless and Rewind are gone; Dot and Khoj Cloud are gone. Nobody combines local-first
ambient memory, proactive routines, E2E multi-device sync, own keys, entity memory and
gated local actions, and the architectures of the funded players prevent them from doing
so quickly (document 7 §7.5).

The technology evidence (document 3) says: graph retrieval helps multi-hop questions and
can hurt small corpora, so an eval set must precede it; the bi-temporal edge is the one
idea to take from agent-memory systems; local 4B models do extraction only with a closed
schema and constrained decoding; approval prompts get rubber-stamped, so gate by
reversibility; proactive briefs die without legibility; and the open-core pricing pattern
(free local core, $4 to $15 for sync and compute) is exactly doc 16's and doc 17's.

## The strategy in one paragraph

Turn the memory into knowledge, then let routines act on it. The spine of the next year is
a small bi-temporal personal graph (people, projects, commitments, decisions, each fact
with its source) built from the dormant topics stack and a local-first extraction tier;
every feature users will feel (entity pages, "since you were away", a commitments ledger,
a decision log, pre-meeting briefs) is a query over it. Around the graph: condition-
triggered routines, a governed tool loop with reversibility tiers and a user-held audit
log, an injection firewall, speaker identity across meetings, and a legible trust and
proactivity surface. Doc 17's releases (Windows parity, mobile approvals, Cloud runner,
Team) interleave unchanged; the Cloud runner becomes worth paying for because the routines
it runs are event-driven and agentic.

## The seven direct answers

**Which three features to build first.**
1. Surface Connected Topics and give it structure (F1 then F2): the entity schema with
   bi-temporal relations, commitments and decisions, built first from heuristics that need
   no model.
2. The evaluation harness with local spans (F3): without it, no retrieval, prompt or model
   change can be judged, and the trends evidence says graph retrieval can make small
   corpora worse.
3. The trust surface and legible proactivity (F4 and F5): the privacy architecture is
   real and invisible, and every proactive feature later depends on users not feeling
   watched.

**The single most differentiating feature.** A user-owned graph of people, promises and
decisions with sources, built locally and synced end to end (F2 with F6 and F9). No
notetaker has it, the infrastructure vendors have no capture, and the cloud products cannot
move their memory without changing their business.

**The feature most likely to increase retention.** "Since you were away" plus the
commitments ledger (F9): a return brief generated from graph deltas with a source on every
line, and one nudge per promise. It makes the memory pay out on the exact moment a user
decides whether the capture is worth keeping on.

**The architectural investment that unlocks the most.** The relation schema with validity
intervals and per-source extraction (document 6 §6.1). Eleven backlog items are queries
over it, the Team product becomes a shared subgraph, and the agent loop has structure to
act on instead of rows to guess from. Second: the tool layer with reversibility tiers
(§6.2), which the Cloud runner's per-routine grants also need.

**The most ambitious realistic agentic capability.** A condition-triggered, bounded
plan-execute-verify routine over the user's own memory and connectors, with reversibility
tiers, a kill switch, an audit log, an injection firewall and a measured pass^4 above 0.7,
running on the primary device in H2 and on the Cloud runner in H3. Not autonomous desktop
control: the evidence (OSWorld 2.0 around 20%, Copilot Actions off by default) says that
is a year or more away for an app that holds a user's screen memory.

**What to avoid.** Full community-summary GraphRAG at personal scale; a graph retrieval
leg before the eval set exists; managed inference of any kind (decision 4); vertical
agents; meeting bots; a hosted memory server; computer use; multi-agent orchestration;
proactive output without a visible reason; blanket approval prompts; lifetime licences;
and any relicensing away from Apache.

**What to build in the first 90 days.** The eval harness and spans (weeks 1 to 2), the
trust surface (1 to 3), Connected Topics surfaced on both desktops with export and sync (3
to 6), legible proactivity controls (4 to 5), the relation schema with heuristic extraction
and the entity read model (6 to 10), entity pages (9 to 12), and the written specs for
condition triggers and the tool loop (11 to 12), so H2 starts on day 91 with no design
debt. Details in document 8 §8.1.

## Corrections to existing docs that this folder makes

- Doc 17 §1: Connected Topics is not shipped; it is complete in core and unreachable.
- Doc 17: "no telemetry" is nearly true; the update feed records a weekly random
  per-install id with platform and version. The roadmap keeps it there and makes it
  visible.
- `docs/02-functional-requirements.md`: FR-AGT-1 to 7, FR-RTN-1 (conditions), FR-MEM-5
  (entity views), FR-MOB-2 and 3 remain unmet; this roadmap schedules all but FR-AGT-1.

## Contents of this folder

| Doc | What it is |
|---|---|
| 01 | Architecture and feature inventory, verified against the tree, with a leverage map |
| 02 | Competitive intelligence, 25 products, sourced and dated, with a matrix and gaps |
| 03 | Technology trends, ten areas, sourced, each with its implication |
| 04 | Scored opportunity backlog, 34 items, 10 unconventional, with the scoring model |
| 05 | Roadmap by horizon, 17 feature records with 18 fields each, dependency graph |
| 06 | Technical proposals for seven features, at spec-ready depth |
| 07 | Agentic AI strategy and differentiation, moats, positioning, risks |
| 08 | First 90 days, engagement design, measurement plan, product vision |
