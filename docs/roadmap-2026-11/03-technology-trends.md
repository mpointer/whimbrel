# 3. Technology trends that bear on the roadmap (checked 2026-11-03)

Tags: **shipped** (available to users), **announced**, **paper**, **OSS**. Dates are the
ones visible on the cited page. Each section ends with what it means for Whimbrel, which
is inference, not evidence.

## 3.1 Graph RAG and graph-enhanced retrieval

- **Microsoft GraphRAG** (OSS): 2.x through 3.3.0; DRIFT search since 0.4.0
  ([changelog](https://github.com/microsoft/graphrag/blob/main/CHANGELOG.md)). **LazyGraphRAG**
  (announced 2024-11-25) claims indexing cost equal to vector RAG and global-query quality
  comparable to full GraphRAG at a small fraction of the query cost; it shipped only inside
  Microsoft products as a preview
  ([Microsoft Research](https://www.microsoft.com/en-us/research/blog/lazygraphrag-setting-a-new-standard-for-quality-and-cost/)).
- **LightRAG** (paper, arXiv 2410.05779): dual-level retrieval over an entity graph. Its
  own guidance recommends 32B-plus models for extraction; the default output format fails
  on most local models unless JSON mode is used
  ([tutorial](https://dev.to/therabbithole/lightrag-tutorial-a-practical-guide-to-knowledge-graph-based-rag-4oa0)).
- **HippoRAG 2** (paper, ICML 2025, arXiv 2502.14802): personalized PageRank over a
  passage-plus-entity graph; about 7% average gain over standard RAG on multi-hop tasks
  with no loss on factual tasks. On the third-party WildGraphBench, plain vector retrieval
  beat two of the three graph systems
  ([benchmarklist](https://benchmarklist.com/benchmarks/wildgraphbench/)).
- **Small corpora**: a 2025 textbook study found vanilla RAG matching or beating GraphRAG
  local search on smaller texts, with graph construction adding distractors
  ([arXiv 2509.07846](https://arxiv.org/pdf/2509.07846)); a scaling study found plain RAG ahead
  at its smallest corpus and GraphRAG only pulling ahead as the corpus grew
  ([arXiv 2506.05690](https://arxiv.org/pdf/2506.05690)). No evaluation on personal data
  exists.
- Others: nano-graphrag (toy), Cognee (Rust SDK for on-device memory, $7.5M seed
  2026-02-19), KAG (schema-constrained, claims an 89% build-token cut, no benchmarks).

**Meaning for Whimbrel.** Full community-summary GraphRAG is not justified at personal
scale by any published evidence. The HippoRAG 2 pattern (entity-passage links, PageRank, no
community summaries) is the cheapest graph upgrade with published multi-hop gains, and
Whimbrel already has the entity-passage bipartite index (`entity_mention`). Build a
personal-data eval set before touching retrieval.

## 3.2 Agent memory systems

- **Mem0** (shipped, Apache-2.0 plus platform): the April 2025 paper claims +26% over
  OpenAI memory on LoCoMo; Zep's rebuttal shows a full-context baseline beating Mem0's best
  graph config because LoCoMo conversations fit in a context window
  ([Zep blog](https://blog.getzep.com/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/)).
  Mem0's 2026 algorithm is single-pass, add-only extraction with temporal decay, platform
  only. OpenMemory MCP is a local memory server for any MCP client.
- **Zep and Graphiti** (shipped, Apache-2.0, 32.5k stars): the bi-temporal model. Every
  edge carries `valid_at` (true in the world) and `invalid_at`, plus an ingestion time;
  contradictions invalidate the old edge instead of deleting it
  ([GitHub](https://github.com/getzep/graphiti)). Reported LongMemEval 71.2% against 60.2%
  full-context. The project warns that smaller models produce incorrect schemas.
- **Letta** (Context Repositories, git-backed memory, Feb 2026), **LangMem**, **A-MEM**
  (NeurIPS 2025, Zettelkasten-style evolving notes).
- **Evals**: LongMemEval (ICLR 2025) is the credible one; commercial assistants drop 30%
  against an oracle. LoCoMo is weak. Vendor numbers above 90% are self-reported and
  unreproduced.

**Meaning for Whimbrel.** Steal the bi-temporal edge: every extracted fact gets
`valid_from`, `valid_to` and `recorded_at`. Every memory product above assumes a frontier
model for extraction; local extraction is the open gap and the one that fits Whimbrel's
posture.

## 3.3 Entity resolution and relation extraction with small local models

- Project guidance is consistent: Graphiti says use the most capable model you can run;
  LightRAG says 32B-plus. No measured head-to-head of 4B to 12B models exists.
- **GLiNER2** (paper and OSS, July 2025): a 205M-parameter encoder doing NER,
  classification and schema-driven relation extraction; GLiNER2.5 Small has an on-device
  LiteRT build ([arXiv 2507.18546](https://arxiv.org/abs/2507.18546)). This is the realistic
  local extraction tier.
- **LightKGG** (paper, Oct 2025): small models are good at entity identification and weak
  at relation recognition; resolve ambiguity with graph topology, not another model pass
  ([arXiv 2510.23341](https://arxiv.org/pdf/2510.23341v1)).
- Grammar-constrained decoding (Ollama 0.5-plus compiles JSON schema to GBNF) makes
  syntax reliable even at 1B; it says nothing about value correctness, and costs 30 to 80%
  in speed by one unsourced estimate.

**Meaning for Whimbrel.** Split the pipeline: a GLiNER-class encoder (or the existing
header and attendee heuristics) for entities and candidate relations; a 4B to 9B model with
grammar-constrained JSON only for relation typing; a frontier model through the user's own
key as an optional upgrade. Resolve entities with blocking plus embeddings plus the graph
(attendee lists, email addresses, co-occurrence). Store extraction per source so
re-extraction is local to the changed source.

## 3.4 Agentic AI for personal productivity

- **Computer-use agents** (shipped): OpenAI Operator (OSWorld 38.1%, Jan 2025), ChatGPT
  agent (July 2025), Claude for Chrome (prompt-injection success 23.6% unmitigated, 11.2%
  mitigated, Aug 2025), Gemini Agent (Nov 2025), Copilot Actions on Windows (agents in a
  separate session under their own accounts, off by default, Oct 2025), Perplexity Comet
  (hit by injection exploits). OSWorld 2.0 (June 2026) shows top models completing about 20%
  of tasks under strict scoring ([arXiv 2606.29537](https://arxiv.org/html/2606.29537v1)).
- **Scheduled and background agents** are table stakes: ChatGPT Tasks (10 active), Gemini
  Scheduled Actions (10), Claude Code routines (April 2026), Claude Cowork scheduled tasks
  with a per-task approval mode. **ChatGPT Pulse** (proactive daily briefs) launched
  2025-09-25 on Pro and was retired 2026-06-17 into a scheduled-tasks hub
  ([9to5Mac](https://9to5mac.com/2026/06/17/openai-launches-scheduled-tasks-in-chatgpt-details-here/)).
- **Reliability**: tau-bench pass^k falls fast (pass^1 0.46 to pass^4 0.225 for one model);
  MAST (NeurIPS 2025) sorts 1,642 failing traces into specification, inter-agent
  misalignment and task verification ([arXiv 2503.13657](https://arxiv.org/abs/2503.13657v2)).
- **Approval fatigue**: users reportedly approve 93% of Claude Code permission prompts;
  auto mode became the default in August 2026. The 2026 Singapore Consensus recommends
  calibrating oversight to risk, reversibility and sensitivity; "Loopjacking" shows HITL
  approvals themselves can be hijacked ([arXiv 2609.21081](https://arxiv.org/pdf/2609.21081)).

**Meaning for Whimbrel.** Approval-gated actions are right; blanket prompts will be
rubber-stamped. Gate by reversibility, auto-run read-only steps, show a diff-style
preview, keep a post-hoc audit log. Use plan, execute, verify with an explicit verification
step. Measure pass^k on a fixed routine suite, not single-run success. Vendors retreating
from inferred proactive briefs is a signal to keep proactive output user-configured.

## 3.5 MCP and interoperability

- Spec 2025-06-18: elicitation, structured tool output, OAuth resource-server model.
  2025-11-25: Tasks (call now, fetch later), URL-mode elicitation, Client ID Metadata
  Documents. A 2026-07-28 revision moved Tasks to an extension
  ([WorkOS](https://workos.com/blog/mcp-2025-11-25-spec-update)).
- Governance: Agentic AI Foundation under the Linux Foundation (2025-12-09); MCP, goose and
  AGENTS.md donated; 10,000-plus public servers claimed. Registry counts in October 2026
  run 43k to 52k; the `mcp` PyPI package saw 71.9M downloads in one week.
- A2A: 150-plus supporting organizations (2026-04-09); positioned as agent-to-agent, MCP
  as agent-to-tool.
- Local memory servers (OpenMemory MCP, Graphiti's) ship without authentication.

**Meaning for Whimbrel.** Target 2025-06-18 as the floor for the local server and adopt
elicitation for "ask before a write." Scoped tokens plus a call log is already ahead of
the ecosystem norm; make that visible. A2A is irrelevant for a single-user app this year.

## 3.6 Local models on a laptop

- Releases: Qwen3.5 small series (0.8B to 9B, Apache-2.0, 262K context, native vision,
  2026-03-05); Gemma 4 (2026-04-02, Apache-2.0, E2B, E4B, 26B MoE, 31B); Phi-4-mini.
- Speed: Qwen3-8B 4-bit about 90 tokens a second on M4 Max; Llama 3.1 8B Q4 about 25 to 45
  on M4 class; Gemma 4 E4B about 30 on a 16GB M1 Pro. Windows NPUs cap around 4B and are
  bound by memory bandwidth; they buy battery, not speed.
- Reliability: tool calling at 4B is uneven across benchmarks; 4B reasoning models can burn
  the whole budget without answering. Grammar-constrained JSON fixes schema validity.
- Embeddings: EmbeddingGemma 308M (top sub-500M multilingual on MTEB at release,
  2025-09-04); Qwen3-Embedding-0.6B (MTEB English v2 70.70, 32K context, Matryoshka).

**Meaning for Whimbrel.** A 4B class model (Qwen3.5-4B or Gemma 4 E4B) is the always-on
local tier for classification, extraction and short summaries, non-reasoning mode, grammar
constrained; 8B to 9B for 16GB-plus Macs; Windows NPUs are a battery feature. The current
embedding model (all-MiniLM-L6-v2, 384 dims) is two generations behind; EmbeddingGemma or
Qwen3-Embedding-0.6B with Matryoshka truncation is the upgrade, gated on `EMBEDDING_MODEL_VERSION`
rebuild cost.

## 3.7 Decision intelligence for individuals

Gartner's first Magic Quadrant for Decision Intelligence Platforms (2026-01-26) is
enterprise. The consumer layer is indie: Fatebook (Brier scores, calibration charts, a
community MCP server), Reckon (iOS decision journal, 2026-08-18), Executive OS Decision
Journal, Calibrate (OSS). Calibration training has peer-reviewed evidence; a decision
journal maker's own review warns that the "19% better forecasting" journaling claim did not
hold under controlled designs
([LessWrong](https://www.lesswrong.com/posts/sfNvnRhnowKz3itCi/grading-your-own-predictions-the-parts-i-couldn-t-solve)).

**Meaning for Whimbrel.** Nobody owns decision provenance for individuals, and Whimbrel is
the only product with the raw material (meetings, email, calendar) to populate a decision
log without typing. Keep claims modest: provenance and recall, not better judgment.

## 3.8 Proactive assistants and the privacy backlash

- No vendor has published acceptance or retention numbers for proactive briefs; the only
  market datapoint is Pulse's retirement.
- Research: two preregistered studies (N=761, N=571) found anticipatory help increased
  perceived threat and reduced adoption, with offering faring better than acting
  ([arXiv 2509.09309](https://arxiv.org/pdf/2509.09309v1)); a CHI 2026 study (N=112) found
  explanations at initiation, incremental disclosure and adjustable proactivity intensity
  drive acceptance.
- Microsoft Recall: pulled June 2024, relaunched April 2025 opt-in in a VBS enclave with
  Hello gating, exclusions and removal; Signal, Brave and AdGuard block capture through DRM
  flags because there is no opt-out API; an April 2026 exploit bypassed the Hello gate.

**Meaning for Whimbrel.** Proactivity opt-in per source, each surfaced item explaining why
it surfaced, default to offer rather than act, adjustable intensity. Recall's lesson:
local storage is not the trust story; user-held keys, per-app exclusions, a visible capture
indicator and an easy export and delete path are. Whimbrel has most of these and shows few
of them.

## 3.9 Evaluation and observability for agents

OpenTelemetry GenAI semantic conventions are still "Development" but the agent span shape
(`create_agent`, `invoke_agent`, `execute_tool`) and the evaluation event are settled
([GitHub](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-agent-spans.md)).
Tooling: Langfuse (MIT), Phoenix, Opik, Braintrust. Practitioner consensus on what to
measure: strict task success, tool-call failures split into model versus external, cost
per successful task including retries, p95 latency, pass^k.

**Meaning for Whimbrel.** The usage ledger already records provider, model, route, tokens,
cents and a trace id per call. Extending it to spans with a parent id covers agents at
near-zero cost, local by default. Ship a fixed eval set: 50 to 100 questions over a seeded
corpus, a LongMemEval-style multi-session set, and a pass^k routine suite, run on every
prompt or retrieval change.

## 3.10 Pricing of local-first and open-core products

Obsidian (core free for all uses since 2025-02-20; Sync $4 to $5; Publish $8 to $10),
Granola (Basic free, Business $14, Enterprise $35; $125M at $1.5B), Screenpipe (relicensed
to source-available, lifetime licences withdrawn, $25 to $200 a month), anarlog (free local
core; Pro $14 for cloud transcription, sync and sharing; "local speech recognition does not
make summaries local").

**Meaning for Whimbrel.** The pattern is uniform: the local core is free and people pay $4
to $15 a month for sync, sharing, cloud compute and team controls. This matches doc 17's
pricing and the five decisions in doc 16 and needs no revision. Avoid lifetime licences;
keep the licence clean, because Screenpipe's relicensing cost it the open-source claim in
reviews.
